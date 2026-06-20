#!/usr/bin/env bash
# Prepare Myelin public-testnet rehearsal artefacts through Phases 1-3.
#
# This is not a release gate. It deliberately does not submit to CKB. It gives
# an operator a reproducible artefact directory that can then be replaced or
# extended with real public-testnet DA, signing, deployment, and submission
# evidence.

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
REHEARSAL_DIR="${MYELIN_REHEARSAL_DIR:-$(mktemp -d /tmp/myelin-public-testnet-rehearsal-prepare.XXXXXX)}"
SUMMARY_PATH="${SUMMARY_PATH:-"${REHEARSAL_DIR}/rehearsal-prepare-summary.json"}"
PROVENANCE="${MYELIN_REHEARSAL_PROVENANCE:-local-fixture-disposable-keys}"

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing required command: $1" >&2
    exit 1
  }
}

run_step() {
  printf '\n==> %s\n' "$1" >&2
  shift
  "$@"
}

myelin() {
  if [[ -n "${MYELIN_BIN:-}" ]]; then
    "${MYELIN_BIN}" "$@"
  else
    cargo run -q -p myelin-cli -- "$@"
  fi
}

hex_repeat() {
  local byte="$1"
  local count="$2"
  local out=""
  for _ in $(seq 1 "$count"); do
    out="${out}${byte}"
  done
  printf '%s' "$out"
}

assert_valid() {
  local path="$1"
  jq -e '.valid == true' "$path" >/dev/null
}

require_cmd jq
require_cmd seq

cd "$ROOT"
mkdir -p "$REHEARSAL_DIR"

DA_PROVIDER="${MYELIN_DA_PROVIDER:-local-rehearsal-da-provider}"
DA_RECEIPT_ID="${MYELIN_DA_RECEIPT_ID:-local-rehearsal-receipt-0001}"
DA_RETRIEVAL_ENDPOINT="${MYELIN_DA_RETRIEVAL_ENDPOINT:-https://da.example.invalid/session-court-payloads/${DA_RECEIPT_ID}}"
DA_AUDIT_LOG_COMMITMENT="${MYELIN_DA_AUDIT_LOG_COMMITMENT:-0x$(hex_repeat a5 32)}"
DA_PROVIDER_SECRET_KEY="${MYELIN_LOCAL_DA_PROVIDER_SECRET_KEY:-$(hex_repeat 44 32)}"
AUTHORITY_SIGNER_KEY_0="${MYELIN_LOCAL_AUTHORITY_SIGNER_KEY_0:-$(hex_repeat 11 32)}"
AUTHORITY_SIGNER_KEY_1="${MYELIN_LOCAL_AUTHORITY_SIGNER_KEY_1:-$(hex_repeat 22 32)}"

COURT_VERIFIER_CODE_HASH="${MYELIN_COURT_VERIFIER_CODE_HASH:-0x$(hex_repeat e1 32)}"
COURT_VERIFIER_CODE_DEP_TX_HASH="${MYELIN_COURT_VERIFIER_CODE_DEP_TX_HASH:-0x$(hex_repeat e2 32)}"
COURT_VERIFIER_CODE_DEP_INDEX="${MYELIN_COURT_VERIFIER_CODE_DEP_INDEX:-0x0}"
COURT_VERIFIER_SOURCE_HASH="${MYELIN_COURT_VERIFIER_SOURCE_HASH:-0x$(hex_repeat e3 32)}"
COURT_VERIFIER_AUDIT_HASH="${MYELIN_COURT_VERIFIER_AUDIT_HASH:-0x$(hex_repeat e4 32)}"
THRESHOLD_LOCK_CODE_HASH="${MYELIN_THRESHOLD_LOCK_CODE_HASH:-0x$(hex_repeat a1 32)}"
THRESHOLD_LOCK_CODE_DEP_TX_HASH="${MYELIN_THRESHOLD_LOCK_CODE_DEP_TX_HASH:-0x$(hex_repeat b2 32)}"
THRESHOLD_LOCK_CODE_DEP_INDEX="${MYELIN_THRESHOLD_LOCK_CODE_DEP_INDEX:-0x0}"
THRESHOLD_LOCK_SOURCE_HASH="${MYELIN_THRESHOLD_LOCK_SOURCE_HASH:-0x$(hex_repeat c3 32)}"
THRESHOLD_LOCK_AUDIT_HASH="${MYELIN_THRESHOLD_LOCK_AUDIT_HASH:-0x$(hex_repeat d4 32)}"

run_step "Copy rehearsal sources and operator starter documents" bash -c '
  set -euo pipefail
  cp cellscript/examples/myelin/da-anchor-carrier.cell "$0/"
  cp cellscript/examples/myelin/settlement-carrier.cell "$0/"
  cp cellscript/examples/myelin/da-anchor-final.cell "$0/"
  cp cellscript/examples/myelin/settlement-final.cell "$0/"
  cp docs/templates/public-testnet-rehearsal/operator-custody-policy.json "$0/"
  cp docs/templates/public-testnet-rehearsal/operator-runbook.json "$0/"
' "$REHEARSAL_DIR"

run_step "Phase 1: build and verify deterministic session artefacts" myelin session open-fixture \
  --consensus static-closed-committee \
  --out "$REHEARSAL_DIR/session-open.json"
myelin session commit-fixture \
  --session "$REHEARSAL_DIR/session-open.json" \
  --out "$REHEARSAL_DIR/session-commit.json"
myelin session court-bundle \
  --commit "$REHEARSAL_DIR/session-commit.json" \
  --chunk-index 0 \
  --out "$REHEARSAL_DIR/session-court.json"
myelin session verify-court-bundle \
  --bundle "$REHEARSAL_DIR/session-court.json" \
  --out "$REHEARSAL_DIR/session-court-verify.json"
assert_valid "$REHEARSAL_DIR/session-court-verify.json"

run_step "Phase 2: build local DA evidence and external DA receipt artefacts" myelin session da-manifest \
  --bundle "$REHEARSAL_DIR/session-court.json" \
  --out "$REHEARSAL_DIR/session-da-in-memory.json"
DA_PAYLOAD_HASH="$(jq -r '.molecule_transaction_hash' "$REHEARSAL_DIR/session-da-in-memory.json")"
DA_SEGMENT_ROOT="$(jq -r '.segment_root' "$REHEARSAL_DIR/session-da-in-memory.json")"
myelin session external-da-receipt \
  --payload-hash "$DA_PAYLOAD_HASH" \
  --segment-root "$DA_SEGMENT_ROOT" \
  --provider "$DA_PROVIDER" \
  --namespace session-court-payloads \
  --receipt-id "$DA_RECEIPT_ID" \
  --availability-window production-retention-30d \
  --service-level production \
  --retention-seconds 2592000 \
  --retrieval-endpoint "$DA_RETRIEVAL_ENDPOINT" \
  --audit-log-commitment "$DA_AUDIT_LOG_COMMITMENT" \
  --signing-request \
  --out "$REHEARSAL_DIR/external-da-receipt.signing-request.json"
myelin session external-da-receipt \
  --payload-hash "$DA_PAYLOAD_HASH" \
  --segment-root "$DA_SEGMENT_ROOT" \
  --provider "$DA_PROVIDER" \
  --namespace session-court-payloads \
  --receipt-id "$DA_RECEIPT_ID" \
  --availability-window production-retention-30d \
  --service-level production \
  --retention-seconds 2592000 \
  --retrieval-endpoint "$DA_RETRIEVAL_ENDPOINT" \
  --audit-log-commitment "$DA_AUDIT_LOG_COMMITMENT" \
  --provider-secret-key "$DA_PROVIDER_SECRET_KEY" \
  --out "$REHEARSAL_DIR/external-da-receipt.json"
myelin session da-manifest \
  --bundle "$REHEARSAL_DIR/session-court.json" \
  --storage-dir "$REHEARSAL_DIR/session-da-store" \
  --external-da-receipt "$REHEARSAL_DIR/external-da-receipt.json" \
  --out "$REHEARSAL_DIR/session-da.json"
myelin session verify-da-manifest \
  --manifest "$REHEARSAL_DIR/session-da.json" \
  --bundle "$REHEARSAL_DIR/session-court.json" \
  --storage-dir "$REHEARSAL_DIR/session-da-store" \
  --out "$REHEARSAL_DIR/session-da-verify.json"
assert_valid "$REHEARSAL_DIR/session-da-verify.json"

run_step "Phase 3: build and verify DA anchor and settlement packages" myelin session da-anchor-package \
  --manifest "$REHEARSAL_DIR/session-da.json" \
  --bundle "$REHEARSAL_DIR/session-court.json" \
  --out "$REHEARSAL_DIR/session-da-anchor-package.json"
myelin session verify-da-anchor-package \
  --package "$REHEARSAL_DIR/session-da-anchor-package.json" \
  --manifest "$REHEARSAL_DIR/session-da.json" \
  --bundle "$REHEARSAL_DIR/session-court.json" \
  --out "$REHEARSAL_DIR/session-da-anchor-package-verify.json"
assert_valid "$REHEARSAL_DIR/session-da-anchor-package-verify.json"
myelin session settlement-intent \
  --bundle "$REHEARSAL_DIR/session-court.json" \
  --da-manifest "$REHEARSAL_DIR/session-da.json" \
  --kind disputed-close \
  --current-time-ms 60000 \
  --challenge-window-ms 60000 \
  --out "$REHEARSAL_DIR/session-settlement-intent.base.json"
myelin session court-economics-deployment-evidence \
  --intent "$REHEARSAL_DIR/session-settlement-intent.base.json" \
  --network ckb-testnet \
  --verifier-code-hash "$COURT_VERIFIER_CODE_HASH" \
  --verifier-hash-type data2 \
  --verifier-code-dep-tx-hash "$COURT_VERIFIER_CODE_DEP_TX_HASH" \
  --verifier-code-dep-index "$COURT_VERIFIER_CODE_DEP_INDEX" \
  --audited-source-hash "$COURT_VERIFIER_SOURCE_HASH" \
  --audit-report-hash "$COURT_VERIFIER_AUDIT_HASH" \
  --ckb-enforceable-checked \
  --testnet-beta-ready \
  --out "$REHEARSAL_DIR/court-economics-deployment.json"
myelin session settlement-intent \
  --bundle "$REHEARSAL_DIR/session-court.json" \
  --da-manifest "$REHEARSAL_DIR/session-da.json" \
  --kind disputed-close \
  --current-time-ms 60000 \
  --challenge-window-ms 60000 \
  --court-economics-deployment-evidence "$REHEARSAL_DIR/court-economics-deployment.json" \
  --out "$REHEARSAL_DIR/session-settlement-intent.json"
myelin session verify-settlement-intent \
  --intent "$REHEARSAL_DIR/session-settlement-intent.json" \
  --bundle "$REHEARSAL_DIR/session-court.json" \
  --da-manifest "$REHEARSAL_DIR/session-da.json" \
  --out "$REHEARSAL_DIR/session-settlement-intent-verify.json"
assert_valid "$REHEARSAL_DIR/session-settlement-intent-verify.json"
myelin session settlement-package \
  --intent "$REHEARSAL_DIR/session-settlement-intent.json" \
  --bundle "$REHEARSAL_DIR/session-court.json" \
  --da-manifest "$REHEARSAL_DIR/session-da.json" \
  --out "$REHEARSAL_DIR/session-settlement-package.base.json"
myelin session authority-signature-evidence \
  --package "$REHEARSAL_DIR/session-settlement-package.base.json" \
  --signer-secret-key "$AUTHORITY_SIGNER_KEY_0" \
  --signer-secret-key "$AUTHORITY_SIGNER_KEY_1" \
  --out "$REHEARSAL_DIR/authority-signature-evidence.json"
myelin session threshold-lock-deployment-evidence \
  --package "$REHEARSAL_DIR/session-settlement-package.base.json" \
  --network ckb-testnet \
  --code-hash "$THRESHOLD_LOCK_CODE_HASH" \
  --hash-type data2 \
  --code-dep-tx-hash "$THRESHOLD_LOCK_CODE_DEP_TX_HASH" \
  --code-dep-index "$THRESHOLD_LOCK_CODE_DEP_INDEX" \
  --audited-source-hash "$THRESHOLD_LOCK_SOURCE_HASH" \
  --audit-report-hash "$THRESHOLD_LOCK_AUDIT_HASH" \
  --ckb-enforceable-checked \
  --testnet-beta-ready \
  --out "$REHEARSAL_DIR/threshold-lock-deployment.json"
myelin session settlement-package \
  --intent "$REHEARSAL_DIR/session-settlement-intent.json" \
  --bundle "$REHEARSAL_DIR/session-court.json" \
  --da-manifest "$REHEARSAL_DIR/session-da.json" \
  --authority-signature-evidence "$REHEARSAL_DIR/authority-signature-evidence.json" \
  --threshold-lock-deployment-evidence "$REHEARSAL_DIR/threshold-lock-deployment.json" \
  --out "$REHEARSAL_DIR/session-settlement-package.json"
myelin session verify-settlement-package \
  --package "$REHEARSAL_DIR/session-settlement-package.json" \
  --intent "$REHEARSAL_DIR/session-settlement-intent.json" \
  --bundle "$REHEARSAL_DIR/session-court.json" \
  --da-manifest "$REHEARSAL_DIR/session-da.json" \
  --out "$REHEARSAL_DIR/session-settlement-package-verify.json"
assert_valid "$REHEARSAL_DIR/session-settlement-package-verify.json"

jq -n \
  --arg schema "myelin-public-testnet-rehearsal-prepare-summary-v1" \
  --arg dir "$REHEARSAL_DIR" \
  --arg provenance "$PROVENANCE" \
  --slurpfile court "$REHEARSAL_DIR/session-court-verify.json" \
  --slurpfile da "$REHEARSAL_DIR/session-da-verify.json" \
  --slurpfile da_manifest "$REHEARSAL_DIR/session-da.json" \
  --slurpfile anchor "$REHEARSAL_DIR/session-da-anchor-package-verify.json" \
  --slurpfile intent "$REHEARSAL_DIR/session-settlement-intent-verify.json" \
  --slurpfile package "$REHEARSAL_DIR/session-settlement-package-verify.json" \
  '{
    schema: $schema,
    rehearsal_dir: $dir,
    provenance: $provenance,
    public_testnet_submitted: false,
    public_testnet_complete: false,
    production_interpretation: "local fixture preparation only; do not treat production_ready booleans as public-testnet or mainnet evidence",
    deployment_evidence_provenance: "local rehearsal values unless MYELIN_* deployment variables are supplied",
    phases_completed_locally: ["phase-1-session-artefacts", "phase-2-da-evidence", "phase-3-packages"],
    checks: {
      court_bundle_valid: $court[0].valid,
      da_manifest_valid: $da[0].valid,
      da_availability_production_ready: $da_manifest[0].availability.production_ready,
      da_anchor_package_valid: $anchor[0].valid,
      settlement_intent_valid: $intent[0].valid,
      settlement_package_valid: $package[0].valid
    },
    remaining_public_testnet_inputs: [
      "real external DA provider signature or explicitly labelled rehearsal-provider signature",
      "public CKB testnet funding cell",
      "public CKB testnet verifier code-dep out-points",
      "participant authority signatures from external signer workflow",
      "carrier or final-script submission tx hashes",
      "public testnet inclusion, stability, finality, context, and economics reports"
    ]
  }' >"$SUMMARY_PATH"

printf '\nPrepared rehearsal artefacts: %s\n' "$REHEARSAL_DIR" >&2
printf 'Summary: %s\n' "$SUMMARY_PATH" >&2
