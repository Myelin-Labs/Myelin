#!/usr/bin/env python3
"""Archive and independently rebuild the reference native replayer.

This is an operator-approved build-kit workflow, not an untrusted tar service.
Rebuild requires the kit manifest digest from an independent trusted channel.
No source, credentials, Cargo cache or target directory is mounted into builds.
"""

import argparse
import datetime
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import shutil
import signal
import stat
import subprocess
import sys
import tarfile
import time
from validation import source_identity


SCHEMA = "myelin-build-kit-v1"
REPORT = "myelin-build-reproduction-v1"
ARCHIVES = {"source.tar": 128 << 20, "vendor.tar": 2 << 30, "toolchain.oci.tar": 16 << 30, "build.sh": 64 << 10}
BINARIES = ("myelin-reference-transfer-replayer", "myelin-session-bundle", "inventory", "turn_based", "fixed_tick")


class UnknownBuildKit(ValueError):
    pass


class BuildIntegrityError(ValueError):
    pass


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def regular(path):
    if not stat.S_ISREG(path.lstat().st_mode):
        raise ValueError(f"expected regular file: {path}")


def relative(name):
    path = PurePosixPath(name)
    if not name or len(name) > 4096 or "\0" in name or path.is_absolute() or any(p in (".", "..") for p in path.parts) or str(path) != name:
        raise ValueError(f"invalid relative package path: {name!r}")
    return path


def command(argv, out, label, timeout=300):
    started = time.monotonic()
    result = {"command": argv, "started_at": datetime.datetime.now(datetime.timezone.utc).isoformat()}
    write_json(out / f"{label}.json", result)
    try:
        with (out / f"{label}.stdout").open("wb") as stdout, (out / f"{label}.stderr").open("wb") as stderr:
            process = subprocess.run(argv, stdout=stdout, stderr=stderr, timeout=timeout, check=False)
        result["exit_code"] = process.returncode
        if process.returncode:
            raise RuntimeError(f"{label} exited {process.returncode}; see {out / (label + '.stderr')}")
    except BaseException as error:
        result["error"] = str(error)
        raise
    finally:
        result["wall_seconds"] = time.monotonic() - started
        write_json(out / f"{label}.json", result)
    return result


def archive(root, names, destination):
    root = root.resolve(strict=True)
    records = {}
    with tarfile.open(destination, "w", format=tarfile.PAX_FORMAT) as output:
        for name in sorted(names):
            relative(name)
            path = root / name
            regular(path)
            if not path.resolve().is_relative_to(root):
                raise ValueError(f"source escapes its package: {path}")
            info = tarfile.TarInfo(name)
            info.size = path.stat().st_size
            info.mode = 0o755 if path.stat().st_mode & 0o111 else 0o644
            info.mtime = 315532800
            records[name] = {"bytes": info.size, "mode": info.mode, "sha256": sha(path)}
            with path.open("rb") as source:
                output.addfile(info, source)
    check_archive(destination, records)
    return records


def check_archive(path, records):
    """Compare each member without extracting anything on the host."""
    seen = set()
    with tarfile.open(path, "r:") as package:
        for member in package:
            relative(member.name)
            if not member.isfile() or member.name in seen or member.name not in records:
                raise ValueError(f"unexpected, duplicate or nonregular archive member: {member.name}")
            record = records[member.name]
            if member.size != record["bytes"] or member.mode != record["mode"]:
                raise ValueError(f"archive length/mode mismatch: {member.name}")
            with package.extractfile(member) as stream:
                if hashlib.file_digest(stream, "sha256").hexdigest() != record["sha256"]:
                    raise BuildIntegrityError(f"archive content mismatch: {member.name}")
            seen.add(member.name)
    if seen != set(records):
        raise ValueError("archive is missing declared source files")


def archive_workspace(source, destination):
    identity = source_identity(source)
    if not {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml"} <= identity["files"].keys():
        raise ValueError("source is not the expected complete Cargo workspace")
    # Ignored files and .git are intentionally excluded. A build that needs an
    # omitted file must fail in the fresh container; caches cannot supply it.
    records = archive(source, identity["files"], destination)
    expected = {name: {"bytes": item["bytes"], "sha256": item["sha256"],
                       "mode": 0o755 if item["mode"] & 0o111 else 0o644}
                for name, item in identity["files"].items()}
    if records != expected:
        raise BuildIntegrityError("archived members differ from the source inventory")
    if destination.stat().st_size > ARCHIVES["source.tar"]:
        raise ValueError("source archive exceeds package byte budget")
    if source_identity(source) != identity:
        raise BuildIntegrityError("source changed while archiving")
    return records, {key: value for key, value in identity.items() if key != "files"}


def pack_source(args):
    source, out = args.source.resolve(strict=True), args.out.resolve()
    if out.is_relative_to(source):
        raise ValueError("source package output must be outside its input")
    out.mkdir(parents=True, exist_ok=False)
    path = out / "myelin-source.tar"
    records, identity = archive_workspace(source, path)
    manifest = {"schema": "myelin-source-package-v1", "source": identity,
                "archive": {"file": path.name, "bytes": path.stat().st_size, "sha256": sha(path)},
                "members": records}
    write_json(out / "source.json", manifest)
    print(json.dumps({"package": str(path), "sha256": manifest["archive"]["sha256"], "source": identity}), flush=True)


def prepare(args):
    source = args.source.resolve(strict=True)
    vendor = args.vendor.resolve(strict=True)
    out = args.out.resolve()
    if out.is_relative_to(source) or out.is_relative_to(vendor):
        raise ValueError("build kit output must be outside its source and vendor inputs")
    out.mkdir(parents=True, exist_ok=False)
    source_records, source_info = archive_workspace(source, out / "source.tar")
    trees = {"source.tar": source_records}
    vendor_names = []
    for path in vendor.rglob("*"):
        if path.is_symlink() or not path.is_dir():
            vendor_names.append(str(path.relative_to(vendor)))
    trees["vendor.tar"] = archive(vendor, vendor_names, out / "vendor.tar")
    image = json.loads(subprocess.check_output([args.podman, "image", "inspect", args.builder_image]))[0]
    image_id = image["Id"].removeprefix("sha256:")
    if not re.fullmatch("[0-9a-f]{64}", image_id):
        raise ValueError("builder is not an immutable image identity")
    command([args.podman, "save", "--format", "oci-archive", "-o", str(out / "toolchain.oci.tar"), image_id], out, "save-toolchain", 600)
    shutil.copyfile(source / "scripts/reproducible-build/build.sh", out / "build.sh")
    artifacts = {}
    for name, limit in ARCHIVES.items():
        size = (out / name).stat().st_size
        if size > limit:
            raise ValueError(f"{name} exceeds package budget {limit}")
        artifacts[name] = {"bytes": size, "sha256": sha(out / name)}
    manifest = {
        "schema": SCHEMA,
        "baseline_revision": source_info["commit"],
        "source_state": "exact listed working-tree files, including uncommitted files; not an assertion of a clean commit",
        "image_id": image_id,
        "image_architecture": image["Architecture"],
        "artifacts": artifacts,
        "trees": trees,
        "recipe": {"script": "build.sh", "packages": ["myelin-session-bundle", "myelin-session-journal"], "features": ["myelin-session-bundle/reference-replayer"], "binaries": list(BINARIES), "profile": "dev, debug=0, incremental=0, opt-level=0", "rust": "1.92.0"},
    }
    write_json(out / "manifest.json", manifest)
    print(json.dumps({"kit": str(out), "manifest_sha256": sha(out / "manifest.json"), "image_id": image_id}), flush=True)


def verify_inputs(kit, expected):
    if not re.fullmatch("[0-9a-f]{64}", expected):
        raise ValueError("expected manifest digest must be 64 lowercase SHA-256 hex characters")
    manifest_path = kit / "manifest.json"
    regular(manifest_path)
    if manifest_path.stat().st_size > 32 << 20 or sha(manifest_path) != expected:
        raise UnknownBuildKit("unrecognized build-kit manifest digest or excessive manifest size")
    manifest = json.loads(manifest_path.read_text())
    if manifest["schema"] != SCHEMA or set(manifest["artifacts"]) != set(ARCHIVES) or set(manifest["trees"]) != {"source.tar", "vendor.tar"}:
        raise ValueError("unsupported build-kit schema or artifact set")
    if not re.fullmatch("[0-9a-f]{64}", manifest["image_id"]):
        raise ValueError("invalid immutable toolchain identity")
    for name, limit in ARCHIVES.items():
        path = kit / name
        regular(path)
        record = manifest["artifacts"][name]
        if path.stat().st_size != record["bytes"] or record["bytes"] > limit or sha(path) != record["sha256"]:
            raise BuildIntegrityError(f"missing, changed or oversized build artifact: {name}")
    for name, records in manifest["trees"].items():
        check_archive(kit / name, records)
    return manifest


def check_outputs(root):
    regular(root / "outputs.json")
    manifest = json.loads((root / "outputs.json").read_text())
    if not set(BINARIES) <= set(manifest["files"]):
        raise ValueError("build did not return all five required executables")
    actual = {str(p.relative_to(root)) for p in root.rglob("*") if not p.is_dir()} - {"outputs.json"}
    if actual != set(manifest["files"]):
        raise ValueError("build output inventory is incomplete")
    for name, record in manifest["files"].items():
        relative(name)
        path = root / name
        regular(path)
        if path.stat().st_size != record["bytes"] or sha(path) != record["sha256"]:
            raise BuildIntegrityError(f"build output mismatch: {name}")
    return manifest


def build_once(podman, manifest, kit, out, number, timeout):
    build = out / f"build-{number}"
    build.mkdir()
    create = [*podman, "create", "--pull=never", "--network=none", "--http-proxy=false", "--cap-drop=ALL",
              "--security-opt=no-new-privileges", "--user=65534:65534", "--pids-limit=512", "--memory=8g", "--cpus=4",
              "--timeout=" + str(timeout), "--label=myelin.build-number=" + str(number),
              "--workdir=/work", "--entrypoint=/bin/sh", manifest["image_id"], "-c",
              "cd /inputs && sha256sum --strict -c approved-inputs.sha256 && exec /bin/sh build.sh"]
    command(create, build, "create")
    cid = (build / "create.stdout").read_text().strip()
    if not re.fullmatch("[0-9a-f]{64}", cid):
        raise ValueError("container creation did not return its exact identity")
    try:
        # Check the actual copies with trusted toolchain utilities before
        # executing even the build script. A file changed after host-side
        # verification cannot silently become a different build input.
        checksums = build / "approved-inputs.sha256"
        checksums.write_text("".join(manifest["artifacts"][name]["sha256"] + "  " + name + "\n"
                                     for name in ("source.tar", "vendor.tar", "build.sh")))
        for name in ("source.tar", "vendor.tar", "build.sh"):
            command([*podman, "cp", str(kit / name), f"{cid}:/inputs/{name}"], build, f"copy-{name}")
        command([*podman, "cp", str(checksums), f"{cid}:/inputs/approved-inputs.sha256"], build, "copy-approved-inputs")
        print(json.dumps({"stage": "building", "build": number, "container": cid, "log": str(build / "build.stderr")}), flush=True)
        command([*podman, "start", "--attach", cid], build, "build", timeout)
        command([*podman, "cp", "--archive=false", f"{cid}:/work/result", str(build / "result")], build, "outputs")
        return check_outputs(build / "result")
    finally:
        command([*podman, "rm", "--force", cid], build, "cleanup")


def record_reproduction(out, manifest, expected_manifest, outputs):
    if len(outputs) != 2:
        raise ValueError("reproduction requires two independent completed builds")
    equal = outputs[0] == outputs[1]
    result = {"schema": REPORT, "manifest_sha256": expected_manifest, "image_id": manifest["image_id"],
              "status": "reproduced" if equal else "mismatch", "comparison": "all binary, runtime-library and tool-version bytes",
              "inputs": manifest["artifacts"], "builds": outputs,
              "scope": "two clean containers and empty Cargo/target directories, network disabled, no host bind mounts; same kernel/CPU architecture",
              "not_claimed": ["source correctness", "official deployment recognition", "bit-identical OCI image rebuilding", "independent physical-machine reproduction", "release performance"]}
    write_json(out / "reproduction.json", result)
    if not equal:
        raise RuntimeError("independent builds differ; inspect reproduction.json")
    print(json.dumps({"status": "reproduced", "report": str(out / "reproduction.json")}), flush=True)


def reproduce(args):
    kit = args.kit.resolve(strict=True)
    manifest = verify_inputs(kit, args.expected_manifest)
    if args.action == "verify-inputs":
        print(json.dumps({"status": "inputs-verified", "execution": "not-run", "manifest_sha256": args.expected_manifest}))
        return
    out = args.out.resolve()
    if out.is_relative_to(kit) or kit.is_relative_to(out):
        raise ValueError("reproduction output and kit must be disjoint")
    out.mkdir(parents=True, exist_ok=False)
    # A fresh image store makes the portable toolchain archive indispensable.
    podman = [args.podman, "--root", str(out / "image-store"), "--runroot", str(out / "run-store"), "--storage-driver=vfs"]
    command([*podman, "load", "-i", str(kit / "toolchain.oci.tar")], out, "load-toolchain", 600)
    loaded = subprocess.check_output([*podman, "image", "inspect", "--format", "{{.Id}}", manifest["image_id"]], text=True).strip().removeprefix("sha256:")
    if loaded != manifest["image_id"]:
        raise ValueError("loaded toolchain differs from the approved immutable image")
    outputs = [build_once(podman, manifest, kit, out, number, args.timeout) for number in (1, 2)]
    record_reproduction(out, manifest, args.expected_manifest, outputs)

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="action", required=True)
    source_parser = commands.add_parser("pack-source", help="export a frozen framework source package for downstream release pins")
    source_parser.add_argument("--source", type=Path, required=True)
    source_parser.add_argument("--out", type=Path, required=True)
    prepare_parser = commands.add_parser("prepare")
    prepare_parser.add_argument("--source", type=Path, required=True)
    prepare_parser.add_argument("--vendor", type=Path, required=True, help="complete cargo vendor --locked --versioned-dirs output")
    prepare_parser.add_argument("--builder-image", required=True, help="explicitly trusted local toolchain image")
    prepare_parser.add_argument("--out", type=Path, required=True)
    for name in ("verify-inputs", "reproduce"):
        subparser = commands.add_parser(name)
        subparser.add_argument("--kit", type=Path, required=True)
        subparser.add_argument("--expected-manifest", required=True, help="SHA-256 received from a trusted source, not from the kit itself")
        if name == "reproduce":
            subparser.add_argument("--out", type=Path, required=True)
            subparser.add_argument("--timeout", type=int, default=3600, help="wall seconds per build before container cleanup")
    for name, subparser in commands.choices.items():
        if name != "pack-source":
            subparser.add_argument("--podman", default="podman")
    args = parser.parse_args()
    if getattr(args, "timeout", 1) < 1 or getattr(args, "timeout", 1) > 14400:
        parser.error("timeout must be between 1 and 14400 seconds")
    if args.action == "pack-source":
        pack_source(args)
    elif args.action == "prepare":
        prepare(args)
    else:
        reproduce(args)


def interrupted(_signum, _frame):
    raise KeyboardInterrupt()


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, interrupted)
    try:
        main()
    except (OSError, ValueError, TypeError, KeyError, tarfile.TarError, RuntimeError, subprocess.SubprocessError, KeyboardInterrupt) as error:
        code = "build-failed"
        for category, name in ((UnknownBuildKit, "unknown-trust-anchor"), (BuildIntegrityError, "integrity"),
                               (FileNotFoundError, "missing-artifact"), (ValueError, "format"),
                               (TypeError, "format"), (KeyError, "format"), (tarfile.TarError, "format"),
                               (subprocess.TimeoutExpired, "deadline"), (KeyboardInterrupt, "cancelled")):
            if isinstance(error, category):
                code = name
                break
        print(json.dumps({"status": "failed", "code": code, "message": str(error) or "build interrupted"}), file=sys.stderr)
        sys.exit(130 if code == "cancelled" else 2)
