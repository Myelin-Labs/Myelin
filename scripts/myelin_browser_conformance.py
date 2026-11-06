#!/usr/bin/env python3
"""Execute native vectors against the shared game rules in real browser WASM.

No workspace test rerun and no Node simulation of a browser. Requested browser
or binding tools must exist; failures never become a skipped/pass report.
"""
import argparse
import functools
import hashlib
import http.server
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading


PAGE = """<!doctype html><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; connect-src 'self'">
<title>Myelin browser conformance</title><pre id="result">pending</pre><script type="module" src="runner.js"></script>"""

RUNNER = """const result = document.getElementById('result');
try {
  const vectors = await (await fetch('native.json')).json();
  if (vectors.format !== 'myelin-game-conformance-v1' || vectors.vectors.length === 0) throw Error('invalid vector set');
  const binary = await (await fetch('checker_bg.wasm')).arrayBuffer();
  const observations = [];
  let comparisons = 0, alteredInputs = 0, alteredSnapshots = 0;
  for (const [index, vector] of vectors.vectors.entries()) {
    // A separate module and WASM instance per vector, including resumed snapshots.
    const module = await import(`./checker.js?instance=${index}`);
    await module.default({module_or_path: binary});
    for (const expected of vector.expected) {
      const actual = JSON.parse(module.run_case(JSON.stringify(vector.scenario), expected.width, expected.restart));
      if (JSON.stringify(actual) !== JSON.stringify(expected)) {
        throw Error(`mismatch: ${vector.scenario.name}, width=${expected.width}, restart=${expected.restart}`);
      }
      comparisons++;
    }
    const baseline = vector.expected[0];
    const changed = structuredClone(vector.scenario);
    changed.commands[0].push(32); // Valid JSON whitespace, but not canonical application bytes.
    const actual = JSON.parse(module.run_case(JSON.stringify(changed), baseline.width, baseline.restart));
    if (!actual.error || JSON.stringify(actual) === JSON.stringify(baseline)) throw Error('changed input was accepted');
    alteredInputs++;
    const corrupt = structuredClone(vector.scenario);
    corrupt.initial[5] ^= 1; // Change profile without changing trusted scope.
    let rejected = false;
    try { module.run_case(JSON.stringify(corrupt), baseline.width, baseline.restart); } catch { rejected = true; }
    if (!rejected) throw Error('foreign snapshot was accepted');
    alteredSnapshots++;
    observations.push({name: vector.scenario.name, comparisons: vector.expected.length, fresh_wasm_instance: true});
  }
  result.textContent = JSON.stringify({status: 'passed', comparisons, altered_inputs_rejected: alteredInputs,
    altered_snapshots_rejected: alteredSnapshots, user_agent: navigator.userAgent, observations});
} catch (error) {
  result.textContent = JSON.stringify({status: 'failed', error: String(error)});
}
"""


class Handler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *_args):
        pass


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--browser", required=True, help="Chrome/Chromium executable")
    parser.add_argument("--wasm-bindgen", required=True, help="CLI matching Cargo.lock")
    parser.add_argument("--out", type=Path, required=True, help="New evidence directory")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    browser = shutil.which(args.browser)
    binding = shutil.which(args.wasm_bindgen)
    node = shutil.which("node")
    if not browser or not binding or not node:
        parser.error("browser, wasm-bindgen and Node 22+ executables are required")
    output = args.out.resolve()
    output.mkdir(parents=True, exist_ok=False)
    web = output / "web"
    web.mkdir()
    env = dict(os.environ)
    env.setdefault("CARGO_BUILD_JOBS", "4")

    def command(argv, name, *, stdout=None, timeout=1200):
        with (output / (name + ".log")).open("wb") as log:
            return subprocess.run(argv, cwd=root, env=env, stdout=stdout or log, stderr=log, timeout=timeout, check=True)

    command(["cargo", "build", "--locked", "-p", "myelin-session-journal", "--example", "browser_vectors"], "native-build")
    command(["cargo", "build", "--locked", "-p", "myelin-game-browser-fixture", "--target", "wasm32-unknown-unknown"], "wasm-build")
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"], cwd=root, env=env))
    target = Path(metadata["target_directory"])
    native = target / "debug/examples/browser_vectors"
    wasm = target / "wasm32-unknown-unknown/debug/myelin_game_browser_fixture.wasm"
    # Retain exact executable bytes before executing or translating them.
    native_copy = output / "browser_vectors"
    shutil.copy2(native, native_copy)
    wasm_copy = output / "checker-input.wasm"
    shutil.copy2(wasm, wasm_copy)
    with (web / "native.json").open("wb") as vectors:
        command([str(native_copy)], "native-execution", stdout=vectors, timeout=60)
    command([binding, str(wasm_copy), "--target", "web", "--out-dir", str(web), "--out-name", "checker"], "wasm-bindgen", timeout=60)
    (web / "index.html").write_text(PAGE)
    (web / "runner.js").write_text(RUNNER)
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), functools.partial(Handler, directory=str(web)))
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix="myelin-browser-") as profile, (output / "browser.json").open("wb") as observed:
            command([node, str(root / "scripts/run_browser_conformance.mjs"), browser,
                     f"http://127.0.0.1:{server.server_port}/", profile],
                    "browser-execution", stdout=observed, timeout=120)
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)
    observed = json.loads((output / "browser.json").read_text())
    (output / "browser.html").write_text(observed["html"])
    result = observed["result"]
    if result.get("status") != "passed" or result.get("comparisons", 0) == 0:
        raise RuntimeError(result)
    files = {}
    for path in sorted(output.rglob("*")):
        if path.is_file():
            with path.open("rb") as source:
                files[str(path.relative_to(output))] = {"sha256": hashlib.file_digest(source, "sha256").hexdigest(), "bytes": path.stat().st_size}
    report = {"format": "myelin-browser-conformance-v1", "browser": subprocess.check_output([browser, "--version"], text=True).strip(),
              "binding_tool": subprocess.check_output([binding, "--version"], text=True).strip(), "result": result, "files": files}
    (output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"status": "passed", "comparisons": result["comparisons"], "report": str(output / "report.json")}))


if __name__ == "__main__":
    main()
