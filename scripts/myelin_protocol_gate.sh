#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
MYELIN_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
OUTPUT_DIR="${OUTPUT_DIR:-/tmp/myelin-protocol-gate}"
RUN_TEEWORLDS="${RUN_TEEWORLDS:-1}"

mkdir -p "${OUTPUT_DIR}"

run_step() {
  printf '\n==> %s\n' "$1"
  shift
  "$@"
}

cd "${MYELIN_ROOT}"

run_step "Scan active Myelin tree for inherited L1 and removed serializer semantics" \
  python3 - <<'PY'
import subprocess
import sys

paths = [
    "README.md",
    "cellscript",
    "docs",
    "scripts",
    "cli",
    "consensus",
    "exec",
    "state",
    "mempool",
    "crypto",
    "math",
    "utils",
]
# The protocol gate and the production gate are allowed to name the patterns
# they scan for; the scan itself is not subject to the scan. We exclude
# the gate scripts and the audit documents explicitly so the gate can be
# the auditor.
exclude = ("scripts/myelin_protocol_gate.sh", "scripts/myelin_production_gate.sh",
           "scripts/myelin_teeworlds_acceptance.sh", "scripts/build_myelin_teeworlds_repro.py",
           "MYELIN_STALE_SURFACE_AUDIT.md", "MYELIN_ARTEFACT_CLEANUP.md",
           "MYELIN_CKB_SEMANTIC_DEVIATIONS.md")
patterns = [
    "Spo" + "ra",
    "spo" + "ra",
    "GHOST" + "DAG",
    "ghost" + "dag",
    "blue" + ".?score",
    "blue" + "_score",
    r"\b" + "D" + "AA" + r"\b",
    "difficulty " + "adjustment",
    "Bo" + "rsh",
    "bo" + "rsh",
    "bin" + "code",
]

failed = False
for pattern in patterns:
    command = ["rg", "-n", "-S", "-i", pattern, *paths]
    result = subprocess.run(command, cwd=".", text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if result.returncode == 0:
        lines = [line for line in result.stdout.splitlines() if not any(line.startswith(ex) for ex in exclude)]
        if lines:
            print(f"removed semantic match for {pattern!r}:", file=sys.stderr)
            for line in lines:
                print(line, file=sys.stderr)
            failed = True
    elif result.returncode not in (1,):
        print(result.stderr, file=sys.stderr)
        failed = True

if failed:
    raise SystemExit(1)
print("active removed-semantics scan passed")
PY

run_step "Check native dependency graph has no removed serializer package" \
  python3 - <<'PY'
import subprocess
import sys

removed_serializers = ["bo" + "rsh", "bin" + "code"]
for removed_serializer in removed_serializers:
    commands = [
        ["cargo", "tree", "-i", removed_serializer],
        ["cargo", "tree", "-p", "myelin-exec", "-i", removed_serializer],
    ]
    for command in commands:
        result = subprocess.run(command, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        combined = result.stdout + result.stderr
        if result.returncode == 0:
            print(combined, file=sys.stderr)
            raise SystemExit(f"unexpected removed serializer dependency reported by: {' '.join(command)}")
        if "did not match any packages" not in combined:
            print(combined, file=sys.stderr)
            raise SystemExit(f"could not prove removed serializer absence with: {' '.join(command)}")
print("removed serializer dependency absence passed")
PY

run_step "Check Rust formatting" cargo fmt --all --check
run_step "Check whole Myelin workspace" cargo check --workspace
run_step "Run focused Myelin protocol tests" \
  cargo test \
    -p myelin-hashes \
    -p myelin-math \
    -p myelin-exec \
    -p myelin-consensus \
    -p myelin-state \
    -p myelin-mempool \
    -p myelin-utils \
    -p myelin-cli

CELLTX_REPORT="${OUTPUT_DIR}/celltx-simple-report.json"
COMMITTEE_CONFIG="${OUTPUT_DIR}/static-committee.toml"
COMMITTEE_REPORT="${OUTPUT_DIR}/committee-finalise-demo.json"
TENDERMINT_CONFIG="${OUTPUT_DIR}/tendermint.toml"
TENDERMINT_REPORT="${OUTPUT_DIR}/tendermint-finalise-demo.json"

run_step "Emit simple CellTx execution report" \
  cargo run -p myelin-cli -- celltx simple-report --out "${CELLTX_REPORT}"

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

[[static_committee.validators]]
id = "validator-2"
public_key = "0303030303030303030303030303030303030303030303030303030303030303"
weight = 1
EOF

run_step "Finalise demo block through selected static closed committee consensus" \
  cargo run -p myelin-cli -- committee finalise-demo \
    --config "${COMMITTEE_CONFIG}" \
    --out "${COMMITTEE_REPORT}"

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

[[tendermint.validators]]
id = "validator-2"
public_key = "0303030303030303030303030303030303030303030303030303030303030303"
weight = 1
EOF

run_step "Finalise demo block through selected Tendermint consensus" \
  cargo run -p myelin-cli -- committee finalise-demo \
    --config "${TENDERMINT_CONFIG}" \
    --out "${TENDERMINT_REPORT}"

run_step "Validate CellTx and committee evidence" \
  python3 - "${CELLTX_REPORT}" "${COMMITTEE_REPORT}" "${TENDERMINT_REPORT}" <<'PY'
import json
import sys
from pathlib import Path

celltx = json.loads(Path(sys.argv[1]).read_text())
committee = json.loads(Path(sys.argv[2]).read_text())
tendermint = json.loads(Path(sys.argv[3]).read_text())

def require(condition, message):
    if not condition:
        raise SystemExit(f"protocol gate failed: {message}")

require(celltx["status"] == "Accepted", "simple CellTx report must be accepted")
require(celltx["semantic_profile"] == "ckb-compatible", "simple CellTx must use ckb-compatible semantic profile")
require(celltx["ckb_projection"]["ckb_projection_possible"] is True, "simple CellTx must project into CKB shape")
require(celltx["ckb_projection"]["molecule_transaction_bytes"] > 0, "simple CellTx must emit Molecule transaction bytes")
require(len(celltx["state_root_after"]) == 32, "simple CellTx must emit a 32-byte state root")
require(committee["consensus_kind"] == "static-closed-committee", "selected consensus must be static closed committee")
require(committee["finalised"] is True, "committee finalise-demo must finalise")
require(committee["quorum_weight"] == 2, "committee quorum weight must match config")
require(len(committee["signer_ids"]) >= 2, "committee certificate must have quorum signers")
require(tendermint["consensus_kind"] == "tendermint", "selected consensus must be tendermint")
require(tendermint["finalised"] is True, "tendermint finalise-demo must finalise")
require(tendermint["quorum_weight"] == 2, "tendermint quorum power must match config")
require(len(tendermint["signer_ids"]) >= 2, "tendermint precommit certificate must have quorum signers")
print(json.dumps({
    "celltx_profile": celltx["semantic_profile"],
    "celltx_status": celltx["status"],
    "celltx_projection_possible": celltx["ckb_projection"]["ckb_projection_possible"],
    "consensus_kinds": [committee["consensus_kind"], tendermint["consensus_kind"]],
    "committee_finalised": committee["finalised"],
    "committee_signers": committee["signer_ids"],
    "tendermint_finalised": tendermint["finalised"],
    "tendermint_signers": tendermint["signer_ids"],
}, indent=2, sort_keys=True))
PY

if [[ "${RUN_TEEWORLDS}" == "1" ]]; then
  TEEWORLDS_OUTPUT_DIR="${OUTPUT_DIR}/teeworlds" \
    OUTPUT_DIR="${OUTPUT_DIR}/teeworlds" \
    run_step "Run Teeworlds acceptance gate" \
    "${SCRIPT_DIR}/myelin_teeworlds_acceptance.sh"
else
  printf '\n==> Skip Teeworlds acceptance gate because RUN_TEEWORLDS=%s\n' "${RUN_TEEWORLDS}"
fi

printf '\nMyelin protocol gate passed.\n'
printf 'Reports written under: %s\n' "${OUTPUT_DIR}"
