#!/usr/bin/env python3
"""Export a complete reference build kit, restore it, rebuild offline and replay.

This explicitly selected development-fixture lane uses public test keys. Its
trusted CLI and Cargo harness are operator inputs, never archive executables.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys

import myelin_reproducible_build as build


def accept_games(args, out, manifest, outputs, packed, original_cli, policy, limits):
    """Original example binaries produce/export their histories in a fresh container.

    Export anchors originate in those trusted source processes, not downloaded
    manifests. The existing sandbox runner then checks the complete archives.
    """
    games = ("inventory", "turn_based", "fixed_tick")
    destination = out / "games"
    destination.mkdir()
    container_build = json.loads(json.dumps(packed))
    for source in container_build["artifacts"]:
        source["path"] = "/work/packed/" + source["id"]
    inventory = build.check_outputs(outputs)
    runtime = [{"path": "/" + name.removeprefix("runtime/"), "source": "/work/outputs/" + name}
               for name in inventory["files"] if name.startswith("runtime/")]
    build.write_json(destination / "artifacts.json", {"build": container_build, "runtime": runtime, "limits": limits,
                     "work_budget": {"max_steps": 100_000, "max_io_bytes": 16 << 30, "max_elapsed_ms": 30_000}})
    script = ["#!/bin/sh", "set -eu", "mkdir /work/results"]
    for game in games:
        folder = "/work/results/" + game
        script.extend([f"mkdir {folder}",
                       f"/work/outputs/{game} {folder}/source demo > {folder}/demo.json",
                       f"/work/outputs/{game} {folder}/source export /work/artifacts.json {folder}/bundle > {folder}/export.json",
                       # Only these newly created fixture databases are removed.
                       f"rm -rf {folder}/source"])
    (destination / "run-games.sh").write_text("\n".join(script) + "\n")
    podman = [args.podman, "--root", str(out / "reproduction/image-store"), "--runroot", str(out / "reproduction/run-store"), "--storage-driver=vfs"]
    build.command([*podman, "create", "--pull=never", "--network=none", "--http-proxy=false", "--cap-drop=ALL",
                   "--security-opt=no-new-privileges", "--user=65534:65534", "--memory=2g", "--cpus=2", "--pids-limit=128",
                   "--timeout=1200", "--workdir=/work", "--entrypoint=/bin/sh", manifest["image_id"], "/work/run-games.sh"],
                  destination, "create")
    cid = (destination / "create.stdout").read_text().strip()
    if not re.fullmatch("[0-9a-f]{64}", cid):
        raise ValueError("game preparation did not return an exact container identity")
    try:
        for source, target, label in [(outputs, "/work/outputs", "outputs"), (out / "packed", "/work/packed", "build-package"),
                                      (destination / "artifacts.json", "/work/artifacts.json", "artifacts"),
                                      (destination / "run-games.sh", "/work/run-games.sh", "driver")]:
            build.command([*podman, "cp", str(source), f"{cid}:{target}"], destination, "copy-" + label, 600)
        build.command([*podman, "start", "--attach", cid], destination, "generate-export", 1200)
        build.command([*podman, "cp", "--archive=false", f"{cid}:/work/results", str(destination / "results")], destination, "collect", 600)
    finally:
        build.command([*podman, "rm", "--force", cid], destination, "destroy-source-container")
    reports = {}
    for game in games:
        folder = destination / "results" / game
        if (folder / "source").exists():
            raise ValueError("source database was not removed")
        demo = json.loads((folder / "demo.json").read_text())
        exported = json.loads((folder / "export.json").read_text())
        anchor = exported["source_anchor"]
        if anchor["genesis_commitment"] != demo["genesis"] or exported["state_root"] != demo["state_root"]:
            raise ValueError("export source identity differs from committed game history")
        bundle = json.loads((folder / "bundle/manifest.json").read_text())
        if len(bundle["sessions"]) != 1:
            raise ValueError("example must export exactly its own session")
        execution = bundle["sessions"][0]["execution"]
        if execution["program"] != bytes(demo["program_digest"]).hex() or execution["program"] != execution["replayer"]:
            raise ValueError("game archive did not retain its original executable")
        if execution["build_inputs"] != packed["root"] or execution["toolchain"] != packed["root"]:
            raise ValueError("game archive lost its complete recognized build kit")
        if build.sha(folder / "bundle/blobs" / execution["program"]) != inventory["files"][game]["sha256"]:
            raise ValueError("archived game differs from the independently reproduced output")
        for library in execution["runtime"]["files"]:
            if build.sha(folder / "bundle/blobs" / library["blob"]) != inventory["files"]["runtime" + library["path"]]["sha256"]:
                raise ValueError("archived game library differs from the reproduced runtime")
        build.write_json(folder / "trust.json", [anchor])
        build.write_json(folder / "policy.json", policy)
        build.write_json(folder / "limits.json", limits)
        session = bytes(anchor["session_id"]).hex()
        build.command([*original_cli, "replay", str(folder / "bundle"), str(folder / "trust.json"), session,
                       str(folder / "policy.json"), str(folder / "limits.json")], folder, "isolated-replay", 600)
        replay = json.loads((folder / "isolated-replay.stdout").read_text())
        if (replay["host_replay"] != "verified" or replay["result"]["state_root_after"] != demo["state_root"]
                or replay["result"]["receipt"] != demo["replay"]):
            raise ValueError("isolated archive replay differs from original game results/receipt")
        reports[game] = {"source_anchor": anchor, "program_sha256": inventory["files"][game]["sha256"],
                         "replay": replay["result"], "source_database_removed": True}
    build.write_json(destination / "acceptance.json", {"status": "verified", "games": reports,
                     "scope": "original reproduced programs and full build kit; local fresh container and Linux sandbox; public development keys"})
    return reports


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--kit", type=Path, required=True)
    parser.add_argument("--expected-manifest", required=True)
    parser.add_argument("--outputs", type=Path, required=True, help="independently reproduced reference binary/runtime outputs")
    parser.add_argument("--trusted-cli", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--podman", default="podman")
    parser.add_argument("--timeout", type=int, default=3600)
    args = parser.parse_args()
    kit, outputs, cli, out = (p.resolve() for p in (args.kit, args.outputs, args.trusted_cli, args.out))
    manifest = build.verify_inputs(kit, args.expected_manifest)
    expected = build.check_outputs(outputs)
    if args.timeout < 1 or args.timeout > 14400:
        raise ValueError("timeout must be between 1 and 14400 seconds")
    for supplied in (kit, outputs, cli):
        if out == supplied or out.is_relative_to(supplied) or supplied.is_relative_to(out):
            raise ValueError("acceptance output must be disjoint from all supplied artifacts")
    out.mkdir(parents=True, exist_ok=False)
    root = Path(__file__).resolve().parent.parent
    plan = {"files": [{"name": name, "path": str(kit / name)} for name in ["manifest.json", *manifest["artifacts"]]],
            "chunk_bytes": 16 * 1024 * 1024}
    build.write_json(out / "pack-plan.json", plan)
    build.command([str(cli), "pack-files", str(out / "pack-plan.json"), str(out / "packed")], out, "pack", 600)
    packed = json.loads((out / "pack.stdout").read_text())
    env = os.environ.copy()
    env.update(MYELIN_BUNDLE_PACKED_KIT=str(out / "pack.stdout"), MYELIN_BUNDLE_REPRODUCED_OUTPUTS=str(outputs),
               MYELIN_BUNDLE_BUILD_EVIDENCE_OUT=str(out / "evidence"), TMPDIR=str(out / "tmp"))
    (out / "tmp").mkdir()
    test = ["cargo", "test", "--locked", "-p", "myelin-session-bundle", "--features", "test-fixture", "--test", "archives",
            "--", "--exact", "sandbox::typed_build_packages_survive_export_reconstruction_and_isolated_historical_replay", "--nocapture"]
    # The test subprocess inherits an explicitly selected shared target cache,
    # if any. Rebuilding from the restored kit never uses that cache.
    with (out / "export.stdout").open("w") as stdout, (out / "export.stderr").open("w") as stderr:
        result = subprocess.run(test, cwd=root, env=env, stdout=stdout, stderr=stderr, timeout=args.timeout)
    build.write_json(out / "export.json", {"command": test, "exit_code": result.returncode,
                                           "environment": {k: env[k] for k in env if k.startswith("MYELIN_BUNDLE_")}})
    result.check_returncode()
    if "1 passed; 0 failed" not in (out / "export.stdout").read_text():
        raise ValueError("complete-kit fixture did not execute exactly one successful test")
    evidence = out / "evidence"
    build.verify_inputs(evidence / "restored-kit", args.expected_manifest)
    build.command([str(cli), "assemble-files", str(evidence / "bundle"), str(evidence / "trust.json"), packed["root"],
                   str(out / "cli-restored-kit"), str(evidence / "limits.json")], out, "assemble-cli", 600)
    build.verify_inputs(out / "cli-restored-kit", args.expected_manifest)
    print(json.dumps({"stage": "restored-complete-kit", "manifest_sha256": args.expected_manifest,
                      "package": packed["root"], "next": "two offline clean-container builds"}), flush=True)
    build.command([sys.executable, str(root / "scripts/myelin_reproducible_build.py"), "reproduce",
                   "--kit", str(out / "cli-restored-kit"), "--expected-manifest", args.expected_manifest,
                   "--out", str(out / "reproduction"), "--podman", args.podman, "--timeout", str(args.timeout)],
                  out, "rebuild", 2 * args.timeout + 1200)
    for number in (1, 2):
        if build.check_outputs(out / "reproduction" / f"build-{number}" / "result") != expected:
            raise ValueError("restored kit did not reproduce its original binaries/runtime")
    bundle = json.loads((evidence / "bundle/manifest.json").read_text())
    for session in bundle["sessions"]:
        execution = session["execution"]
        if execution["build_inputs"] != packed["root"] or execution["toolchain"] != packed["root"]:
            raise ValueError("session does not bind the complete reconstructed kit")
        program = evidence / "bundle/blobs" / execution["replayer"]
        if build.sha(program) != expected["files"]["myelin-reference-transfer-replayer"]["sha256"]:
            raise ValueError("archive did not pin the reproduced replayer")
        for file in execution["runtime"]["files"]:
            key = "runtime" + file["path"]
            if build.sha(evidence / "bundle/blobs" / file["blob"]) != expected["files"][key]["sha256"]:
                raise ValueError("archive runtime differs from reproduced outputs")
    original_cli = out / "reproduction/build-1/result"
    libraries = sorted({str((original_cli / name).parent) for name in expected["files"] if name.startswith("runtime/")})
    loaders = [original_cli / name for name in expected["files"] if name.startswith("runtime/") and Path(name).name.startswith("ld-linux")]
    if len(loaders) != 1:
        raise ValueError("reference build must contain one exact dynamic loader")
    # This executable was just rebuilt from the recognized source kit and
    # compared byte-for-byte with the separately supplied original inventory.
    command = [str(loaders[0]), "--library-path", ":".join(libraries), str(original_cli / "myelin-session-bundle")]
    build.command([*command, "verify", str(evidence / "bundle"), str(evidence / "trust.json"), str(evidence / "limits.json")], out, "original-verify", 600)
    session = bytes(bundle["roots"][0]).hex()
    build.command([*command, "replay", str(evidence / "bundle"), str(evidence / "trust.json"), session,
                   str(evidence / "policy.json"), str(evidence / "limits.json")], out, "original-replay", 600)
    replay = json.loads((out / "original-replay.stdout").read_text())
    expected_replay = json.loads((evidence / "replay.json").read_text())
    if replay["result"] != expected_replay["result"]:
        raise ValueError("original historical verifier changed the session result")
    games = accept_games(args, out, manifest, original_cli, packed, command,
                         json.loads((evidence / "policy.json").read_text()), json.loads((evidence / "limits.json").read_text()))
    report = {"schema": "myelin-bundle-build-acceptance-v1", "status": "verified", "kit_manifest_sha256": args.expected_manifest,
              "package_root": packed["root"], "outputs": expected, "replay_result": replay["result"], "games": games,
              "claims": ["complete source/vendor/toolchain bytes restored from session bundle", "two offline builds equal original binary/runtime inventory",
                         "original historical verifier audits and replays exact source/target history",
                         "all three original game programs export and replay after deleting source databases"],
              "not_claimed": ["independent physical-machine acceptance", "official application recognition", "CKB-VM, DA, L1 or court adjudication",
                              "rebuilding the toolchain OCI image", "reproducibility of later source changes"]}
    build.write_json(out / "acceptance.json", report)
    print(json.dumps({"status": "verified", "report": str(out / "acceptance.json")}), flush=True)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError, RuntimeError, subprocess.SubprocessError) as error:
        print(json.dumps({"status": "failed", "message": str(error)}), file=sys.stderr)
        sys.exit(2)
