#!/usr/bin/env python3
"""Replay a retained fixture in a fresh, offline Linux container.

The CLI and bubblewrap paths are OPERATOR-TRUSTED executables: ldd is allowed
only on those two inputs, never on any program supplied by the bundle. The
application itself is run only by the existing isolated replay boundary.
"""

import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import stat
import subprocess
import time


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def run(command, out, label, timeout=120):
    started = time.monotonic()
    with (out / f"{label}.stdout").open("wb") as stdout, (out / f"{label}.stderr").open("wb") as stderr:
        process = subprocess.run(command, stdout=stdout, stderr=stderr, timeout=timeout, check=False)
    result = {"command": command, "exit_code": process.returncode, "wall_seconds": time.monotonic() - started}
    (out / f"{label}.json").write_text(json.dumps(result, indent=2) + "\n")
    if process.returncode:
        raise RuntimeError(f"{label} failed ({process.returncode}); see {out / (label + '.stderr')}")
    return result


def copy_trusted_program(source, destination, root, copied):
    """Package all loader-reported libraries of an explicitly trusted program."""
    source = source.resolve(strict=True)
    if not source.is_file():
        raise ValueError(f"not a regular trusted executable: {source}")
    target = root / destination.lstrip("/")
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, target)
    target.chmod(0o755)
    result = subprocess.run(["ldd", str(source)], capture_output=True, text=True, check=False, timeout=30)
    if result.returncode and "not a dynamic executable" not in result.stderr + result.stdout:
        raise RuntimeError(f"trusted executable dependency discovery failed: {result.stderr}")
    if "not found" in result.stdout:
        raise RuntimeError(f"trusted executable has missing libraries: {result.stdout}")
    libraries = []
    for line in result.stdout.splitlines():
        paths = [part for part in line.split() if part.startswith("/")]
        if not paths:
            continue
        path = Path(paths[0])
        if not re.fullmatch(r"/(?:usr/)?lib(?:64)?/[A-Za-z0-9_./+\-]+", str(path)) or ".." in path.parts:
            raise ValueError(f"unsupported trusted-library destination: {path}")
        resolved = path.resolve(strict=True)
        identity = digest(resolved)
        if str(path) in copied and copied[str(path)] != identity:
            raise ValueError(f"conflicting trusted runtime libraries: {path}")
        target = root / path.relative_to("/")
        target.parent.mkdir(parents=True, exist_ok=True)
        if str(path) not in copied:
            shutil.copyfile(resolved, target)
            target.chmod(0o555)
        copied[str(path)] = identity
        libraries.append({"path": str(path), "sha256": identity})
    return {"source": str(source), "destination": destination, "sha256": digest(source), "libraries": libraries}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evidence", type=Path, required=True, help="retained bundle fixture directory")
    parser.add_argument("--trusted-cli", type=Path, required=True, help="operator-approved bundle CLI; ldd may execute it")
    parser.add_argument("--trusted-launcher", type=Path, default=Path("/usr/bin/bwrap"), help="operator-approved bubblewrap")
    parser.add_argument("--out", type=Path, required=True, help="new output directory, never overwritten")
    parser.add_argument("--podman", default="podman", help="Podman executable (rootless is supported)")
    args = parser.parse_args()
    evidence = args.evidence.resolve(strict=True)
    out = args.out.resolve()
    if out.is_relative_to(evidence) or evidence.is_relative_to(out):
        raise ValueError("output and input evidence directories must be disjoint")
    out.mkdir(parents=True, exist_ok=False)
    root = out / "root"
    root.mkdir()
    (root / "tmp").mkdir(mode=0o1777)
    (root / "tmp").chmod(0o1777)
    libraries = {}
    programs = [
        copy_trusted_program(args.trusted_cli, "/usr/bin/myelin-session-bundle", root, libraries),
        copy_trusted_program(args.trusted_launcher, "/usr/bin/bwrap", root, libraries),
    ]
    # Preserve symlinks so the normal importer can reject them, rather than
    # following an archive link while constructing the container context.
    shutil.copytree(evidence / "bundle", root / "evidence/bundle", symlinks=True)
    operator = root / "operator"
    operator.mkdir()
    for name in ("trust.json", "limits.json"):
        shutil.copyfile(evidence / name, operator / name)
    policy = json.loads((evidence / "policy.json").read_text())
    policy["launcher"] = "/usr/bin/bwrap"
    (operator / "policy.json").write_text(json.dumps(policy) + "\n")
    expected = json.loads((evidence / "cli-report.json").read_text())
    session = bytes(expected["replayed_session"]).hex()
    files = {}
    for path in sorted(root.rglob("*")):
        mode = path.lstat().st_mode
        if stat.S_ISREG(mode):
            files[str(path.relative_to(root))] = {"bytes": path.stat().st_size, "sha256": digest(path), "mode": stat.S_IMODE(mode)}
        elif not stat.S_ISDIR(mode):
            raise ValueError(f"runtime kit must contain regular files and directories only: {path}")
    (out / "runtime-files.json").write_text(json.dumps(files, indent=2) + "\n")
    (out / "Dockerfile").write_text(
        'FROM scratch\nCOPY root/ /\nENV TMPDIR=/tmp\nENTRYPOINT ["/usr/bin/myelin-session-bundle"]\n'
    )
    # Build context excludes future image exports, reports and all host paths.
    (out / ".dockerignore").write_text("*\n!Dockerfile\n!root\n!root/**\n")
    image_tag = "localhost/myelin-offline-replay:" + digest(out / "runtime-files.json")[:24]
    build = run([args.podman, "build", "--pull=false", "--network=none", "-t", image_tag, str(out)], out, "build")
    image_id = subprocess.check_output([args.podman, "image", "inspect", "--format", "{{.Id}}", image_tag], text=True).strip()
    replay_args = ["replay", "/evidence/bundle", "/operator/trust.json", session, "/operator/policy.json", "/operator/limits.json"]
    command = [
        args.podman, "run", "--rm", "--pull=never", "--network=none", "--read-only", "--cap-drop=ALL", "--user=65534:65534",
        "--security-opt=no-new-privileges", "--security-opt=seccomp=unconfined",
        # The trusted launcher must create its own PID/user namespaces and
        # proc mount. Outer OCI masked proc mounts prohibit that even with a
        # new user namespace. The inner runner still requires its own strict
        # seccomp filter, read-only mounts and no capabilities; no fallback.
        "--security-opt=unmask=ALL", "--http-proxy=false",
        "--pids-limit=32", "--memory=2g", "--cpus=2", "--tmpfs=/tmp:rw,nosuid,nodev,size=1g", "--workdir=/",
    ]
    command += [image_id, *replay_args]
    replay = run(command, out, "replay", timeout=180)
    report = json.loads((out / "replay.stdout").read_text())
    if report["result"] != expected["result"] or report["host_replay"] != "verified":
        raise RuntimeError("fresh-container replay differs from the original exact result/receipt")
    archive = out / "runtime-image.tar"
    run([args.podman, "save", "-o", str(archive), "--format", "oci-archive", image_id], out, "save")
    # Restore using an empty image store. Neither the original built image nor
    # the host's image cache can satisfy missing runtime artifacts here.
    fresh_store = [args.podman, "--root", str(out / "loaded-store"), "--runroot", str(out / "loaded-run"), "--storage-driver=vfs"]
    run([*fresh_store, "load", "-i", str(archive)], out, "load")
    restored = run([*fresh_store, *command[1:]], out, "restored-replay", timeout=180)
    restored_report = json.loads((out / "restored-replay.stdout").read_text())
    if restored_report["result"] != expected["result"] or restored_report["host_replay"] != "verified":
        raise RuntimeError("replay after offline image restore differs from the original exact result/receipt")
    summary = {
        "schema": "myelin-bundle-container-acceptance-v1",
        "scope": "fresh offline container on the same Linux kernel; no source checkout, Cargo cache, original database or host bind mounts",
        "not_claimed": ["second physical machine", "reproducible application builds", "official deployment trust", "CKB-VM/DA/L1/court verification"],
        "image_id": image_id, "image_archive_sha256": digest(archive), "runtime_files_sha256": digest(out / "runtime-files.json"),
        "trusted_programs": programs, "build": build, "replay": replay, "restored_replay": restored,
        "result": report["result"], "execution": report["execution"],
        "image_load": [args.podman, "load", "-i", "runtime-image.tar"],
        "offline_replay": command,
    }
    (out / "acceptance.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps({"status": "passed", "report": str(out / "acceptance.json"), "image": image_id}))


if __name__ == "__main__":
    main()
