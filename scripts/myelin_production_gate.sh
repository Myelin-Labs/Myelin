#!/usr/bin/env bash
# Myelin production-readiness gate.
#
# Runs:
#   - cargo fmt --all --check
#   - git diff --check
#   - cargo check --locked --workspace --all-targets
#   - cargo test --locked --workspace
#   - cargo test -p myelin-consensus
#   - cargo check --locked -p cellscript --all-targets
#   - myelin CLI smoke tests for both consensus modes
#   - Teeworlds acceptance, if the Teeworlds repo path exists
#   - stale-surface grep
#   - forbidden parent-path audit
#
# Exits non-zero on any failure.

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
MYELIN_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
TEEWORLDS_ROOT="${TEEWORLDS_ROOT:-/Users/arthur/RustroverProjects/teeworlds}"
OUTPUT_DIR="${OUTPUT_DIR:-/tmp/myelin-production-gate}"
RUN_TEEWORLDS="${RUN_TEEWORLDS:-1}"

mkdir -p "${OUTPUT_DIR}"

cd "${MYELIN_ROOT}"

run_step() {
  printf '\n==> %s\n' "$1"
  shift
  "$@"
}

# 1. cargo fmt
run_step "Check Rust formatting" cargo fmt --all --check

# 2. git diff
run_step "Check whitespace in git diff" git diff --check

# 3. workspace check
run_step "Check Myelin workspace (locked)" cargo check --locked --workspace --all-targets

# 4. workspace test
run_step "Run focused Myelin protocol tests" \
  cargo test --locked --workspace \
    -p myelin-hashes \
    -p myelin-math \
    -p myelin-exec \
    -p myelin-consensus \
    -p myelin-state \
    -p myelin-mempool \
    -p myelin-utils \
    -p myelin-cli

# 4b. run full workspace tests for state and mempool
run_step "Run myelin-state tests" cargo test --locked -p myelin-state
run_step "Run myelin-mempool tests" cargo test --locked -p myelin-mempool

# 5. consensus tests
run_step "Run myelin-consensus tests" cargo test --locked -p myelin-consensus

# 6. cellscript (must be invoked from the cellscript workspace root)
run_step "Check cellscript (locked)" bash -c "cd cellscript && cargo check --locked -p cellscript --all-targets"

# 7. CLI smoke for both consensus modes
COMMITTEE_CONFIG="${OUTPUT_DIR}/static-committee.toml"
COMMITTEE_REPORT="${OUTPUT_DIR}/static-committee.json"
cat > "${COMMITTEE_CONFIG}" <<'EOF'
kind = "static-closed-committee"

[static_committee]
quorum_weight = 2

[[static_committee.validators]]
id = "validator-0"
public_key = "0101010101010101010101010101010101010101010101010101010101010101"
weight = 1

[[static_committee.validators]]
id = "validator-1"
public_key = "0202020202020202020202020202020202020202020202020202020202020202"
weight = 1
EOF

run_step "Smoke: myelin-cli static-closed-committee finalise" \
  cargo run -p myelin-cli -- committee finalise-demo \
    --config "${COMMITTEE_CONFIG}" \
    --out "${COMMITTEE_REPORT}"

TENDERMINT_CONFIG="${OUTPUT_DIR}/tendermint.toml"
TENDERMINT_REPORT="${OUTPUT_DIR}/tendermint.json"
cat > "${TENDERMINT_CONFIG}" <<'EOF'
kind = "tendermint"

[tendermint]
quorum_power = 2

[[tendermint.validators]]
id = "validator-0"
public_key = "0101010101010101010101010101010101010101010101010101010101010101"
weight = 1

[[tendermint.validators]]
id = "validator-1"
public_key = "0202020202020202020202020202020202020202020202020202020202020202"
weight = 1
EOF

run_step "Smoke: myelin-cli tendermint finalise" \
  cargo run -p myelin-cli -- committee finalise-demo \
    --config "${TENDERMINT_CONFIG}" \
    --out "${TENDERMINT_REPORT}"

# 8. CLI JSON contract
run_step "Validate CLI JSON contract" \
  python3 - "${COMMITTEE_REPORT}" "${TENDERMINT_REPORT}" <<'PY'
import json
import sys
from pathlib import Path

committee = json.loads(Path(sys.argv[1]).read_text())
tendermint = json.loads(Path(sys.argv[2]).read_text())

def require(condition, message):
    if not condition:
        raise SystemExit(f"production gate failed: {message}")

require(committee["consensus_kind"] == "static-closed-committee", "static committee kind")
require(committee["finalised"] is True, "static committee finalised")
require(len(committee["signer_ids"]) >= 2, "static committee signer count")

require(tendermint["consensus_kind"] == "tendermint", "tendermint kind")
require(tendermint["finalised"] is True, "tendermint finalised")
require(len(tendermint["signer_ids"]) >= 2, "tendermint signer count")
require(tendermint["certificate_step"] == "precommit", "tendermint precommit")
require(tendermint["certificate_round"] == 0, "tendermint round 0")
print(json.dumps({
    "static_committee": committee["block_hash"],
    "tendermint": tendermint["block_hash"],
}, indent=2, sort_keys=True))
PY

# 9. Stale-surface grep
run_step "Scan Myelin tree for stale identity" python3 - <<'PY'
import subprocess
import sys

paths = [
    "README.md", "docs", "scripts", "cli", "consensus", "exec", "state", "mempool",
    "crypto", "math", "utils",
]
# The production gate is allowed to name the patterns it scans for; the scan
# itself is not subject to the scan. We exclude the gate and the audit doc
# explicitly so the gate can be the auditor.
exclude = ("scripts/myelin_production_gate.sh", "scripts/myelin_teeworlds_acceptance.sh",
           "scripts/myelin_protocol_gate.sh", "scripts/build_myelin_teeworlds_repro.py",
           "MYELIN_STALE_SURFACE_AUDIT.md", "MYELIN_ARTEFACT_CLEANUP.md",
           "MYELIN_CKB_SEMANTIC_DEVIATIONS.md")

# Forbidden vocabulary for the active Myelin tree.
patterns = [
    "Spora", "spora",
    "NovaSeal", "novaseal",
    "certifier", "certify",
    "website/astro", "website/src",
    "editors/vscode-cellscript",
    "cellscript_gate.sh",
    "novaseal_",
    "release[-_ ]note",
]

failed = False
for pattern in patterns:
    command = ["rg", "-n", "-S", "-i", pattern, *paths]
    result = subprocess.run(command, cwd=".", text=True, capture_output=True)
    if result.returncode == 0:
        lines = [line for line in result.stdout.splitlines() if not any(line.startswith(ex) for ex in exclude)]
        if lines:
            print(f"stale surface match for {pattern!r}:", file=sys.stderr)
            for line in lines:
                print(line, file=sys.stderr)
            failed = True
    elif result.returncode not in (1,):
        print(result.stderr, file=sys.stderr)
        failed = True

if failed:
    raise SystemExit("stale surface scan failed")
print("stale surface scan passed")
PY

# 10. Forbidden parent-path audit
run_step "Audit for forbidden parent Spora path references" python3 - <<'PY'
import subprocess
import sys

paths = [
    "README.md", "docs", "scripts", "cli", "consensus", "exec", "state", "mempool",
    "crypto", "math", "utils", "Cargo.toml", "Cargo.lock",
]
exclude = ("scripts/myelin_production_gate.sh", "MYELIN_STALE_SURFACE_AUDIT.md",
           "MYELIN_ARTEFACT_CLEANUP.md", "MYELIN_CKB_SEMANTIC_DEVIATIONS.md")
patterns = [
    r"path\s*=\s*\"[^\"]*Spora[^\"]*\"",
    r"\.\./\.\./[Ss]pora",
    r"\.\./Spora",
]

failed = False
for pattern in patterns:
    command = ["rg", "-n", "-S", pattern, *paths]
    result = subprocess.run(command, cwd=".", text=True, capture_output=True)
    if result.returncode == 0:
        lines = [line for line in result.stdout.splitlines() if not any(line.startswith(ex) for ex in exclude)]
        if lines:
            print(f"forbidden parent Spora path match for {pattern!r}:", file=sys.stderr)
            for line in lines:
                print(line, file=sys.stderr)
            failed = True
    elif result.returncode not in (1,):
        print(result.stderr, file=sys.stderr)
        failed = True

if failed:
    raise SystemExit("forbidden parent path audit failed")
print("forbidden parent path audit passed")
PY

# 11. Teeworlds acceptance, required by default
if [[ "${RUN_TEEWORLDS}" == "1" ]]; then
  REPLAYER="${TEEWORLDS_ROOT}/ckb/build/replayer_stripped"
  if [[ ! -f "${REPLAYER}" || ! -f "${TEEWORLDS_ROOT}/rust-tools/Cargo.toml" ]]; then
    if [[ "${ALLOW_SKIP_TEEWORLDS:-0}" == "1" ]]; then
      printf '\n==> ALLOW_SKIP_TEEWORLDS=1: missing replayer or rust-tools manifest at %s; skipping Teeworlds acceptance\n' "${TEEWORLDS_ROOT}"
    else
      printf '\nERROR: Teeworlds acceptance is required by default. Missing replayer (%s) or rust-tools manifest (%s).\n' "${REPLAYER}" "${TEEWORLDS_ROOT}/rust-tools/Cargo.toml"
      printf 'Set ALLOW_SKIP_TEEWORLDS=1 to skip Teeworlds explicitly, or set TEEWORLDS_ROOT to a valid clone.\n'
      exit 1
    fi
  else
    TEEWORLDS_OUTPUT_DIR="${OUTPUT_DIR}/teeworlds" \
      run_step "Run Teeworlds acceptance gate" \
        "${SCRIPT_DIR}/myelin_teeworlds_acceptance.sh"

    run_step "Regenerate Teeworlds reproducibility report" \
      python3 "${SCRIPT_DIR}/build_myelin_teeworlds_repro.py"
  fi
else
  printf '\n==> Skip Teeworlds acceptance because RUN_TEEWORLDS=%s\n' "${RUN_TEEWORLDS}"
fi

printf '\nMyelin production gate passed.\n'
printf 'Reports written under: %s\n' "${OUTPUT_DIR}"
