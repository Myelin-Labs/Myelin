#!/usr/bin/env bash
# Myelin production-readiness gate.
#
# Runs:
#   - cargo fmt --all --check
#   - git diff --check
#   - cargo check --locked --workspace --all-targets
#   - cargo clippy --locked --workspace --all-targets -- -D warnings
#   - cargo test --locked --workspace
#   - cargo test -p myelin-consensus
#   - fail-closed CellScript process-adapter tests
#   - myelin CLI smoke tests for all three consensus modes
#   - Teeworlds acceptance, if the Teeworlds repo path exists
#   - stale-surface grep
#   - forbidden parent-path audit
#
# Exits non-zero on any failure.

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
MYELIN_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
if [[ -n "${HOME:-}" ]]; then
  DEFAULT_TEEWORLDS_ROOT="${HOME}/RustroverProjects/teeworlds"
else
  DEFAULT_TEEWORLDS_ROOT="$(cd -- "${MYELIN_ROOT}/.." && pwd)/teeworlds"
fi
TEEWORLDS_ROOT="${TEEWORLDS_ROOT:-${DEFAULT_TEEWORLDS_ROOT}}"
OUTPUT_DIR="${OUTPUT_DIR:-/tmp/myelin-production-gate}"
RUN_TEEWORLDS="${RUN_TEEWORLDS:-1}"
RUN_CELLSCRIPT_TOOLCHAIN="${RUN_CELLSCRIPT_TOOLCHAIN:-1}"
RUN_CKB_DEVNET="${RUN_CKB_DEVNET:-1}"

mkdir -p "${OUTPUT_DIR}"

cd "${MYELIN_ROOT}"

# rocksdb's bindgen step needs libclang. Keep the Homebrew location scoped to
# macOS so Linux builders can discover their system libclang normally.
if [[ "$(uname -s)" == "Darwin" && -d "/opt/homebrew/opt/llvm/lib" ]]; then
  export LIBCLANG_PATH="${LIBCLANG_PATH:-/opt/homebrew/opt/llvm/lib}"
  export DYLD_LIBRARY_PATH="${DYLD_LIBRARY_PATH:-/opt/homebrew/opt/llvm/lib}"
  export DYLD_FALLBACK_LIBRARY_PATH="${DYLD_FALLBACK_LIBRARY_PATH:-/opt/homebrew/opt/llvm/lib}"
fi

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

# 4. workspace clippy
run_step "Lint Myelin workspace with Clippy" cargo clippy --locked --workspace --all-targets -- -D warnings

# 5. workspace test
run_step "Run focused Myelin protocol tests" \
  cargo test --locked --workspace \
    -p myelin-hashes \
    -p myelin-math \
    -p myelin-exec \
    -p myelin-ckb-adapter \
    -p myelin-consensus \
    -p myelin-state \
    -p myelin-mempool \
    -p myelin-session \
    -p myelin-session-network \
    -p myelin-session-runtime \
    -p myelin-session-store-rocksdb \
    -p myelin-session-escrow \
    -p myelin-wallet-auth \
    -p myelin-cli

# 5b. run full workspace tests for state and mempool
run_step "Run myelin-state tests" cargo test --locked -p myelin-state
run_step "Run myelin-mempool tests" cargo test --locked -p myelin-mempool

# 6. consensus tests
run_step "Run myelin-consensus tests" cargo test --locked -p myelin-consensus

# 7. CellScript is an independently versioned upstream toolchain. Test the
# adapter and, by default, reproduce the exact locked 0.22 patch revision and
# compile every Myelin-owned integration fixture through it.
run_step "Test CellScript process adapter" cargo test --locked -p myelin-cellscript-adapter
if [[ "${RUN_CELLSCRIPT_TOOLCHAIN}" == "1" ]]; then
  run_step "Reproduce locked CellScript 0.22 patch revision and compile Myelin fixtures" \
    "${SCRIPT_DIR}/myelin_cellscript_toolchain_gate.sh"
else
  printf '\n==> Skip locked CellScript toolchain reproduction because RUN_CELLSCRIPT_TOOLCHAIN=%s\n' \
    "${RUN_CELLSCRIPT_TOOLCHAIN}"
fi

# 7b. Exercise the exact compiled contracts on the parent CKB implementation.
# This deploys all four verifier artefacts, submits valid carrier/final-script
# transactions, and requires tamper/replay probes to be rejected by CKB.
if [[ "${RUN_CKB_DEVNET}" == "1" ]]; then
  CELLSCRIPT_TOOLCHAIN_ROOT="${CELLSCRIPT_TOOLCHAIN_ROOT:-/tmp/myelin-cellscript-toolchain-4c02e213}"
  CELLSCRIPT_BIN="${CELLSCRIPT_BIN:-${CELLSCRIPT_TOOLCHAIN_ROOT}/CellScript/target/release/cellc}"
  CELLSCRIPT_ATTESTATION="${CELLSCRIPT_ATTESTATION:-${CELLSCRIPT_TOOLCHAIN_ROOT}/myelin-evidence/compiler-attestation.json}"
  CKB_DEVNET_WORKDIR="$(mktemp -d "${OUTPUT_DIR}/ckb-devnet.XXXXXX")"
  run_step "Deploy and exercise locked CellScript artefacts on parent CKB devnet" \
    env \
      CELLSCRIPT_BIN="${CELLSCRIPT_BIN}" \
      CELLSCRIPT_ATTESTATION="${CELLSCRIPT_ATTESTATION}" \
      WORKDIR="${CKB_DEVNET_WORKDIR}" \
      REPORT="${CKB_DEVNET_WORKDIR}/report.json" \
      "${SCRIPT_DIR}/myelin_ckb_devnet_smoke.sh"
else
  printf '\n==> Skip parent CKB devnet acceptance because RUN_CKB_DEVNET=%s\n' "${RUN_CKB_DEVNET}"
fi

# 8. CLI smoke for all three consensus modes
COMMITTEE_CONFIG="${OUTPUT_DIR}/static-committee.toml"
COMMITTEE_REPORT="${OUTPUT_DIR}/static-committee.json"
cat > "${COMMITTEE_CONFIG}" <<'EOF'
kind = "static-closed-committee"

[static_committee]
quorum_weight = 2

[[static_committee.validators]]
id = "validator-0"
public_key = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
weight = 1

[[static_committee.validators]]
id = "validator-1"
public_key = "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5"
weight = 1
EOF

run_step "Smoke: myelin-cli static-closed-committee finalise" \
  cargo run -p myelin-cli -- committee finalise-demo \
    --config "${COMMITTEE_CONFIG}" \
    --out "${COMMITTEE_REPORT}"

POA_CONFIG="${OUTPUT_DIR}/proof-of-authority.toml"
POA_REPORT="${OUTPUT_DIR}/proof-of-authority.json"
cat > "${POA_CONFIG}" <<'EOF'
kind = "proof-of-authority"

[proof_of_authority]

[[proof_of_authority.authorities]]
id = "validator-0"
public_key = "0279be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"

[[proof_of_authority.authorities]]
id = "validator-1"
public_key = "02c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5"

[[proof_of_authority.authorities]]
id = "validator-2"
public_key = "02f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9"
EOF

run_step "Smoke: myelin-cli proof-of-authority finalise" \
  cargo run -p myelin-cli -- committee finalise-demo \
    --config "${POA_CONFIG}" \
    --out "${POA_REPORT}"

WEIGHTED_PRECOMMIT_CONFIG="${OUTPUT_DIR}/tendermint.toml"
WEIGHTED_PRECOMMIT_REPORT="${OUTPUT_DIR}/tendermint.json"
cat > "${WEIGHTED_PRECOMMIT_CONFIG}" <<'EOF'
kind = "tendermint"

[tendermint]
quorum_power = 3

[[tendermint.validators]]
id = "validator-0"
public_key = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
weight = 1

[[tendermint.validators]]
id = "validator-1"
public_key = "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5"
weight = 1

[[tendermint.validators]]
id = "validator-2"
public_key = "f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9"
weight = 1

[[tendermint.validators]]
id = "validator-3"
public_key = "e493dbf1c10d80f3581e4904930b1404cc6c13900ee0758474fa94abe8c4cd13"
weight = 1
EOF

run_step "Smoke: myelin-cli Tendermint finalise" \
  cargo run -p myelin-cli -- committee finalise-demo \
    --config "${WEIGHTED_PRECOMMIT_CONFIG}" \
    --out "${WEIGHTED_PRECOMMIT_REPORT}"

# 9. CLI JSON contract
run_step "Validate CLI JSON contract" \
  python3 - "${COMMITTEE_REPORT}" "${POA_REPORT}" "${WEIGHTED_PRECOMMIT_REPORT}" <<'PY'
import json
import sys
from pathlib import Path

committee = json.loads(Path(sys.argv[1]).read_text())
poa = json.loads(Path(sys.argv[2]).read_text())
weighted_precommit = json.loads(Path(sys.argv[3]).read_text())

def require(condition, message):
    if not condition:
        raise SystemExit(f"production gate failed: {message}")

require(committee["consensus_kind"] == "static-closed-committee", "static committee kind")
require(committee["finalised"] is True, "static committee finalised")
require(len(committee["signer_ids"]) >= 2, "static committee signer count")

require(poa["consensus_kind"] == "proof-of-authority", "PoA kind")
require(poa["finalised"] is True, "PoA finalised")
require(len(poa["signer_ids"]) == 1, "PoA signer count")
require(poa["certificate_step"] == "seal", "PoA seal step")
require(poa["certificate_round"] is None, "PoA has no round")

require(weighted_precommit["consensus_kind"] == "tendermint", "Tendermint kind")
require(weighted_precommit["finalised"] is True, "weighted_precommit finalised")
require(len(weighted_precommit["signer_ids"]) >= 3, "Tendermint signer count")
require(weighted_precommit["certificate_step"] == "precommit", "weighted_precommit precommit")
require(weighted_precommit["certificate_round"] == 0, "weighted_precommit round 0")
print(json.dumps({
    "static_committee": committee["block_hash"],
    "proof_of_authority": poa["block_hash"],
    "tendermint": weighted_precommit["block_hash"],
}, indent=2, sort_keys=True))
PY

# 10. Runtime smoke — exercise myelin-state + myelin-mempool + all three consensus
#     engines end-to-end through the binary, not just the unit tests.
RUNTIME_STATIC_REPORT="${OUTPUT_DIR}/runtime-smoke-static.json"
RUNTIME_POA_REPORT="${OUTPUT_DIR}/runtime-smoke-proof-of-authority.json"
RUNTIME_WEIGHTED_PRECOMMIT_REPORT="${OUTPUT_DIR}/runtime-smoke-weighted-precommit.json"

run_step "Smoke: runtime smoke (static-closed-committee)" \
  cargo run -p myelin-cli -- runtime smoke \
    --consensus static-closed-committee \
    --out "${RUNTIME_STATIC_REPORT}"

run_step "Smoke: runtime smoke (tendermint)" \
  cargo run -p myelin-cli -- runtime smoke \
    --consensus tendermint \
    --out "${RUNTIME_WEIGHTED_PRECOMMIT_REPORT}"

run_step "Smoke: runtime smoke (proof-of-authority)" \
  cargo run -p myelin-cli -- runtime smoke \
    --consensus proof-of-authority \
    --out "${RUNTIME_POA_REPORT}"

run_step "Validate runtime smoke reports" \
  python3 - "${RUNTIME_STATIC_REPORT}" "${RUNTIME_POA_REPORT}" "${RUNTIME_WEIGHTED_PRECOMMIT_REPORT}" <<'PY'
import json
import sys
from pathlib import Path

static_report = json.loads(Path(sys.argv[1]).read_text())
poa_report = json.loads(Path(sys.argv[2]).read_text())
weighted_precommit_report = json.loads(Path(sys.argv[3]).read_text())

def require(condition, message):
    if not condition:
        raise SystemExit(f"production gate failed: {message}")

# All reports must use the current schema and be finalised.
for report, kind in ((static_report, "static-closed-committee"),
                     (poa_report, "proof-of-authority"),
                     (weighted_precommit_report, "tendermint")):
    require(report["schema"] == "myelin-runtime-smoke",
            f"{kind} schema must be myelin-runtime-smoke")
    require(report["consensus_kind"] == kind, f"{kind} consensus_kind")
    require(report["vm_profile"] == "no-vm-runtime-smoke", f"{kind} vm profile")
    require(isinstance(report["ckb_spawn_ipc_enabled"], bool), f"{kind} spawn/IPC flag")
    require(report["finalised"] is True, f"{kind} finalised")
    require(len(report["cell_tx_id"]) == 64, f"{kind} txid length")
    require(len(report["cell_wtxid"]) == 64, f"{kind} wtxid length")
    require(len(report["state_root_before"]) == 64, f"{kind} state_root_before length")
    require(len(report["state_root_after"]) == 64, f"{kind} state_root_after length")
    require(len(report["certificate_hash"]) == 64, f"{kind} certificate_hash length")
    require(report["pool_size_before"] == 0, f"{kind} pool empty before")
    require(report["pool_size_after"] == 1, f"{kind} pool has 1 tx after")
    require(report["state_root_before"] != report["state_root_after"],
            f"{kind} state must mutate")

# The CellTx + state mutation is consensus-independent: txid, wtxid, and
# both state roots MUST be identical across all engines. Only the
# consensus-bound certificate_hash is allowed to differ.
require(static_report["cell_tx_id"] == poa_report["cell_tx_id"],
        "cell txid must match PoA")
require(static_report["cell_tx_id"] == weighted_precommit_report["cell_tx_id"],
        "cell txid must match across engines")
require(static_report["cell_wtxid"] == poa_report["cell_wtxid"],
        "cell wtxid must match PoA")
require(static_report["cell_wtxid"] == weighted_precommit_report["cell_wtxid"],
        "cell wtxid must match across engines")
require(static_report["state_root_before"] == poa_report["state_root_before"],
        "state_root_before must match PoA")
require(static_report["state_root_before"] == weighted_precommit_report["state_root_before"],
        "state_root_before must match across engines")
require(static_report["state_root_after"] == poa_report["state_root_after"],
        "state_root_after must match PoA")
require(static_report["state_root_after"] == weighted_precommit_report["state_root_after"],
        "state_root_after must match across engines")
require(static_report["vm_profile"] == weighted_precommit_report["vm_profile"],
        "vm profile must match across engines")
require(static_report["ckb_spawn_ipc_enabled"] == weighted_precommit_report["ckb_spawn_ipc_enabled"],
        "spawn/IPC build flag must match across engines")
require(static_report["certificate_hash"] != weighted_precommit_report["certificate_hash"],
        "static and Tendermint finality domains must differ")
require(static_report["certificate_hash"] != poa_report["certificate_hash"],
        "static and PoA finality domains must differ")
require(poa_report["certificate_hash"] != weighted_precommit_report["certificate_hash"],
        "PoA and Tendermint finality domains must differ")

print(json.dumps({
    "txid": static_report["cell_tx_id"],
    "vm_profile": static_report["vm_profile"],
    "ckb_spawn_ipc_enabled": static_report["ckb_spawn_ipc_enabled"],
    "state_root_after": static_report["state_root_after"],
    "static_certificate_hash": static_report["certificate_hash"],
    "proof_of_authority_certificate_hash": poa_report["certificate_hash"],
    "weighted_precommit_certificate_hash": weighted_precommit_report["certificate_hash"],
}, indent=2, sort_keys=True))
PY

# 11. Session L2 fixture — exercise open/commit/court/verify for both
#     consensus engines through the binary.
SESSION_OPEN_STATIC="${OUTPUT_DIR}/session-open-static.json"
SESSION_COMMIT_STATIC="${OUTPUT_DIR}/session-commit-static.json"
SESSION_COURT_STATIC="${OUTPUT_DIR}/session-court-static.json"
SESSION_VERIFY_STATIC="${OUTPUT_DIR}/session-verify-static.json"
SESSION_DA_STATIC="${OUTPUT_DIR}/session-da-static.json"
SESSION_DA_VERIFY_STATIC="${OUTPUT_DIR}/session-da-verify-static.json"
SESSION_DA_STORE_STATIC="${OUTPUT_DIR}/session-da-store-static"
SESSION_DA_ANCHOR_STATIC="${OUTPUT_DIR}/session-da-anchor-static.json"
SESSION_DA_ANCHOR_VERIFY_STATIC="${OUTPUT_DIR}/session-da-anchor-verify-static.json"
SESSION_DA_ANCHOR_SUBMIT_STATIC="${OUTPUT_DIR}/session-da-anchor-submit-static.json"
SESSION_DA_ANCHOR_INCLUSION_STATIC="${OUTPUT_DIR}/session-da-anchor-inclusion-static.json"
SESSION_DA_ANCHOR_STABILITY_STATIC="${OUTPUT_DIR}/session-da-anchor-stability-static.json"
SESSION_DA_ANCHOR_FINALITY_STATIC="${OUTPUT_DIR}/session-da-anchor-finality-static.json"
SESSION_DA_ANCHOR_CONTEXT_STATIC="${OUTPUT_DIR}/session-da-anchor-context-static.json"
SESSION_DA_ANCHOR_ECONOMICS_STATIC="${OUTPUT_DIR}/session-da-anchor-economics-static.json"
SESSION_DA_ANCHOR_READINESS_STATIC="${OUTPUT_DIR}/session-da-anchor-readiness-static.json"
SESSION_SETTLEMENT_STATIC="${OUTPUT_DIR}/session-settlement-static.json"
SESSION_SETTLEMENT_VERIFY_STATIC="${OUTPUT_DIR}/session-settlement-verify-static.json"
SESSION_PACKAGE_STATIC="${OUTPUT_DIR}/session-package-static.json"
SESSION_PACKAGE_VERIFY_STATIC="${OUTPUT_DIR}/session-package-verify-static.json"
SESSION_PACKAGE_SUBMIT_STATIC="${OUTPUT_DIR}/session-package-submit-static.json"
SESSION_PACKAGE_INCLUSION_STATIC="${OUTPUT_DIR}/session-package-inclusion-static.json"
SESSION_PACKAGE_STABILITY_STATIC="${OUTPUT_DIR}/session-package-stability-static.json"
SESSION_PACKAGE_FINALITY_STATIC="${OUTPUT_DIR}/session-package-finality-static.json"
SESSION_PACKAGE_CONTEXT_STATIC="${OUTPUT_DIR}/session-package-context-static.json"
SESSION_PACKAGE_ECONOMICS_STATIC="${OUTPUT_DIR}/session-package-economics-static.json"
SESSION_PACKAGE_READINESS_STATIC="${OUTPUT_DIR}/session-package-readiness-static.json"
SESSION_OPEN_WEIGHTED_PRECOMMIT="${OUTPUT_DIR}/session-open-weighted-precommit.json"
SESSION_COMMIT_WEIGHTED_PRECOMMIT="${OUTPUT_DIR}/session-commit-weighted-precommit.json"
SESSION_COURT_WEIGHTED_PRECOMMIT="${OUTPUT_DIR}/session-court-weighted-precommit.json"
SESSION_VERIFY_WEIGHTED_PRECOMMIT="${OUTPUT_DIR}/session-verify-weighted-precommit.json"
SESSION_DA_WEIGHTED_PRECOMMIT="${OUTPUT_DIR}/session-da-weighted-precommit.json"
SESSION_DA_VERIFY_WEIGHTED_PRECOMMIT="${OUTPUT_DIR}/session-da-verify-weighted-precommit.json"
SESSION_DA_STORE_WEIGHTED_PRECOMMIT="${OUTPUT_DIR}/session-da-store-weighted-precommit"
SESSION_DA_ANCHOR_WEIGHTED_PRECOMMIT="${OUTPUT_DIR}/session-da-anchor-weighted-precommit.json"
SESSION_DA_ANCHOR_VERIFY_WEIGHTED_PRECOMMIT="${OUTPUT_DIR}/session-da-anchor-verify-weighted-precommit.json"
SESSION_DA_ANCHOR_SUBMIT_WEIGHTED_PRECOMMIT="${OUTPUT_DIR}/session-da-anchor-submit-weighted-precommit.json"
SESSION_DA_ANCHOR_INCLUSION_WEIGHTED_PRECOMMIT="${OUTPUT_DIR}/session-da-anchor-inclusion-weighted-precommit.json"
SESSION_DA_ANCHOR_STABILITY_WEIGHTED_PRECOMMIT="${OUTPUT_DIR}/session-da-anchor-stability-weighted-precommit.json"
SESSION_DA_ANCHOR_FINALITY_WEIGHTED_PRECOMMIT="${OUTPUT_DIR}/session-da-anchor-finality-weighted-precommit.json"
SESSION_DA_ANCHOR_CONTEXT_WEIGHTED_PRECOMMIT="${OUTPUT_DIR}/session-da-anchor-context-weighted-precommit.json"
SESSION_DA_ANCHOR_ECONOMICS_WEIGHTED_PRECOMMIT="${OUTPUT_DIR}/session-da-anchor-economics-weighted-precommit.json"
SESSION_DA_ANCHOR_READINESS_WEIGHTED_PRECOMMIT="${OUTPUT_DIR}/session-da-anchor-readiness-weighted-precommit.json"
SESSION_SETTLEMENT_WEIGHTED_PRECOMMIT="${OUTPUT_DIR}/session-settlement-weighted-precommit.json"
SESSION_SETTLEMENT_VERIFY_WEIGHTED_PRECOMMIT="${OUTPUT_DIR}/session-settlement-verify-weighted-precommit.json"
SESSION_PACKAGE_WEIGHTED_PRECOMMIT="${OUTPUT_DIR}/session-package-weighted-precommit.json"
SESSION_PACKAGE_VERIFY_WEIGHTED_PRECOMMIT="${OUTPUT_DIR}/session-package-verify-weighted-precommit.json"
SESSION_PACKAGE_SUBMIT_WEIGHTED_PRECOMMIT="${OUTPUT_DIR}/session-package-submit-weighted-precommit.json"
SESSION_PACKAGE_INCLUSION_WEIGHTED_PRECOMMIT="${OUTPUT_DIR}/session-package-inclusion-weighted-precommit.json"
SESSION_PACKAGE_STABILITY_WEIGHTED_PRECOMMIT="${OUTPUT_DIR}/session-package-stability-weighted-precommit.json"
SESSION_PACKAGE_FINALITY_WEIGHTED_PRECOMMIT="${OUTPUT_DIR}/session-package-finality-weighted-precommit.json"
SESSION_PACKAGE_CONTEXT_WEIGHTED_PRECOMMIT="${OUTPUT_DIR}/session-package-context-weighted-precommit.json"
SESSION_PACKAGE_ECONOMICS_WEIGHTED_PRECOMMIT="${OUTPUT_DIR}/session-package-economics-weighted-precommit.json"
SESSION_PACKAGE_READINESS_WEIGHTED_PRECOMMIT="${OUTPUT_DIR}/session-package-readiness-weighted-precommit.json"
rm -rf "${SESSION_DA_STORE_STATIC}" "${SESSION_DA_STORE_WEIGHTED_PRECOMMIT}"

run_step "Session: open fixture (static-closed-committee)" \
  cargo run -p myelin-cli -- session open-fixture \
    --consensus static-closed-committee \
    --out "${SESSION_OPEN_STATIC}"

run_step "Session: commit fixture (static-closed-committee)" \
  cargo run -p myelin-cli -- session commit-fixture \
    --session "${SESSION_OPEN_STATIC}" \
    --out "${SESSION_COMMIT_STATIC}"

run_step "Session: court bundle (static-closed-committee)" \
  cargo run -p myelin-cli -- session court-bundle \
    --commit "${SESSION_COMMIT_STATIC}" \
    --chunk-index 0 \
    --out "${SESSION_COURT_STATIC}"

run_step "Session: verify court bundle (static-closed-committee)" \
  cargo run -p myelin-cli -- session verify-court-bundle \
    --bundle "${SESSION_COURT_STATIC}" \
    --out "${SESSION_VERIFY_STATIC}"

run_step "Session: DA manifest (static-closed-committee)" \
  cargo run -p myelin-cli -- session da-manifest \
    --bundle "${SESSION_COURT_STATIC}" \
    --storage-dir "${SESSION_DA_STORE_STATIC}" \
    --out "${SESSION_DA_STATIC}"

run_step "Session: verify DA manifest (static-closed-committee)" \
  cargo run -p myelin-cli -- session verify-da-manifest \
    --manifest "${SESSION_DA_STATIC}" \
    --bundle "${SESSION_COURT_STATIC}" \
    --storage-dir "${SESSION_DA_STORE_STATIC}" \
    --out "${SESSION_DA_VERIFY_STATIC}"

run_step "Session: DA anchor package (static-closed-committee)" \
  cargo run -p myelin-cli -- session da-anchor-package \
    --manifest "${SESSION_DA_STATIC}" \
    --bundle "${SESSION_COURT_STATIC}" \
    --out "${SESSION_DA_ANCHOR_STATIC}"

run_step "Session: verify DA anchor package (static-closed-committee)" \
  cargo run -p myelin-cli -- session verify-da-anchor-package \
    --package "${SESSION_DA_ANCHOR_STATIC}" \
    --manifest "${SESSION_DA_STATIC}" \
    --bundle "${SESSION_COURT_STATIC}" \
    --out "${SESSION_DA_ANCHOR_VERIFY_STATIC}"

run_step "Session: dry-run DA anchor RPC submission (static-closed-committee)" \
  cargo run -p myelin-cli -- session submit-da-anchor-package \
    --package "${SESSION_DA_ANCHOR_STATIC}" \
    --dry-run \
    --out "${SESSION_DA_ANCHOR_SUBMIT_STATIC}"

run_step "Session: settlement intent (static-closed-committee)" \
  cargo run -p myelin-cli -- session settlement-intent \
    --bundle "${SESSION_COURT_STATIC}" \
    --da-manifest "${SESSION_DA_STATIC}" \
    --kind disputed-close \
    --current-time-ms 60000 \
    --challenge-window-ms 60000 \
    --out "${SESSION_SETTLEMENT_STATIC}"

run_step "Session: verify settlement intent (static-closed-committee)" \
  cargo run -p myelin-cli -- session verify-settlement-intent \
    --intent "${SESSION_SETTLEMENT_STATIC}" \
    --bundle "${SESSION_COURT_STATIC}" \
    --da-manifest "${SESSION_DA_STATIC}" \
    --out "${SESSION_SETTLEMENT_VERIFY_STATIC}"

run_step "Session: settlement package (static-closed-committee)" \
  cargo run -p myelin-cli -- session settlement-package \
    --intent "${SESSION_SETTLEMENT_STATIC}" \
    --bundle "${SESSION_COURT_STATIC}" \
    --da-manifest "${SESSION_DA_STATIC}" \
    --out "${SESSION_PACKAGE_STATIC}"

run_step "Session: verify settlement package (static-closed-committee)" \
  cargo run -p myelin-cli -- session verify-settlement-package \
    --package "${SESSION_PACKAGE_STATIC}" \
    --intent "${SESSION_SETTLEMENT_STATIC}" \
    --bundle "${SESSION_COURT_STATIC}" \
    --da-manifest "${SESSION_DA_STATIC}" \
    --out "${SESSION_PACKAGE_VERIFY_STATIC}"

run_step "Session: dry-run settlement RPC submission (static-closed-committee)" \
  cargo run -p myelin-cli -- session submit-settlement-package \
    --package "${SESSION_PACKAGE_STATIC}" \
    --dry-run \
    --out "${SESSION_PACKAGE_SUBMIT_STATIC}"

run_step "Session: open fixture (weighted-precommit)" \
  cargo run -p myelin-cli -- session open-fixture \
    --consensus tendermint \
    --out "${SESSION_OPEN_WEIGHTED_PRECOMMIT}"

run_step "Session: commit fixture (weighted-precommit)" \
  cargo run -p myelin-cli -- session commit-fixture \
    --session "${SESSION_OPEN_WEIGHTED_PRECOMMIT}" \
    --out "${SESSION_COMMIT_WEIGHTED_PRECOMMIT}"

run_step "Session: court bundle (weighted-precommit)" \
  cargo run -p myelin-cli -- session court-bundle \
    --commit "${SESSION_COMMIT_WEIGHTED_PRECOMMIT}" \
    --chunk-index 0 \
    --out "${SESSION_COURT_WEIGHTED_PRECOMMIT}"

run_step "Session: verify court bundle (weighted-precommit)" \
  cargo run -p myelin-cli -- session verify-court-bundle \
    --bundle "${SESSION_COURT_WEIGHTED_PRECOMMIT}" \
    --out "${SESSION_VERIFY_WEIGHTED_PRECOMMIT}"

run_step "Session: DA manifest (weighted-precommit)" \
  cargo run -p myelin-cli -- session da-manifest \
    --bundle "${SESSION_COURT_WEIGHTED_PRECOMMIT}" \
    --storage-dir "${SESSION_DA_STORE_WEIGHTED_PRECOMMIT}" \
    --out "${SESSION_DA_WEIGHTED_PRECOMMIT}"

run_step "Session: verify DA manifest (weighted-precommit)" \
  cargo run -p myelin-cli -- session verify-da-manifest \
    --manifest "${SESSION_DA_WEIGHTED_PRECOMMIT}" \
    --bundle "${SESSION_COURT_WEIGHTED_PRECOMMIT}" \
    --storage-dir "${SESSION_DA_STORE_WEIGHTED_PRECOMMIT}" \
    --out "${SESSION_DA_VERIFY_WEIGHTED_PRECOMMIT}"

run_step "Session: DA anchor package (weighted-precommit)" \
  cargo run -p myelin-cli -- session da-anchor-package \
    --manifest "${SESSION_DA_WEIGHTED_PRECOMMIT}" \
    --bundle "${SESSION_COURT_WEIGHTED_PRECOMMIT}" \
    --out "${SESSION_DA_ANCHOR_WEIGHTED_PRECOMMIT}"

run_step "Session: verify DA anchor package (weighted-precommit)" \
  cargo run -p myelin-cli -- session verify-da-anchor-package \
    --package "${SESSION_DA_ANCHOR_WEIGHTED_PRECOMMIT}" \
    --manifest "${SESSION_DA_WEIGHTED_PRECOMMIT}" \
    --bundle "${SESSION_COURT_WEIGHTED_PRECOMMIT}" \
    --out "${SESSION_DA_ANCHOR_VERIFY_WEIGHTED_PRECOMMIT}"

run_step "Session: dry-run DA anchor RPC submission (weighted-precommit)" \
  cargo run -p myelin-cli -- session submit-da-anchor-package \
    --package "${SESSION_DA_ANCHOR_WEIGHTED_PRECOMMIT}" \
    --dry-run \
    --out "${SESSION_DA_ANCHOR_SUBMIT_WEIGHTED_PRECOMMIT}"

run_step "Session: settlement intent (weighted-precommit)" \
  cargo run -p myelin-cli -- session settlement-intent \
    --bundle "${SESSION_COURT_WEIGHTED_PRECOMMIT}" \
    --da-manifest "${SESSION_DA_WEIGHTED_PRECOMMIT}" \
    --kind disputed-close \
    --current-time-ms 60000 \
    --challenge-window-ms 60000 \
    --out "${SESSION_SETTLEMENT_WEIGHTED_PRECOMMIT}"

run_step "Session: verify settlement intent (weighted-precommit)" \
  cargo run -p myelin-cli -- session verify-settlement-intent \
    --intent "${SESSION_SETTLEMENT_WEIGHTED_PRECOMMIT}" \
    --bundle "${SESSION_COURT_WEIGHTED_PRECOMMIT}" \
    --da-manifest "${SESSION_DA_WEIGHTED_PRECOMMIT}" \
    --out "${SESSION_SETTLEMENT_VERIFY_WEIGHTED_PRECOMMIT}"

run_step "Session: settlement package (weighted-precommit)" \
  cargo run -p myelin-cli -- session settlement-package \
    --intent "${SESSION_SETTLEMENT_WEIGHTED_PRECOMMIT}" \
    --bundle "${SESSION_COURT_WEIGHTED_PRECOMMIT}" \
    --da-manifest "${SESSION_DA_WEIGHTED_PRECOMMIT}" \
    --out "${SESSION_PACKAGE_WEIGHTED_PRECOMMIT}"

run_step "Session: verify settlement package (weighted-precommit)" \
  cargo run -p myelin-cli -- session verify-settlement-package \
    --package "${SESSION_PACKAGE_WEIGHTED_PRECOMMIT}" \
    --intent "${SESSION_SETTLEMENT_WEIGHTED_PRECOMMIT}" \
    --bundle "${SESSION_COURT_WEIGHTED_PRECOMMIT}" \
    --da-manifest "${SESSION_DA_WEIGHTED_PRECOMMIT}" \
    --out "${SESSION_PACKAGE_VERIFY_WEIGHTED_PRECOMMIT}"

run_step "Session: dry-run settlement RPC submission (weighted-precommit)" \
  cargo run -p myelin-cli -- session submit-settlement-package \
    --package "${SESSION_PACKAGE_WEIGHTED_PRECOMMIT}" \
    --dry-run \
    --out "${SESSION_PACKAGE_SUBMIT_WEIGHTED_PRECOMMIT}"

# PoA is exercised through the consensus-sensitive Session spine. The
# downstream DA/settlement package shapes are already exercised above for both
# quorum-based engines and are consensus-neutral after court verification.
SESSION_OPEN_POA="${OUTPUT_DIR}/session-open-proof-of-authority.json"
SESSION_COMMIT_POA="${OUTPUT_DIR}/session-commit-proof-of-authority.json"
SESSION_COURT_POA="${OUTPUT_DIR}/session-court-proof-of-authority.json"
SESSION_VERIFY_POA="${OUTPUT_DIR}/session-verify-proof-of-authority.json"

run_step "Session: open fixture (proof-of-authority)" \
  cargo run -p myelin-cli -- session open-fixture \
    --consensus proof-of-authority \
    --out "${SESSION_OPEN_POA}"

run_step "Session: commit fixture (proof-of-authority)" \
  cargo run -p myelin-cli -- session commit-fixture \
    --session "${SESSION_OPEN_POA}" \
    --out "${SESSION_COMMIT_POA}"

run_step "Session: court bundle (proof-of-authority)" \
  cargo run -p myelin-cli -- session court-bundle \
    --commit "${SESSION_COMMIT_POA}" \
    --chunk-index 0 \
    --out "${SESSION_COURT_POA}"

run_step "Session: verify court bundle (proof-of-authority)" \
  cargo run -p myelin-cli -- session verify-court-bundle \
    --bundle "${SESSION_COURT_POA}" \
    --out "${SESSION_VERIFY_POA}"

run_step "Validate proof-of-authority Session finality evidence" \
  python3 - "${SESSION_COMMIT_POA}" "${SESSION_COURT_POA}" "${SESSION_VERIFY_POA}" <<'PY'
import json
import sys
from pathlib import Path

commit, court, verification = [json.loads(Path(path).read_text()) for path in sys.argv[1:]]

def require(condition, message):
    if not condition:
        raise SystemExit(f"production gate failed: {message}")

for report, label in ((commit, "commit"), (court, "court")):
    require(report["consensus_kind"] == "proof-of-authority", f"PoA {label} consensus kind")
    require(report["static_committee_evidence"]["finalised"] is False, f"PoA {label} static evidence inactive")
    require(report["weighted_precommit_evidence"] is None, f"PoA {label} Tendermint evidence absent")
    proof = report["proof_of_authority_evidence"]
    require(proof is not None, f"PoA {label} seal present")
    require(proof["consensus_kind"] == "proof-of-authority", f"PoA {label} proof kind")
    require(proof["certificate_step"] == "seal", f"PoA {label} proof step")
    require(proof["height"] == 1, f"PoA {label} height")
    require(proof["authority_id"] == "validator-1", f"PoA {label} scheduled authority")
    require(proof["finalised"] is True, f"PoA {label} finalised")

require(commit["block"]["block_hash"] == court["block"]["block_hash"], "PoA court preserves canonical block")
require(verification["valid"] is True, "PoA court verification")
require(all(check["ok"] for check in verification["checks"]), "PoA court checks")
require(any(check["name"] == "proof-of-authority-seal" for check in verification["checks"]), "PoA seal checked")
PY

run_step "Session: mock CKB inclusion verification" \
  python3 - \
    "${SESSION_DA_ANCHOR_SUBMIT_STATIC}" "${SESSION_DA_ANCHOR_INCLUSION_STATIC}" \
    "${SESSION_DA_ANCHOR_SUBMIT_WEIGHTED_PRECOMMIT}" "${SESSION_DA_ANCHOR_INCLUSION_WEIGHTED_PRECOMMIT}" \
    "${SESSION_PACKAGE_SUBMIT_STATIC}" "${SESSION_PACKAGE_INCLUSION_STATIC}" \
    "${SESSION_PACKAGE_SUBMIT_WEIGHTED_PRECOMMIT}" "${SESSION_PACKAGE_INCLUSION_WEIGHTED_PRECOMMIT}" <<'PY'
import json
import subprocess
import sys
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

pairs = [(Path(sys.argv[i]), Path(sys.argv[i + 1])) for i in range(1, len(sys.argv), 2)]
expected_hashes = set()

def normalise(hash_value):
    text = str(hash_value)
    if text.startswith("0x"):
        text = text[2:]
    if len(text) != 64:
        raise SystemExit(f"production gate failed: malformed CKB tx hash {hash_value}")
    return "0x" + text

for submission_path, _ in pairs:
    submission = json.loads(submission_path.read_text())
    expected_hashes.add(normalise(submission["ckb_raw_tx_hash"]))

class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length)
        request = json.loads(body)
        if request.get("method") != "get_transaction":
            self.send_error(400, "expected get_transaction")
            return
        tx_hash = request.get("params", [None])[0]
        if tx_hash not in expected_hashes:
            self.send_error(400, f"unexpected tx hash {tx_hash}")
            return
        response = {
            "jsonrpc": "2.0",
            "id": request.get("id"),
            "result": {
                "transaction": {},
                "tx_status": {
                    "status": "committed",
                    "block_hash": "0x" + "ab" * 32,
                    "block_number": "0x64",
                },
            },
        }
        encoded = json.dumps(response).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def log_message(self, *_):
        return

server = HTTPServer(("127.0.0.1", 0), Handler)
thread = threading.Thread(target=server.serve_forever, daemon=True)
thread.start()
url = f"http://127.0.0.1:{server.server_port}"
try:
    for submission_path, out_path in pairs:
        subprocess.run(
            [
                "cargo",
                "run",
                "-p",
                "myelin-cli",
                "--",
                "session",
                "verify-submission-inclusion",
                "--submission",
                str(submission_path),
                "--rpc-url",
                url,
                "--min-status",
                "committed",
                "--out",
                str(out_path),
            ],
            check=True,
        )
        report = json.loads(out_path.read_text())
        if report["schema"] != "myelin-session-submission-inclusion":
            raise SystemExit("production gate failed: inclusion schema")
        if report["request_method"] != "get_transaction":
            raise SystemExit("production gate failed: inclusion method")
        if report["status"] != "committed":
            raise SystemExit("production gate failed: inclusion committed status")
        if report["met_min_status"] is not True or report["live_l1_observed"] is not True:
            raise SystemExit("production gate failed: inclusion live marker")
        if report["block_number"] != "0x64":
            raise SystemExit("production gate failed: inclusion block number")
finally:
    server.shutdown()
    thread.join(timeout=5)
PY

run_step "Session: mock CKB committed-block stability verification" \
  python3 - \
    "${SESSION_DA_ANCHOR_INCLUSION_STATIC}" "${SESSION_DA_ANCHOR_STABILITY_STATIC}" \
    "${SESSION_DA_ANCHOR_INCLUSION_WEIGHTED_PRECOMMIT}" "${SESSION_DA_ANCHOR_STABILITY_WEIGHTED_PRECOMMIT}" \
    "${SESSION_PACKAGE_INCLUSION_STATIC}" "${SESSION_PACKAGE_STABILITY_STATIC}" \
    "${SESSION_PACKAGE_INCLUSION_WEIGHTED_PRECOMMIT}" "${SESSION_PACKAGE_STABILITY_WEIGHTED_PRECOMMIT}" <<'PY'
import json
import subprocess
import sys
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

pairs = [(Path(sys.argv[i]), Path(sys.argv[i + 1])) for i in range(1, len(sys.argv), 2)]
expected_hashes = set()

for inclusion_path, _ in pairs:
    inclusion = json.loads(inclusion_path.read_text())
    expected_hashes.add(inclusion["expected_ckb_tx_hash"])

class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length)
        request = json.loads(body)
        if request.get("method") != "get_transaction":
            self.send_error(400, "expected get_transaction")
            return
        tx_hash = request.get("params", [None])[0]
        if tx_hash not in expected_hashes:
            self.send_error(400, f"unexpected tx hash {tx_hash}")
            return
        response = {
            "jsonrpc": "2.0",
            "id": request.get("id"),
            "result": {
                "transaction": {},
                "tx_status": {
                    "status": "committed",
                    "block_hash": "0x" + "ab" * 32,
                    "block_number": "0x64",
                },
            },
        }
        encoded = json.dumps(response).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def log_message(self, *_):
        return

server = HTTPServer(("127.0.0.1", 0), Handler)
thread = threading.Thread(target=server.serve_forever, daemon=True)
thread.start()
url = f"http://127.0.0.1:{server.server_port}"
try:
    for inclusion_path, out_path in pairs:
        subprocess.run(
            [
                "cargo",
                "run",
                "-p",
                "myelin-cli",
                "--",
                "session",
                "verify-submission-stability",
                "--inclusion",
                str(inclusion_path),
                "--rpc-url",
                url,
                "--out",
                str(out_path),
            ],
            check=True,
        )
        report = json.loads(out_path.read_text())
        if report["schema"] != "myelin-session-submission-stability":
            raise SystemExit("production gate failed: stability schema")
        if report["request_method"] != "get_transaction":
            raise SystemExit("production gate failed: stability method")
        if report["expected_block_hash"] != "0x" + "ab" * 32 or report["observed_block_hash"] != "0x" + "ab" * 32:
            raise SystemExit("production gate failed: stability block hash")
        if report["expected_block_number"] != "0x64" or report["observed_block_number"] != "0x64":
            raise SystemExit("production gate failed: stability block number")
        if report["stable_block_identity"] is not True:
            raise SystemExit("production gate failed: stability marker")
        if report["reorg_detected"] is not False or report["missing_or_uncommitted"] is not False:
            raise SystemExit("production gate failed: stability false positive")
finally:
    server.shutdown()
    thread.join(timeout=5)
PY

run_step "Session: mock CKB finality-depth verification" \
  python3 - \
    "${SESSION_DA_ANCHOR_INCLUSION_STATIC}" "${SESSION_DA_ANCHOR_FINALITY_STATIC}" \
    "${SESSION_DA_ANCHOR_INCLUSION_WEIGHTED_PRECOMMIT}" "${SESSION_DA_ANCHOR_FINALITY_WEIGHTED_PRECOMMIT}" \
    "${SESSION_PACKAGE_INCLUSION_STATIC}" "${SESSION_PACKAGE_FINALITY_STATIC}" \
    "${SESSION_PACKAGE_INCLUSION_WEIGHTED_PRECOMMIT}" "${SESSION_PACKAGE_FINALITY_WEIGHTED_PRECOMMIT}" <<'PY'
import json
import subprocess
import sys
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

pairs = [(Path(sys.argv[i]), Path(sys.argv[i + 1])) for i in range(1, len(sys.argv), 2)]

class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length)
        request = json.loads(body)
        if request.get("method") != "get_tip_header":
            self.send_error(400, "expected get_tip_header")
            return
        response = {
            "jsonrpc": "2.0",
            "id": request.get("id"),
            "result": {
                "number": "0x6a",
            },
        }
        encoded = json.dumps(response).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def log_message(self, *_):
        return

server = HTTPServer(("127.0.0.1", 0), Handler)
thread = threading.Thread(target=server.serve_forever, daemon=True)
thread.start()
url = f"http://127.0.0.1:{server.server_port}"
try:
    for inclusion_path, out_path in pairs:
        subprocess.run(
            [
                "cargo",
                "run",
                "-p",
                "myelin-cli",
                "--",
                "session",
                "verify-submission-finality",
                "--inclusion",
                str(inclusion_path),
                "--rpc-url",
                url,
                "--min-confirmations",
                "6",
                "--out",
                str(out_path),
            ],
            check=True,
        )
        report = json.loads(out_path.read_text())
        if report["schema"] != "myelin-session-submission-finality":
            raise SystemExit("production gate failed: finality schema")
        if report["request_method"] != "get_tip_header":
            raise SystemExit("production gate failed: finality method")
        if report["committed_block_number"] != "0x64" or report["tip_block_number"] != "0x6a":
            raise SystemExit("production gate failed: finality block numbers")
        if report["confirmations"] != 7:
            raise SystemExit("production gate failed: finality confirmation count")
        if report["min_confirmations"] != 6:
            raise SystemExit("production gate failed: finality policy")
        if report["finality_confirmed"] is not True or report["reorg_risk_bounded"] is not True:
            raise SystemExit("production gate failed: finality marker")
finally:
    server.shutdown()
    thread.join(timeout=5)
PY

run_step "Session: mock CKB live-cell context preflight" \
  python3 - \
    "${SESSION_DA_ANCHOR_SUBMIT_STATIC}" "${SESSION_DA_ANCHOR_CONTEXT_STATIC}" \
    "${SESSION_DA_ANCHOR_SUBMIT_WEIGHTED_PRECOMMIT}" "${SESSION_DA_ANCHOR_CONTEXT_WEIGHTED_PRECOMMIT}" \
    "${SESSION_PACKAGE_SUBMIT_STATIC}" "${SESSION_PACKAGE_CONTEXT_STATIC}" \
    "${SESSION_PACKAGE_SUBMIT_WEIGHTED_PRECOMMIT}" "${SESSION_PACKAGE_CONTEXT_WEIGHTED_PRECOMMIT}" <<'PY'
import json
import subprocess
import sys
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

pairs = [(Path(sys.argv[i]), Path(sys.argv[i + 1])) for i in range(1, len(sys.argv), 2)]
live_outpoints = set()

def normalise_hash(hash_value):
    text = str(hash_value)
    if text.startswith("0x"):
        text = text[2:]
    if len(text) != 64:
        raise SystemExit(f"production gate failed: malformed CKB outpoint hash {hash_value}")
    return "0x" + text

def outpoint_key(out_point):
    return f"{normalise_hash(out_point['tx_hash'])}:{out_point['index']}"

for submission_path, _ in pairs:
    submission = json.loads(submission_path.read_text())
    tx = submission["ckb_transaction_json"]
    for input_cell in tx["inputs"]:
        live_outpoints.add(outpoint_key(input_cell["previous_output"]))
    for cell_dep in tx["cell_deps"]:
        live_outpoints.add(outpoint_key(cell_dep["out_point"]))

class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length)
        request = json.loads(body)
        if request.get("method") != "get_live_cell":
            self.send_error(400, "expected get_live_cell")
            return
        out_point = request.get("params", [None])[0]
        key = outpoint_key(out_point)
        if key not in live_outpoints:
            result = {"cell": None, "status": "unknown"}
        else:
            result = {
                "cell": {
                    "output": {
                        "capacity": "0x3e8",
                        "lock": {
                            "code_hash": "0x" + "00" * 32,
                            "hash_type": "type",
                            "args": "0x",
                        },
                        "type": None,
                    },
                    "data": {
                        "content": "0x",
                        "hash": "0x" + "00" * 32,
                    },
                },
                "status": "live",
            }
        response = {"jsonrpc": "2.0", "id": request.get("id"), "result": result}
        encoded = json.dumps(response).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def log_message(self, *_):
        return

server = HTTPServer(("127.0.0.1", 0), Handler)
thread = threading.Thread(target=server.serve_forever, daemon=True)
thread.start()
url = f"http://127.0.0.1:{server.server_port}"
try:
    for submission_path, out_path in pairs:
        subprocess.run(
            [
                "cargo",
                "run",
                "-p",
                "myelin-cli",
                "--",
                "session",
                "verify-submission-context",
                "--submission",
                str(submission_path),
                "--rpc-url",
                url,
                "--out",
                str(out_path),
            ],
            check=True,
        )
        report = json.loads(out_path.read_text())
        if report["schema"] != "myelin-session-submission-context":
            raise SystemExit("production gate failed: context schema")
        if report["request_method"] != "get_live_cell":
            raise SystemExit("production gate failed: context method")
        if report["ready_for_ckb_submission"] is not True:
            raise SystemExit("production gate failed: context not ready")
        if report["all_inputs_live"] is not True or report["all_cell_deps_live"] is not True:
            raise SystemExit("production gate failed: context live markers")
finally:
    server.shutdown()
    thread.join(timeout=5)
PY

run_step "Session: mock CKB capacity and fee preflight" \
  python3 - \
    "${SESSION_DA_ANCHOR_SUBMIT_STATIC}" "${SESSION_DA_ANCHOR_ECONOMICS_STATIC}" \
    "${SESSION_DA_ANCHOR_SUBMIT_WEIGHTED_PRECOMMIT}" "${SESSION_DA_ANCHOR_ECONOMICS_WEIGHTED_PRECOMMIT}" \
    "${SESSION_PACKAGE_SUBMIT_STATIC}" "${SESSION_PACKAGE_ECONOMICS_STATIC}" \
    "${SESSION_PACKAGE_SUBMIT_WEIGHTED_PRECOMMIT}" "${SESSION_PACKAGE_ECONOMICS_WEIGHTED_PRECOMMIT}" <<'PY'
import json
import subprocess
import sys
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

pairs = [(Path(sys.argv[i]), Path(sys.argv[i + 1])) for i in range(1, len(sys.argv), 2)]
live_input_capacities = {}
fee_rate = 1000

def normalise_hash(hash_value):
    text = str(hash_value)
    if text.startswith("0x"):
        text = text[2:]
    if len(text) != 64:
        raise SystemExit(f"production gate failed: malformed CKB economics hash {hash_value}")
    return "0x" + text

def parse_quantity(value):
    text = str(value)
    if not text.startswith("0x"):
        raise SystemExit(f"production gate failed: malformed CKB quantity {value}")
    return int(text[2:], 16)

def outpoint_key(out_point):
    return f"{normalise_hash(out_point['tx_hash'])}:{out_point['index']}"

def required_fee_for_rate(transaction_size_bytes, rate):
    return (transaction_size_bytes * rate + 999) // 1000

for submission_path, _ in pairs:
    submission = json.loads(submission_path.read_text())
    tx = submission["ckb_transaction_json"]
    transaction_size = len(json.dumps(tx, separators=(",", ":")).encode())
    output_capacity = sum(parse_quantity(output["capacity"]) for output in tx["outputs"])
    required_fee = max(1, required_fee_for_rate(transaction_size, fee_rate))
    exact_input_capacity = output_capacity + required_fee
    for input_cell in tx["inputs"]:
        live_input_capacities[outpoint_key(input_cell["previous_output"])] = exact_input_capacity

class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length)
        request = json.loads(body)
        if request.get("method") != "get_live_cell":
            self.send_error(400, "expected get_live_cell")
            return
        out_point = request.get("params", [None])[0]
        key = outpoint_key(out_point)
        capacity = live_input_capacities.get(key)
        if capacity is None:
            result = {"cell": None, "status": "unknown"}
        else:
            result = {
                "cell": {
                    "output": {
                        "capacity": hex(capacity),
                        "lock": {
                            "code_hash": "0x" + "00" * 32,
                            "hash_type": "type",
                            "args": "0x",
                        },
                        "type": None,
                    },
                    "data": {
                        "content": "0x",
                        "hash": "0x" + "00" * 32,
                    },
                },
                "status": "live",
            }
        response = {"jsonrpc": "2.0", "id": request.get("id"), "result": result}
        encoded = json.dumps(response).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def log_message(self, *_):
        return

server = HTTPServer(("127.0.0.1", 0), Handler)
thread = threading.Thread(target=server.serve_forever, daemon=True)
thread.start()
url = f"http://127.0.0.1:{server.server_port}"
try:
    for submission_path, out_path in pairs:
        submission = json.loads(submission_path.read_text())
        tx = submission["ckb_transaction_json"]
        transaction_size = len(json.dumps(tx, separators=(",", ":")).encode())
        output_capacity = sum(parse_quantity(output["capacity"]) for output in tx["outputs"])
        expected_fee = max(1, required_fee_for_rate(transaction_size, fee_rate))
        expected_input_capacity = output_capacity + expected_fee
        if expected_fee < 1:
            raise SystemExit("production gate failed: economics fixture has no positive fee")
        subprocess.run(
            [
                "cargo",
                "run",
                "-p",
                "myelin-cli",
                "--",
                "session",
                "verify-submission-economics",
                "--submission",
                str(submission_path),
                "--rpc-url",
                url,
                "--min-fee-shannons",
                "1",
                "--min-fee-rate-shannons-per-kb",
                "1000",
                "--max-fee-shannons",
                str(expected_fee),
                "--out",
                str(out_path),
            ],
            check=True,
        )
        report = json.loads(out_path.read_text())
        if report["schema"] != "myelin-session-submission-economics":
            raise SystemExit("production gate failed: economics schema")
        if report["request_method"] != "get_live_cell":
            raise SystemExit("production gate failed: economics method")
        if report["total_input_capacity"] != expected_input_capacity:
            raise SystemExit("production gate failed: economics input capacity")
        if report["total_output_capacity"] != output_capacity:
            raise SystemExit("production gate failed: economics output capacity")
        if report["fee_shannons"] != expected_fee or report["fee_sufficient"] is not True:
            raise SystemExit("production gate failed: economics fee")
        if report["transaction_size_bytes"] <= 0:
            raise SystemExit("production gate failed: economics transaction size")
        if report["min_fee_rate_shannons_per_kb"] != 1000:
            raise SystemExit("production gate failed: economics fee rate policy")
        if report["required_fee_shannons"] > expected_fee or report["fee_rate_sufficient"] is not True:
            raise SystemExit("production gate failed: economics fee rate")
        if report["max_fee_shannons"] != expected_fee or report["fee_not_excessive"] is not True:
            raise SystemExit("production gate failed: economics max fee")
        if report["economically_ready"] is not True:
            raise SystemExit("production gate failed: economics readiness")
finally:
    server.shutdown()
    thread.join(timeout=5)
PY

run_step "Session: aggregate production submission readiness" \
  python3 - \
    "${SESSION_DA_ANCHOR_CONTEXT_STATIC}" "${SESSION_DA_ANCHOR_ECONOMICS_STATIC}" "${SESSION_DA_ANCHOR_INCLUSION_STATIC}" "${SESSION_DA_ANCHOR_STABILITY_STATIC}" "${SESSION_DA_ANCHOR_FINALITY_STATIC}" "${SESSION_DA_ANCHOR_READINESS_STATIC}" \
    "${SESSION_DA_ANCHOR_CONTEXT_WEIGHTED_PRECOMMIT}" "${SESSION_DA_ANCHOR_ECONOMICS_WEIGHTED_PRECOMMIT}" "${SESSION_DA_ANCHOR_INCLUSION_WEIGHTED_PRECOMMIT}" "${SESSION_DA_ANCHOR_STABILITY_WEIGHTED_PRECOMMIT}" "${SESSION_DA_ANCHOR_FINALITY_WEIGHTED_PRECOMMIT}" "${SESSION_DA_ANCHOR_READINESS_WEIGHTED_PRECOMMIT}" \
    "${SESSION_PACKAGE_CONTEXT_STATIC}" "${SESSION_PACKAGE_ECONOMICS_STATIC}" "${SESSION_PACKAGE_INCLUSION_STATIC}" "${SESSION_PACKAGE_STABILITY_STATIC}" "${SESSION_PACKAGE_FINALITY_STATIC}" "${SESSION_PACKAGE_READINESS_STATIC}" \
    "${SESSION_PACKAGE_CONTEXT_WEIGHTED_PRECOMMIT}" "${SESSION_PACKAGE_ECONOMICS_WEIGHTED_PRECOMMIT}" "${SESSION_PACKAGE_INCLUSION_WEIGHTED_PRECOMMIT}" "${SESSION_PACKAGE_STABILITY_WEIGHTED_PRECOMMIT}" "${SESSION_PACKAGE_FINALITY_WEIGHTED_PRECOMMIT}" "${SESSION_PACKAGE_READINESS_WEIGHTED_PRECOMMIT}" <<'PY'
import json
import subprocess
import sys
from pathlib import Path

groups = [(Path(sys.argv[i]), Path(sys.argv[i + 1]), Path(sys.argv[i + 2]), Path(sys.argv[i + 3]), Path(sys.argv[i + 4]), Path(sys.argv[i + 5]))
          for i in range(1, len(sys.argv), 6)]

for context_path, economics_path, inclusion_path, stability_path, finality_path, out_path in groups:
    subprocess.run(
        [
            "cargo",
            "run",
            "-p",
            "myelin-cli",
            "--",
            "session",
            "verify-submission-readiness",
            "--context",
            str(context_path),
            "--economics",
            str(economics_path),
            "--inclusion",
            str(inclusion_path),
            "--stability",
            str(stability_path),
            "--finality",
            str(finality_path),
            "--out",
            str(out_path),
        ],
        check=True,
    )
    report = json.loads(out_path.read_text())
    if report["schema"] != "myelin-session-submission-readiness":
        raise SystemExit("production gate failed: readiness schema")
    if report["report_hashes_match"] is not True:
        raise SystemExit("production gate failed: readiness hash binding")
    if report["block_identity_matches"] is not True:
        raise SystemExit("production gate failed: readiness block identity")
    for key in (
        "context_ready",
        "economically_ready",
        "inclusion_observed",
        "stable_block_identity",
        "finality_confirmed",
        "reorg_risk_bounded",
        "production_submission_ready",
    ):
        if report[key] is not True:
            raise SystemExit(f"production gate failed: readiness marker {key}")
    if not report["checks"] or not all(check["ok"] for check in report["checks"]):
        raise SystemExit("production gate failed: readiness checks")
    if report.get("readiness_evidence_mode") != "coherent-offline-or-mock":
        raise SystemExit("production gate failed: readiness evidence mode")
    if report.get("strict_production_submission_ready") is not False:
        raise SystemExit("production gate failed: dry-run readiness cannot be strict production readiness")
    if report.get("end_to_end_production_ready") is not False:
        raise SystemExit("production gate failed: dry-run readiness cannot claim end-to-end production readiness")
    end_to_end_blockers = set(report.get("end_to_end_production_blockers", []))
    if "final-l1-script-submission-not-ready" not in end_to_end_blockers:
        raise SystemExit("production gate failed: dry-run readiness must expose final L1 production blocker")
    if "operator-custody-policy-missing" not in end_to_end_blockers or "operator-runbook-missing" not in end_to_end_blockers:
        raise SystemExit("production gate failed: dry-run readiness must expose operational production blockers")
    if report.get("live_carrier_submission_ready") is not False:
        raise SystemExit("production gate failed: readiness live carrier marker")
    if report.get("final_l1_script_submission_ready") is not False:
        raise SystemExit("production gate failed: readiness final L1 marker")
    policy = report["operational_policy"]
    if policy["schema"] != "myelin-public-chain-operational-policy":
        raise SystemExit("production gate failed: operational policy schema")
    if policy["mode"] != "ckb-public-chain-testnet-beta":
        raise SystemExit("production gate failed: operational policy mode")
    for key in ("reorg_policy_checked", "fee_policy_checked", "retry_policy_checked", "monitoring_policy_checked", "testnet_beta_ready"):
        if policy[key] is not True:
            raise SystemExit(f"production gate failed: operational policy marker {key}")
    if policy["key_policy_checked"] is not False:
        raise SystemExit("production gate failed: dry-run readiness cannot claim live key policy")
    if policy["operator_custody_policy_checked"] is not False or policy["operator_custody_policy_hash"] is not None:
        raise SystemExit("production gate failed: dry-run readiness cannot claim operator custody policy")
    if policy.get("operator_custody_policy_requirements") != []:
        raise SystemExit("production gate failed: dry-run readiness cannot claim operator custody requirements")
    if policy["operator_runbook_checked"] is not False or policy["operator_runbook_hash"] is not None:
        raise SystemExit("production gate failed: dry-run readiness cannot claim operator runbook")
    if policy.get("operator_runbook_requirements") != []:
        raise SystemExit("production gate failed: dry-run readiness cannot claim operator runbook requirements")
    blockers = set(policy.get("production_blockers", []))
    if "operator-custody-policy-missing" not in blockers or "operator-runbook-missing" not in blockers:
        raise SystemExit("production gate failed: operational policy must expose missing operator blockers")
    if policy["public_chain_ready"] is not False:
        raise SystemExit("production gate failed: dry-run readiness cannot claim public-chain ready")
    if policy["production_ready"] is not False:
        raise SystemExit("production gate failed: operational policy production marker")
    if len(policy["policy_commitment"]) != 64:
        raise SystemExit("production gate failed: operational policy commitment")
PY

run_step "Validate Session L2 reports" \
  python3 - \
    "${SESSION_OPEN_STATIC}" "${SESSION_COMMIT_STATIC}" "${SESSION_COURT_STATIC}" "${SESSION_VERIFY_STATIC}" "${SESSION_DA_STATIC}" "${SESSION_DA_VERIFY_STATIC}" "${SESSION_DA_ANCHOR_STATIC}" "${SESSION_DA_ANCHOR_VERIFY_STATIC}" "${SESSION_DA_ANCHOR_SUBMIT_STATIC}" "${SESSION_SETTLEMENT_STATIC}" "${SESSION_SETTLEMENT_VERIFY_STATIC}" "${SESSION_PACKAGE_STATIC}" "${SESSION_PACKAGE_VERIFY_STATIC}" "${SESSION_PACKAGE_SUBMIT_STATIC}" \
    "${SESSION_OPEN_WEIGHTED_PRECOMMIT}" "${SESSION_COMMIT_WEIGHTED_PRECOMMIT}" "${SESSION_COURT_WEIGHTED_PRECOMMIT}" "${SESSION_VERIFY_WEIGHTED_PRECOMMIT}" "${SESSION_DA_WEIGHTED_PRECOMMIT}" "${SESSION_DA_VERIFY_WEIGHTED_PRECOMMIT}" "${SESSION_DA_ANCHOR_WEIGHTED_PRECOMMIT}" "${SESSION_DA_ANCHOR_VERIFY_WEIGHTED_PRECOMMIT}" "${SESSION_DA_ANCHOR_SUBMIT_WEIGHTED_PRECOMMIT}" "${SESSION_SETTLEMENT_WEIGHTED_PRECOMMIT}" "${SESSION_SETTLEMENT_VERIFY_WEIGHTED_PRECOMMIT}" "${SESSION_PACKAGE_WEIGHTED_PRECOMMIT}" "${SESSION_PACKAGE_VERIFY_WEIGHTED_PRECOMMIT}" "${SESSION_PACKAGE_SUBMIT_WEIGHTED_PRECOMMIT}" <<'PY'
import json
import sys
from pathlib import Path

open_static, commit_static, court_static, verify_static, da_static, da_verify_static, da_anchor_static, da_anchor_verify_static, da_anchor_submit_static, settlement_static, settlement_verify_static, package_static, package_verify_static, package_submit_static, \
open_tm, commit_tm, court_tm, verify_tm, da_tm, da_verify_tm, da_anchor_tm, da_anchor_verify_tm, da_anchor_submit_tm, settlement_tm, settlement_verify_tm, package_tm, package_verify_tm, package_submit_tm = \
    [json.loads(Path(path).read_text()) for path in sys.argv[1:]]

def require(condition, message):
    if not condition:
        raise SystemExit(f"production gate failed: {message}")

for report, kind in ((open_static, "static-closed-committee"), (open_tm, "tendermint")):
    require(report["schema"] == "myelin-session-open", f"{kind} session open schema")
    require(report["consensus_kind"] == kind, f"{kind} session open consensus")
    require(report["vm_profile"] == "ckb-strict-basic", f"{kind} session open vm profile")
    require(report["ckb_spawn_ipc_required"] is False, f"{kind} session open spawn/IPC")
    require(len(report["session_id"]) == 64, f"{kind} session id")
    require(len(report["escrow_input_cells"]) >= 1, f"{kind} escrow cells")

for report, kind in ((commit_static, "static-closed-committee"), (commit_tm, "tendermint")):
    require(report["schema"] == "myelin-session-commit", f"{kind} session commit schema")
    require(report["consensus_kind"] == kind, f"{kind} session commit consensus")
    require(report["vm_profile"] == "ckb-strict-basic", f"{kind} session commit vm profile")
    require(report["ckb_spawn_ipc_required"] is False, f"{kind} session commit spawn/IPC")
    require(report["finalised"] is True, f"{kind} session finalised")
    require(report["pool_size_before"] == 0, f"{kind} session pool before")
    require(report["pool_size_after"] == 1, f"{kind} session pool after")
    require(report["state_root_before"] != report["state_root_after"], f"{kind} session state mutates")
    require(len(report["ordered_cell_tx_commitments"]) == 1, f"{kind} ordered tx commitments")
    require(len(report["data_commitments"]) == 1, f"{kind} data commitments")
    require(report["block"]["block_hash"], f"{kind} block hash present")

for report, verify, kind in ((court_static, verify_static, "static-closed-committee"),
                             (court_tm, verify_tm, "tendermint")):
    require(report["schema"] == "myelin-session-court-bundle", f"{kind} court schema")
    require(report["vm_profile"] == "ckb-strict-basic", f"{kind} court vm profile")
    require(report["ckb_spawn_ipc_required"] is False, f"{kind} court spawn/IPC")
    require(report["court_verifiable"] is False, f"{kind} court remains unverified until contextual CKB consensus evidence exists")
    require(report["l1_court_implemented"] is False, f"{kind} L1 court marker")
    require(report["ckb_projection"]["projection_stage"] == "wire-encoded", f"{kind} projection stage")
    require(report["ckb_projection"]["wire_encoded"] is True, f"{kind} projection wire encoding")
    require(verify["valid"] is True, f"{kind} court verification")
    require(all(check["ok"] for check in verify["checks"]), f"{kind} all court checks pass")

for da, verify, court, kind in ((da_static, da_verify_static, court_static, "static-closed-committee"),
                                (da_tm, da_verify_tm, court_tm, "tendermint")):
    require(da["schema"] == "myelin-session-da-manifest", f"{kind} DA schema")
    require(da["da_profile"] == "single-segment-merkle", f"{kind} DA profile")
    require(da["payload_kind"] == "session-court-molecule-transaction", f"{kind} DA payload kind")
    require(da["session_id"] == court["session_id"], f"{kind} DA session binding")
    require(da["chunk_index"] == court["chunk_index"], f"{kind} DA chunk binding")
    require(da["consensus_kind"] == kind, f"{kind} DA consensus")
    require(da["vm_profile"] == "ckb-strict-basic", f"{kind} DA vm profile")
    require(da["molecule_transaction_hash"] == court["molecule_transaction_hash"], f"{kind} DA molecule tx binding")
    require(da["challenge_payload_hash"] == court["challenge_payload_hash"], f"{kind} DA challenge binding")
    require(da["proof_valid"] is True, f"{kind} DA proof valid")
    availability = da["availability"]
    require(availability["schema"] == "myelin-da-availability", f"{kind} DA availability schema")
    require(availability["mode"] == "fixture-replicated-committee", f"{kind} DA availability mode")
    require(availability["signature_scheme"] == "secp256k1-recoverable-blake3-pubkey-hash20", f"{kind} DA availability signature scheme")
    require(availability["required_attestations"] == 2, f"{kind} DA availability threshold")
    require(availability["attestation_count"] >= availability["required_attestations"], f"{kind} DA availability attestation count")
    require(len(availability["attester_pubkey_hashes"]) >= availability["required_attestations"], f"{kind} DA availability pubkey-hash count")
    require(len(availability["attestation_signatures"]) >= availability["required_attestations"], f"{kind} DA availability signature count")
    require(all(len(pubkey_hash) == 40 for pubkey_hash in availability["attester_pubkey_hashes"]), f"{kind} DA availability pubkey-hash length")
    require(all(len(signature) == 130 for signature in availability["attestation_signatures"]), f"{kind} DA availability signature length")
    require(availability["attestation_signature_verified"] is True, f"{kind} DA availability local signature verification")
    require(availability["retrieval_probe_count"] >= 1, f"{kind} DA availability retrieval probe")
    require(availability["payload_hash"] == da["molecule_transaction_hash"], f"{kind} DA availability payload binding")
    require(availability["segment_root"] == da["segment_root"], f"{kind} DA availability segment-root binding")
    require(len(availability["proof_molecule_hash"]) == 64, f"{kind} DA availability proof hash")
    require(availability["external_receipt_count"] == 0, f"{kind} DA external receipt count")
    require(availability["external_receipt_checked"] is False, f"{kind} DA external receipt marker")
    require(availability["external_receipt"] is None, f"{kind} DA external receipt absence")
    require(len(availability["availability_commitment"]) == 64, f"{kind} DA availability commitment")
    require(availability["availability_checked"] is True, f"{kind} DA availability checked")
    require(availability["testnet_beta_ready"] is False, f"{kind} DA availability is commitment-only, not testnet-beta ready")
    require(availability["production_ready"] is False, f"{kind} DA availability production marker")
    require(da["local_da_published"] is True, f"{kind} local DA published")
    require(da["segment_sealed"] is True, f"{kind} DA segment sealed")
    require(da["da_storage_path"], f"{kind} DA storage path")
    require(da["l1_da_published"] is False, f"{kind} DA L1 publication marker")
    require(verify["valid"] is True, f"{kind} DA verification")
    require(all(check["ok"] for check in verify["checks"]), f"{kind} all DA checks pass")

for anchor, verify, da, kind in ((da_anchor_static, da_anchor_verify_static, da_static, "static-closed-committee"),
                                 (da_anchor_tm, da_anchor_verify_tm, da_tm, "tendermint")):
    require(anchor["schema"] == "myelin-session-da-anchor-package", f"{kind} DA anchor schema")
    require(anchor["session_id"] == da["session_id"], f"{kind} DA anchor session binding")
    require(anchor["chunk_index"] == da["chunk_index"], f"{kind} DA anchor chunk binding")
    require(anchor["consensus_kind"] == kind, f"{kind} DA anchor consensus")
    require(anchor["vm_profile"] == "ckb-strict-basic", f"{kind} DA anchor vm profile")
    require(len(anchor["da_manifest_hash"]) == 64, f"{kind} DA anchor manifest hash")
    require(anchor["court_bundle_hash"] == da["court_bundle_hash"], f"{kind} DA anchor court hash binding")
    require(anchor["court_molecule_transaction_hash"] == da["molecule_transaction_hash"], f"{kind} DA anchor molecule tx binding")
    require(anchor["challenge_payload_hash"] == da["challenge_payload_hash"], f"{kind} DA anchor challenge binding")
    require(anchor["segment_root"] == da["segment_root"], f"{kind} DA anchor segment-root binding")
    require(len(anchor["da_anchor_cell_tx_id"]) == 64, f"{kind} DA anchor txid")
    require(len(anchor["da_anchor_cell_wtxid"]) == 64, f"{kind} DA anchor wtxid")
    require(len(anchor["molecule_transaction_hash"]) == 64, f"{kind} DA anchor molecule tx hash")
    require(anchor["ckb_projection"]["projection_stage"] == "wire-encoded", f"{kind} DA anchor projection stage")
    require(anchor["ckb_projection"]["wire_encoded"] is True, f"{kind} DA anchor wire encoding")
    require(anchor["da_anchor_wire_encoded"] is True, f"{kind} DA anchor wire encoding")
    require(anchor["l1_da_publication_implemented"] is False, f"{kind} DA anchor L1 publication marker")
    require(verify["valid"] is True, f"{kind} DA anchor verification")
    require(all(check["ok"] for check in verify["checks"]), f"{kind} all DA anchor checks pass")

for submit, anchor, kind in ((da_anchor_submit_static, da_anchor_static, "static-closed-committee"),
                             (da_anchor_submit_tm, da_anchor_tm, "tendermint")):
    require(submit["schema"] == "myelin-session-da-anchor-submission", f"{kind} DA anchor submit schema")
    require(submit["dry_run"] is True, f"{kind} DA anchor submit dry-run")
    require(submit["request_method"] == "send_transaction", f"{kind} DA anchor submit method")
    require(submit["outputs_validator"] == "passthrough", f"{kind} DA anchor submit outputs validator")
    require(len(submit["request_json_hash"]) == 64, f"{kind} DA anchor submit request hash")
    require(submit["da_anchor_cell_tx_id"] == anchor["da_anchor_cell_tx_id"], f"{kind} DA anchor submit txid binding")
    require(submit["da_anchor_cell_wtxid"] == anchor["da_anchor_cell_wtxid"], f"{kind} DA anchor submit wtxid binding")
    require(submit["molecule_transaction_hash"] == anchor["molecule_transaction_hash"], f"{kind} DA anchor submit molecule binding")
    require(submit["ckb_raw_tx_hash"] == anchor["ckb_projection"]["ckb_raw_tx_hash"], f"{kind} DA anchor submit raw hash")
    require(submit["ckb_wtx_hash"] == anchor["ckb_projection"]["ckb_wtx_hash"], f"{kind} DA anchor submit wtx hash")
    require(submit["ckb_transaction_json"]["outputs_data"], f"{kind} DA anchor submit CKB outputs data")
    require(submit["submitted_to_rpc"] is False, f"{kind} DA anchor submit no offline RPC")
    require(submit["accepted_by_rpc"] is False, f"{kind} DA anchor submit no offline acceptance")
    require(submit["l1_da_published"] is False, f"{kind} DA anchor submit L1 marker")

for settlement, verify, court, da, kind in ((settlement_static, settlement_verify_static, court_static, da_static, "static-closed-committee"),
                                            (settlement_tm, settlement_verify_tm, court_tm, da_tm, "tendermint")):
    require(settlement["schema"] == "myelin-session-settlement-intent", f"{kind} settlement schema")
    require(settlement["kind"] == "disputed-close", f"{kind} settlement kind")
    require(settlement["session_id"] == court["session_id"], f"{kind} settlement session binding")
    require(settlement["chunk_index"] == court["chunk_index"], f"{kind} settlement chunk binding")
    require(settlement["consensus_kind"] == kind, f"{kind} settlement consensus")
    require(settlement["vm_profile"] == "ckb-strict-basic", f"{kind} settlement vm profile")
    require(settlement["state_root_before"] == court["state_root_before"], f"{kind} settlement before-root binding")
    require(settlement["final_state_root"] == court["state_root_after"], f"{kind} settlement final-root binding")
    require(settlement["challenge_payload_hash"] == court["challenge_payload_hash"], f"{kind} settlement challenge binding")
    require(settlement["da_profile"] == da["da_profile"], f"{kind} settlement DA profile binding")
    require(settlement["da_segment_root"] == da["segment_root"], f"{kind} settlement DA segment-root binding")
    require(settlement["challenge_window_ms"] == 60000, f"{kind} settlement challenge window")
    require(settlement["challenge_deadline_ms"] == 60000, f"{kind} settlement challenge deadline")
    require(settlement["settlement_permitted"] is True, f"{kind} settlement permitted")
    economics = settlement["court_economics"]
    require(economics["schema"] == "myelin-session-court-economics", f"{kind} court economics schema")
    require(economics["mode"] == "disputed-close-explicit-policy", f"{kind} court economics mode")
    require(economics["escrow_binding_mode"] == "session-escrow-input-cells-hash", f"{kind} court economics escrow binding mode")
    require(economics["minimum_dispute_bond_shannons"] == 100000000, f"{kind} court economics minimum bond")
    require(economics["challenger_reward_bps"] == 5000, f"{kind} court economics challenger reward")
    require(economics["loser_slash_bps"] == 10000, f"{kind} court economics loser slash")
    require(economics["honest_party_refund_bps"] == 5000, f"{kind} court economics honest refund")
    require(economics["unresolved_remainder_bps"] == 0, f"{kind} court economics unresolved remainder")
    require(economics["payout_balance_bps"] == economics["loser_slash_bps"], f"{kind} court economics payout balance")
    require(
        economics["challenger_reward_bps"] + economics["honest_party_refund_bps"] + economics["unresolved_remainder_bps"] == economics["payout_balance_bps"],
        f"{kind} court economics payout conservation",
    )
    require(economics["settlement_after_deadline_only"] is True, f"{kind} court economics deadline-only settlement")
    require(economics["da_evidence_required"] is True, f"{kind} court economics DA requirement")
    require(economics["economics_invariant_checked"] is True, f"{kind} court economics invariant check")
    require(economics["participant_set_hash"] == court["participant_set_hash"], f"{kind} court economics participant binding")
    require(economics["escrow_input_cells_hash"] == court["escrow_input_cells_hash"], f"{kind} court economics escrow binding")
    require(economics["challenge_payload_hash"] == court["challenge_payload_hash"], f"{kind} court economics challenge binding")
    require(economics["da_availability_commitment"] == da["availability"]["availability_commitment"], f"{kind} court economics DA availability binding")
    require(economics["court_economics_checked"] is True, f"{kind} court economics checked")
    require("court_economics_deployment" not in economics, f"{kind} default court economics has no deployment evidence")
    require(economics["testnet_beta_ready"] is False, f"{kind} court economics is explicit local policy, not testnet-beta ready")
    require(economics["production_ready"] is False, f"{kind} court economics production marker")
    require(len(economics["economics_commitment"]) == 64, f"{kind} court economics commitment")
    require(settlement["l1_da_published"] is False, f"{kind} settlement L1 DA marker")
    require(settlement["l1_court_implemented"] is False, f"{kind} settlement L1 court marker")
    require(verify["valid"] is True, f"{kind} settlement verification")
    require(all(check["ok"] for check in verify["checks"]), f"{kind} all settlement checks pass")

for package, verify, settlement, court, da, kind in ((package_static, package_verify_static, settlement_static, court_static, da_static, "static-closed-committee"),
                                                     (package_tm, package_verify_tm, settlement_tm, court_tm, da_tm, "tendermint")):
    require(package["schema"] == "myelin-session-settlement-package", f"{kind} settlement package schema")
    require(package["session_id"] == settlement["session_id"], f"{kind} package session binding")
    require(package["chunk_index"] == settlement["chunk_index"], f"{kind} package chunk binding")
    require(package["consensus_kind"] == kind, f"{kind} package consensus")
    require(package["vm_profile"] == "ckb-strict-basic", f"{kind} package vm profile")
    require(len(package["intent_hash"]) == 64, f"{kind} package intent hash")
    require(package["court_bundle_hash"] == settlement["court_bundle_hash"], f"{kind} package court hash binding")
    require(package["da_manifest_hash"] == settlement["da_manifest_hash"], f"{kind} package DA hash binding")
    require(package["challenge_payload_hash"] == settlement["challenge_payload_hash"], f"{kind} package challenge binding")
    require(package["court_economics"] == settlement["court_economics"], f"{kind} package court economics binding")
    require(package["final_state_root"] == settlement["final_state_root"], f"{kind} package final root")
    authority = package["settlement_authority"]
    auth = authority["authority_authentication"]
    require(authority["schema"] == "myelin-session-settlement-authority", f"{kind} authority schema")
    require(len(authority["data"]) == 386, f"{kind} authority data length")
    require(len(authority["data_hash"]) == 66, f"{kind} authority data hash")
    require(authority["data_semantics"] == "settlement-authority-lineage", f"{kind} authority data semantics")
    require(authority["session_id"] == package["session_id"], f"{kind} authority session binding")
    require(authority["participant_set_hash"] == package["participant_set_hash"], f"{kind} authority participant binding")
    require(authority["escrow_input_cells_hash"] == package["escrow_input_cells_hash"], f"{kind} authority escrow binding")
    require(authority["session_lineage_commitment"] == package["session_lineage_commitment"], f"{kind} authority lineage binding")
    require(auth["schema"] == "myelin-session-settlement-authority-auth", f"{kind} authority authentication schema")
    require(auth["mode"] == "ckb-secp256k1-blake160-multisig-all", f"{kind} authority authentication mode")
    require(auth["signature_scheme"] == "secp256k1-recoverable-blake3-message-ckb-blake160-pubkey-hash", f"{kind} authority authentication signature scheme")
    require(auth["threshold"] == 2, f"{kind} authority authentication threshold")
    require(auth["signer_count"] >= auth["threshold"], f"{kind} authority authentication signer count")
    require(len(auth["signer_pubkey_hashes"]) >= auth["threshold"], f"{kind} authority authentication pubkey-hash count")
    require(len(auth["signatures"]) >= auth["threshold"], f"{kind} authority authentication signature count")
    require(all(len(pubkey_hash) == 40 for pubkey_hash in auth["signer_pubkey_hashes"]), f"{kind} authority authentication pubkey-hash length")
    require(all(len(signature) == 130 for signature in auth["signatures"]), f"{kind} authority authentication signature length")
    require(auth["signature_verified"] is True, f"{kind} authority authentication local signature verification")
    require(auth["participant_set_hash"] == authority["participant_set_hash"], f"{kind} authority authentication participant binding")
    require(len(auth["message_hash"]) == 64, f"{kind} authority authentication message hash")
    require(len(auth["attestation_hash"]) == 64, f"{kind} authority authentication hash")
    require(auth["ckb_lock_args"].startswith("0x"), f"{kind} authority authentication lock args prefix")
    require(len(auth["ckb_lock_args"]) == 42, f"{kind} canonical multisig lock args are Blake160")
    require(auth["ckb_multisig_require_first_n"] == 0, f"{kind} canonical multisig require_first_n")
    require(len(auth["ckb_multisig_participant_pubkey_hashes"]) == 3, f"{kind} canonical multisig participant count")
    require(all(len(pubkey_hash) == 40 for pubkey_hash in auth["ckb_multisig_participant_pubkey_hashes"]), f"{kind} canonical multisig participant hash length")
    multisig_config = bytes.fromhex(auth["ckb_multisig_witness_config"].removeprefix("0x"))
    require(multisig_config[:4] == bytes([0, 0, 2, 3]), f"{kind} canonical multisig config header")
    require(len(multisig_config) == 64, f"{kind} canonical multisig config length")
    require(len(auth["ckb_multisig_config_hash"]) == 64, f"{kind} canonical multisig config hash")
    require(len(auth["ckb_lock_args_hash"]) == 64, f"{kind} authority authentication lock args hash")
    require(len(bytes.fromhex(auth["ckb_lock_args_hash"])) == 32, f"{kind} authority authentication lock args hash hex")
    require("participant_signature_evidence" not in auth, f"{kind} default authority authentication has no participant signature evidence")
    require("threshold_lock_deployment" not in auth, f"{kind} default authority authentication has no deployment evidence")
    require(auth["ckb_enforceable"] is False, f"{kind} authority authentication is not yet CKB-enforceable")
    require(auth["testnet_beta_ready"] is False, f"{kind} authority authentication is commitment-only, not testnet-beta ready")
    require(auth["production_ready"] is False, f"{kind} authority authentication production marker")
    require(len(package["settlement_cell_tx_id"]) == 64, f"{kind} package txid")
    require(len(package["settlement_cell_wtxid"]) == 64, f"{kind} package wtxid")
    require(len(package["molecule_transaction_hash"]) == 64, f"{kind} package molecule tx hash")
    require(package["ckb_projection"]["projection_stage"] == "wire-encoded", f"{kind} package projection stage")
    require(package["ckb_projection"]["wire_encoded"] is True, f"{kind} package wire encoding")
    require(package["settlement_wire_encoded"] is True, f"{kind} package wire encoding")
    require(package["l1_court_script_implemented"] is False, f"{kind} package L1 court marker")
    require(verify["valid"] is True, f"{kind} package verification")
    require(all(check["ok"] for check in verify["checks"]), f"{kind} all package checks pass")

for submit, package, kind in ((package_submit_static, package_static, "static-closed-committee"),
                              (package_submit_tm, package_tm, "tendermint")):
    require(submit["schema"] == "myelin-session-settlement-submission", f"{kind} settlement submit schema")
    require(submit["dry_run"] is True, f"{kind} settlement submit dry-run")
    require(submit["request_method"] == "send_transaction", f"{kind} settlement submit method")
    require(submit["outputs_validator"] == "passthrough", f"{kind} settlement submit outputs validator")
    require(len(submit["request_json_hash"]) == 64, f"{kind} settlement submit request hash")
    require(submit["settlement_cell_tx_id"] == package["settlement_cell_tx_id"], f"{kind} settlement submit txid binding")
    require(submit["settlement_cell_wtxid"] == package["settlement_cell_wtxid"], f"{kind} settlement submit wtxid binding")
    require(submit["molecule_transaction_hash"] == package["molecule_transaction_hash"], f"{kind} settlement submit molecule binding")
    require(submit["court_economics"] == package["court_economics"], f"{kind} settlement submit court economics binding")
    require(submit["ckb_raw_tx_hash"] == package["ckb_projection"]["ckb_raw_tx_hash"], f"{kind} settlement submit raw hash")
    require(submit["ckb_wtx_hash"] == package["ckb_projection"]["ckb_wtx_hash"], f"{kind} settlement submit wtx hash")
    require(submit["ckb_transaction_json"]["outputs_data"], f"{kind} settlement submit CKB outputs data")
    require(submit["submitted_to_rpc"] is False, f"{kind} settlement submit no offline RPC")
    require(submit["accepted_by_rpc"] is False, f"{kind} settlement submit no offline acceptance")
    require(submit["l1_court_submitted"] is False, f"{kind} settlement submit L1 marker")

require(open_static["session_id"] == open_tm["session_id"], "session id must be consensus-independent")
for field in ("session_id", "cell_tx_id", "cell_wtxid", "state_root_before", "state_root_after",
              "ordered_cell_tx_commitments", "data_commitments", "scheduler_commitment"):
    require(commit_static[field] == commit_tm[field], f"{field} must match across session consensus engines")
require(commit_static["block"]["block_hash"] != commit_tm["block"]["block_hash"],
        "session block hashes must remain consensus-domain separated")
require(commit_static["static_committee_evidence"]["finalised"] is True, "static session evidence finalised")
require(commit_static["weighted_precommit_evidence"] is None, "static session has no WeightedPrecommit evidence")
require(commit_tm["static_committee_evidence"]["finalised"] is False, "WeightedPrecommit session static slot is informational")
require(commit_tm["weighted_precommit_evidence"]["finalised"] is True, "WeightedPrecommit session evidence finalised")

print(json.dumps({
    "session_id": open_static["session_id"],
    "state_root_after": commit_static["state_root_after"],
    "static_block_hash": commit_static["block"]["block_hash"],
    "weighted_precommit_block_hash": commit_tm["block"]["block_hash"],
    "static_checks": len(verify_static["checks"]),
    "static_da_checks": len(da_verify_static["checks"]),
    "static_da_anchor_checks": len(da_anchor_verify_static["checks"]),
    "static_da_anchor_submit_dry_run": da_anchor_submit_static["request_method"],
    "weighted_precommit_checks": len(verify_tm["checks"]),
    "weighted_precommit_da_checks": len(da_verify_tm["checks"]),
    "weighted_precommit_da_anchor_checks": len(da_anchor_verify_tm["checks"]),
    "weighted_precommit_da_anchor_submit_dry_run": da_anchor_submit_tm["request_method"],
    "static_settlement_checks": len(settlement_verify_static["checks"]),
    "weighted_precommit_settlement_checks": len(settlement_verify_tm["checks"]),
    "static_package_checks": len(package_verify_static["checks"]),
    "static_package_submit_dry_run": package_submit_static["request_method"],
    "weighted_precommit_package_checks": len(package_verify_tm["checks"]),
    "weighted_precommit_package_submit_dry_run": package_submit_tm["request_method"],
}, indent=2, sort_keys=True))
PY

# 12. Dependency tree must not reintroduce the forbidden bloated surface.
run_step "Scan dependency tree for forbidden crates" \
  bash -c "set -e; cargo tree -p myelin-cli -e normal > '${OUTPUT_DIR}/cli-tree.txt'; cargo tree -p myelin-exec -e normal > '${OUTPUT_DIR}/exec-tree.txt'; if rg -q 'workflow-node|workflow-perf-monitor' '${OUTPUT_DIR}/cli-tree.txt' '${OUTPUT_DIR}/exec-tree.txt'; then echo 'forbidden workflow crate leaked back into dependency tree' >&2; rg -n 'workflow-node|workflow-perf-monitor' '${OUTPUT_DIR}/cli-tree.txt' '${OUTPUT_DIR}/exec-tree.txt' >&2; exit 1; fi; echo 'dependency tree clean of workflow-* crates'; cat '${OUTPUT_DIR}/cli-tree.txt' | sed -n '1,40p'"

# 13. Stale-surface grep
run_step "Scan Myelin tree for stale identity" python3 - <<'PY'
import subprocess
import sys

paths = [
    "README.md", "CHANGELOG.md", "docs", "scripts", "cli", "consensus", "exec", "state", "mempool",
    "crypto", "math", "core-utils", "ckb-adapter", "cellscript-adapter", "fixtures",
    "session", "session-network", "session-runtime", "session-store-rocksdb", "session-escrow", "wallet-auth",
    "website",
]
version_paths = [
    *paths,
    "MYELIN_CKB_PROJECTION_AUDIT.md",
    "MYELIN_CKB_SEMANTIC_DEVIATIONS.md",
    "MYELIN_PRODUCTION_REHEARSAL_REPORT.md",
    "MYELIN_SESSION_L2_PLAN.md",
]
# The production gate is allowed to name the patterns it scans for; the scan
# itself is not subject to the scan. We exclude the gate and the audit doc
# explicitly so the gate can be the auditor.
exclude = ("scripts/myelin_production_gate.sh", "scripts/myelin_teeworlds_acceptance.sh",
           "scripts/build_myelin_teeworlds_repro.py",
           "MYELIN_CKB_SEMANTIC_DEVIATIONS.md")

# Forbidden vocabulary for the active Myelin tree.
version_patterns = [
    r"myelin[a-z0-9:+._-]*[-:]v[0-9]+([^-a-z0-9]|$)",
    r"(single-segment-merkle|disputed-close-explicit-policy|settlement-authority-lineage|mainnet-production-court-dispute-economics|testnet-beta-court-dispute-economics)-v[0-9]+([^a-z0-9-]|$)",
    r"ignored-v[0-9]+-binding-hash",
    r"Script::hash_v[0-9]+",
    r"SCRIPT_HASH_V[0-9]+",
    r"place-entry-witness-v[0-9]+",
]
patterns = [
    "Spora", "spora",
    "NovaSeal", "novaseal",
    "certifier", "certify",
    "website/astro", "website/src",
    "editors/vscode-cellscript",
    "cellscript_gate.sh",
    "novaseal_",
    "release[-_ ]note",
    *version_patterns,
]

failed = False
for pattern in patterns:
    # Myelin is pre-release and has one current protocol surface. The broader
    # version scan includes authoritative root documents and rejects owned
    # generation labels without banning upstream names such as CKB multisig-v2
    # or the pinned CellScript witness ABI.
    scan_paths = version_paths if pattern in version_patterns else paths
    pattern_exclude = ("scripts/myelin_production_gate.sh",) if pattern in version_patterns else exclude
    command = ["rg", "-n", "-S", "-i", pattern, *scan_paths]
    result = subprocess.run(command, cwd=".", text=True, capture_output=True)
    if result.returncode == 0:
        lines = [line for line in result.stdout.splitlines() if not any(line.startswith(ex) for ex in pattern_exclude)]
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

# 14. Forbidden parent-path audit
run_step "Audit for forbidden parent Spora path references" python3 - <<'PY'
import subprocess
import sys

paths = [
    "README.md", "docs", "scripts", "cli", "consensus", "exec", "state", "mempool",
    "crypto", "math", "core-utils", "wallet-auth", "Cargo.toml", "Cargo.lock",
]
exclude = ("scripts/myelin_production_gate.sh", "MYELIN_CKB_SEMANTIC_DEVIATIONS.md")
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

# 15. Teeworlds acceptance, required by default
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
