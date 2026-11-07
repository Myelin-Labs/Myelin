"""Run explicitly selected acceptance lanes and retain exact execution evidence."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import signal
import stat
import subprocess
import sys
import time


LANES = ("core", "cellscript", "ckb-devnet", "teeworlds", "browser", "bundle-container", "bundle-build")
PROFILES = {"core": ("core",), "ckb": ("core", "cellscript", "ckb-devnet"),
            "games": ("core", "teeworlds", "browser", "bundle-container", "bundle-build"), "full": LANES}
FIXTURES = ("myelin-exec/vm-ipc,myelin-session-worker/test-fixture,myelin-session-transfer/test-fixture,"
            "myelin-session-authority/test-fixture,myelin-session-bundle/test-fixture")
REMOVED_FLAGS = ("RUN_TEEWORLDS", "RUN_CELLSCRIPT_TOOLCHAIN", "RUN_CKB_DEVNET", "RUN_BUNDLE_CONTAINER",
                 "RUN_BUNDLE_BUILD", "RUN_BROWSER_CONFORMANCE")
SCRATCH = ("ckb-devnet/build", "ckb-devnet/court/chain", "bundle-build/evidence/reproduction/image-store", "bundle-build/evidence/reproduction/run-store",
           "bundle-container/evidence/loaded-store", "bundle-container/evidence/loaded-run")


def now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def sha(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def file_identity(path):
    info = path.lstat()
    if stat.S_ISLNK(info.st_mode):
        return {"kind": "symlink", "target": os.readlink(path)}
    if not stat.S_ISREG(info.st_mode):
        raise ValueError(f"not a regular evidence/source file: {path}")
    return {"kind": "file", "bytes": info.st_size, "mode": stat.S_IMODE(info.st_mode), "sha256": sha(path)}


def source_identity(root):
    def git(*args):
        return subprocess.check_output(["git", "-C", str(root), *args])
    names = set(git("ls-files", "--cached", "--others", "--exclude-standard", "-z").split(b"\0")) - {b""}
    names -= set(git("ls-files", "--deleted", "-z").split(b"\0"))
    files = {os.fsdecode(name): file_identity(root / os.fsdecode(name)) for name in sorted(names)}
    encoded = json.dumps(files, sort_keys=True, separators=(",", ":")).encode()
    return {"commit": git("rev-parse", "HEAD").decode().strip(), "dirty": bool(git("status", "--porcelain")),
            "tree_sha256": hashlib.sha256(encoded).hexdigest(), "files": files}


def artifact_inventory(out):
    files, excluded = {}, []
    def fail(error):
        raise error
    for folder, dirs, names in os.walk(out, onerror=fail, followlinks=False):
        base = Path(folder)
        for name in list(dirs):
            path = base / name
            relative = str(path.relative_to(out))
            if relative in SCRATCH:
                excluded.append(relative)
                dirs.remove(name)
            elif path.is_symlink():
                files[relative] = file_identity(path)
                dirs.remove(name)
        for name in names:
            path = base / name
            if path not in (out / "report.json", out / "report.pending"):
                files[str(path.relative_to(out))] = file_identity(path)
    return files, excluded


class Failed(RuntimeError):
    pass


class Run:
    def __init__(self, root, out, selected, profile, timeout):
        self.root, self.out, self.timeout = root, out, timeout
        self.env = os.environ.copy()
        self.env.setdefault("CARGO_BUILD_JOBS", "4")
        if platform.system() == "Darwin" and Path("/opt/homebrew/opt/llvm/lib").is_dir():
            self.env.setdefault("LIBCLANG_PATH", "/opt/homebrew/opt/llvm/lib")
        self.report = {"schema": "myelin-validation-v1", "status": "running", "started_at": now(),
                       "profile": profile, "selected": selected, "selected_profile_passed": False,
                       "platform": {"system": platform.system(), "release": platform.release(), "machine": platform.machine(),
                                    "logical_cpus": os.cpu_count()},
                       "build_environment": {key: self.env[key] for key in ("CARGO_BUILD_JOBS", "CARGO_TARGET_DIR", "CARGO_HOME",
                           "CARGO_NET_OFFLINE", "CARGO_PROFILE_DEV_DEBUG", "CARGO_PROFILE_TEST_DEBUG", "CARGO_INCREMENTAL",
                           "RUSTUP_TOOLCHAIN", "RUSTFLAGS", "RUSTDOCFLAGS", "LIBCLANG_PATH", "CC", "CXX", "CFLAGS", "CXXFLAGS")
                           if key in self.env},
                       "ci": {key: os.environ[key] for key in ("GITHUB_SHA", "GITHUB_RUN_ID", "GITHUB_RUN_ATTEMPT",
                              "GITHUB_JOB", "GITHUB_SERVER_URL", "GITHUB_REPOSITORY") if key in os.environ},
                       "lanes": {name: {"status": "pending" if name in selected else "skipped", "steps": [],
                                        "reason": None if name in selected else "not selected"} for name in LANES}}
        ci = self.report["ci"]
        if all(ci.get(key) for key in ("GITHUB_SERVER_URL", "GITHUB_REPOSITORY", "GITHUB_RUN_ID")):
            ci["run_url"] = f"{ci['GITHUB_SERVER_URL']}/{ci['GITHUB_REPOSITORY']}/actions/runs/{ci['GITHUB_RUN_ID']}"
        self.save()

    def save(self):
        temporary = self.out / "report.pending"
        temporary.write_text(json.dumps(self.report, indent=2, sort_keys=True) + "\n")
        temporary.replace(self.out / "report.json")

    def step(self, lane, name, argv, env=None):
        folder = self.out / lane
        folder.mkdir(exist_ok=True)
        log = folder / (name + ".log")
        entry = {"name": name, "command": [str(a) for a in argv], "cwd": str(self.root), "started_at": now(),
                 "status": "running", "log": str(log.relative_to(self.out)), "timeout_seconds": self.timeout}
        self.report["lanes"][lane]["steps"].append(entry)
        self.save()
        print(f"[{lane}] {name}", flush=True)
        started = time.monotonic()
        process = None
        try:
            with log.open("wb") as output:
                process = subprocess.Popen(entry["command"], cwd=self.root, env=self.env | (env or {}),
                                           stdout=output, stderr=subprocess.STDOUT, start_new_session=True)
                entry["exit_code"] = process.wait(timeout=self.timeout)
            if process.returncode:
                raise Failed(f"{lane}/{name} exited {process.returncode}; see {log}")
            entry["status"] = "passed"
        except BaseException as error:
            entry["status"] = "cancelled" if isinstance(error, KeyboardInterrupt) else "failed"
            entry["error"] = str(error) or type(error).__name__
            raise
        finally:
            if process is not None and process.poll() is None:
                try:
                    os.killpg(process.pid, signal.SIGTERM)
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
                except ProcessLookupError:
                    process.wait()
            entry["wall_seconds"] = time.monotonic() - started
            self.save()
        return log

    def required(self, key):
        value = self.env.get(key)
        if not value:
            raise Failed(f"selected lane requires {key}")
        return value

    def linux(self):
        if platform.system() != "Linux" or platform.machine() not in ("x86_64", "aarch64"):
            raise Failed("selected isolation lane requires Linux x86_64/aarch64")

    def lane(self, name):
        step = lambda label, command, env=None: self.step(name, label, command, env)
        script = lambda filename: str(self.root / "scripts" / filename)
        python = lambda filename: [sys.executable, script(filename)]
        if name == "core":
            step("python-version", [sys.executable, "--version"])
            step("rustc-version", ["rustc", "-vV"])
            step("cargo-version", ["cargo", "-V"])
            step("clang-version", ["clang", "--version"])
            step("cxx-version", ["c++", "--version"])
            step("protoc-version", ["protoc", "--version"])
            step("format", ["cargo", "fmt", "--all", "--check"])
            step("diff", ["git", "diff", "HEAD", "--check"])
            # Clippy checks all targets; a separate cargo check would repeat it.
            step("clippy", ["cargo", "clippy", "--locked", "--workspace", "--all-targets", "--all-features", "--", "-D", "warnings"])
            step("workspace", ["cargo", "test", "--locked", "--workspace", "--features", FIXTURES, "--", "--test-threads=1"],
                 {"MYELIN_WORKER_METRICS_OUT": str(self.out / "native-worker-metrics.json"),
                  "MYELIN_BUNDLE_RUNNER_EVIDENCE_OUT": str(self.out / "isolated-bundle"),
                  "MYELIN_BUNDLE_MIGRATION_EVIDENCE_OUT": str(self.out / "isolated-migration"),
                  "MYELIN_BUNDLE_COLLECTION_EVIDENCE_OUT": str(self.out / "archive-collection"),
                  "MYELIN_DEPENDENCY_EVIDENCE_OUT": str(self.out / "epoch-dependencies")})
            step("python-boundaries", [sys.executable, "-m", "unittest", "discover", "-s", "scripts", "-p", "test_*.py"])
            # The workspace test pass already builds examples with their real main.
            metadata = step("cargo-metadata", ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"])
            target = Path(json.loads(metadata.read_text())["target_directory"])
            step("transfer-cli", python("myelin_transfer_retirement_acceptance.py") +
                 ["--binary", str(target / "debug/examples/shared_store_transfer"),
                  "--remote-binary", str(target / "debug/examples/remote_receipt"),
                  "--terminal-binary", str(target / "debug/examples/remote_transfer"), "--output", str(self.out / "transfer.json")])
        elif name == "cellscript":
            step("reproduce-compiler", [script("myelin_cellscript_toolchain_gate.sh")],
                 {"CELLSCRIPT_EVIDENCE_ROOT": str(self.out / "cellscript/evidence")})
        elif name == "ckb-devnet":
            fixture = self.root / "fixtures/ckb-microgame/Cargo.toml"
            build = self.out / "ckb-devnet/build"
            evidence = self.out / "ckb-devnet/court/evidence"
            program = build / "riscv64imac-unknown-none-elf/release/myelin-microgame-court"
            replayer = build / "release/myelin-microgame-replay"
            step("court-toolchain", ["rustc", "+1.95.0", "-vV"])
            step("court-format", ["cargo", "+1.95.0", "fmt", "--all", "--manifest-path", str(fixture), "--", "--check"])
            step("court-program", ["cargo", "+1.95.0", "build", "--locked", "--release", "--manifest-path", str(fixture),
                 "--target-dir", str(build), "--target", "riscv64imac-unknown-none-elf", "-p", "myelin-microgame-court"],
                 {"CARGO_TARGET_RISCV64IMAC_UNKNOWN_NONE_ELF_RUSTFLAGS": "-C target-feature=-a -C passes=lower-atomic"})
            step("court-replayer", ["cargo", "+1.95.0", "build", "--locked", "--release", "--manifest-path", str(fixture),
                 "--target-dir", str(build), "-p", "myelin-microgame-replay"])
            step("court-transactions", python("myelin_ckb_microgame.py") + ["--ckb", self.required("CKB_BIN"), "--program", str(program),
                 "--replayer", str(replayer), "--out", str(evidence.parent)])
            step("court-package", python("myelin_microgame_archive.py") + ["package", "--evidence", str(evidence),
                 "--replayer", str(replayer), "--out", str(self.out / "ckb-devnet/package.json")])
        elif name == "teeworlds":
            destination = self.out / "teeworlds/evidence"
            step("game-acceptance", [script("myelin_teeworlds_acceptance.sh")], {"OUTPUT_DIR": str(destination)})
            step("game-report", python("build_myelin_teeworlds_repro.py"), {"MYELIN_TEEWORLDS_ACCEPTANCE_DIR": str(destination)})
        elif name == "browser":
            step("native-wasm", python("myelin_browser_conformance.py") + ["--browser", self.required("MYELIN_BROWSER"),
                 "--wasm-bindgen", self.required("MYELIN_WASM_BINDGEN"), "--out", str(self.out / "browser/evidence")])
        elif name == "bundle-container":
            self.linux()
            evidence = self.out / "isolated-bundle" if self.report["lanes"]["core"]["status"] == "passed" else Path(self.required("MYELIN_BUNDLE_EVIDENCE"))
            cli = evidence / "myelin-session-bundle" if self.report["lanes"]["core"]["status"] == "passed" else Path(self.required("MYELIN_TRUSTED_BUNDLE_CLI"))
            step("offline-restore", python("myelin_bundle_container_acceptance.py") + ["--evidence", str(evidence),
                 "--trusted-cli", str(cli), "--trusted-launcher", self.env.get("MYELIN_BWRAP", "/usr/bin/bwrap"),
                 "--out", str(self.out / "bundle-container/evidence")])
        elif name == "bundle-build":
            self.linux()
            cli = self.out / "isolated-bundle/myelin-session-bundle" if self.report["lanes"]["core"]["status"] == "passed" else Path(self.required("MYELIN_TRUSTED_BUNDLE_CLI"))
            step("archived-build", python("myelin_bundle_build_acceptance.py") + ["--kit", self.required("MYELIN_BUILD_KIT"),
                 "--expected-manifest", self.required("MYELIN_BUILD_KIT_SHA256"), "--outputs", self.required("MYELIN_BUILD_OUTPUTS"),
                 "--trusted-cli", str(cli), "--out", str(self.out / "bundle-build/evidence")])

    def finish(self, error=None):
        if error is not None:
            self.report["error"] = str(error) or type(error).__name__
        for lane in self.report["lanes"].values():
            if lane["status"] == "pending":
                lane.update(status="skipped", reason="not executed after an earlier failure or cancellation")
        self.report["selected_profile_passed"] = error is None and all(
            self.report["lanes"][name]["status"] == "passed" for name in self.report["selected"])
        self.report["status"] = ("passed" if self.report["selected_profile_passed"] else
                                 "cancelled" if isinstance(error, KeyboardInterrupt) else "failed")
        self.report["finished_at"] = now()
        try:
            self.report["artifacts"], self.report["excluded_container_scratch"] = artifact_inventory(self.out)
        except (OSError, ValueError) as caught:
            self.report.update(status="failed", selected_profile_passed=False, error=f"artifact inventory: {caught}")
        self.save()

    def execute(self):
        error = None
        current = None
        try:
            self.report["source"] = source_identity(self.root)
            self.save()
            for current in self.report["selected"]:
                self.report["lanes"][current]["status"] = "running"
                self.save()
                self.lane(current)
                self.report["lanes"][current]["status"] = "passed"
                self.save()
            final = source_identity(self.root)
            self.report["source_after"] = {k: v for k, v in final.items() if k != "files"}
            if final != self.report["source"]:
                raise Failed("source changed during validation; no complete source-bound pass")
        except (OSError, ValueError, KeyError, Failed, subprocess.SubprocessError, KeyboardInterrupt) as caught:
            error = caught
            if current and self.report["lanes"][current]["status"] == "running":
                self.report["lanes"][current].update(status="cancelled" if isinstance(caught, KeyboardInterrupt) else "failed",
                                                     reason=str(caught) or type(caught).__name__)
        finally:
            self.finish(error)
        return 0 if self.report["selected_profile_passed"] else 1


def interrupted(*_):
    raise KeyboardInterrupt()


def main():
    parser = argparse.ArgumentParser(prog="myelin_production_gate.sh", description=__doc__)
    selection = parser.add_mutually_exclusive_group()
    selection.add_argument("--profile", choices=PROFILES)
    selection.add_argument("--lane", action="append", choices=LANES, help="repeat to select independent required lanes")
    parser.add_argument("--out", type=Path)
    parser.add_argument("--timeout-seconds", type=int, default=7200, help="finite wall limit for each command")
    args = parser.parse_args()
    if sys.version_info < (3, 11):
        parser.error("Python 3.11 or newer is required")
    removed = [key for key in REMOVED_FLAGS if key in os.environ]
    if removed:
        parser.error("removed environment flags: " + ", ".join(removed) + "; select --profile or --lane explicitly")
    if not 1 <= args.timeout_seconds <= 14400:
        parser.error("timeout must be between 1 and 14400 seconds")
    root = Path(__file__).resolve().parent.parent
    out = (args.out or root / "target/validation").resolve()
    if out.is_relative_to(root) and not out.is_relative_to(root / "target"):
        parser.error("validation output must be outside the source tree or under target/")
    selected = [name for name in LANES if name in args.lane] if args.lane else list(PROFILES[args.profile or "core"])
    out.mkdir(parents=True, exist_ok=False)
    signal.signal(signal.SIGTERM, interrupted)
    run = Run(root, out, selected, "custom" if args.lane else args.profile or "core", args.timeout_seconds)
    result = run.execute()
    print(json.dumps({"status": run.report["status"], "selected": selected, "report": str(out / "report.json")}), flush=True)
    return result


if __name__ == "__main__":
    sys.exit(main())
