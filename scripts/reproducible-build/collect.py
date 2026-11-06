"""Capture outputs and libraries of the explicitly approved source build."""
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys

target = sys.argv[1]
result = Path("/work/result")
runtime = result / "runtime"
runtime.mkdir()
for name in ("myelin-reference-transfer-replayer", "myelin-session-bundle", "inventory", "turn_based", "fixed_tick"):
    source = Path("/work/target") / target / "debug"
    source = source / ("examples/" + name if name in ("inventory", "turn_based", "fixed_tick") else name)
    shutil.copyfile(source, result / name)
    (result / name).chmod(0o755)
    # These are outputs of the approved build, never arbitrary bundle inputs.
    dependencies = subprocess.check_output(["ldd", str(source)], text=True)
    if "not found" in dependencies:
        raise RuntimeError(dependencies)
    for line in dependencies.splitlines():
        paths = [word for word in line.split() if word.startswith("/")]
        if paths:
            library = Path(paths[0])
            destination = runtime / library.relative_to("/")
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(library, destination)
            destination.chmod(0o755)
files = {}
for path in sorted(result.rglob("*")):
    if path.is_file():
        with path.open("rb") as stream:
            identity = hashlib.file_digest(stream, "sha256").hexdigest()
        files[str(path.relative_to(result))] = {"sha256": identity, "bytes": path.stat().st_size}
(result / "outputs.json").write_text(json.dumps({"target": target, "files": files}, indent=2) + "\n")
