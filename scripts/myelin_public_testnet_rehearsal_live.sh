#!/usr/bin/env bash
# Run Myelin public-testnet rehearsal Phases 4-5 for carrier submissions.
#
# This is not a release gate. It consumes a rehearsal directory prepared by
# scripts/myelin_public_testnet_rehearsal_prepare.sh plus real public CKB
# testnet inputs, then submits and observes selected carrier transactions.

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
REHEARSAL_DIR="${MYELIN_REHEARSAL_DIR:-}"
SUMMARY_PATH="${SUMMARY_PATH:-}"
ROLES="${MYELIN_REHEARSAL_ROLES:-da-anchor}"
MIN_STATUS="${CKB_TESTNET_MIN_STATUS:-committed}"
MIN_CONFIRMATIONS="${CKB_TESTNET_MIN_CONFIRMATIONS:-6}"
MIN_FEE_SHANNONS="${CKB_TESTNET_MIN_FEE_SHANNONS:-1}"
MIN_FEE_RATE_SHANNONS_PER_KB="${CKB_TESTNET_MIN_FEE_RATE_SHANNONS_PER_KB:-1000}"
MAX_FEE_SHANNONS="${CKB_TESTNET_MAX_FEE_SHANNONS:-100000}"

usage() {
  cat >&2 <<'EOF'
Usage:
  MYELIN_REHEARSAL_DIR=/path/to/rehearsal \
  MYELIN_REHEARSAL_LIVE_SUBMIT=1 \
  CKB_TESTNET_RPC=https://... \
  scripts/myelin_public_testnet_rehearsal_live.sh

Common role selection:
  MYELIN_REHEARSAL_ROLES="da-anchor"
  MYELIN_REHEARSAL_ROLES="da-anchor settlement"

This script submits live public-testnet transactions. It refuses to run unless
MYELIN_REHEARSAL_LIVE_SUBMIT=1 is set.
EOF
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing required command: $1" >&2
    exit 1
  }
}

require_env() {
  local name="$1"
  if [[ -z "${!name:-}" ]]; then
    echo "missing required environment variable: ${name}" >&2
    exit 1
  fi
}

require_file() {
  local path="$1"
  if [[ ! -f "$path" ]]; then
    echo "missing required file: ${path}" >&2
    exit 1
  fi
}

env_or() {
  local primary="$1"
  local fallback="$2"
  local value="${!primary:-}"
  if [[ -z "$value" ]]; then
    value="${!fallback:-}"
  fi
  printf '%s' "$value"
}

require_value() {
  local label="$1"
  local value="$2"
  if [[ -z "$value" ]]; then
    echo "missing required value: ${label}" >&2
    exit 1
  fi
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

role_config() {
  local role="$1"
  case "$role" in
    da-anchor)
      role_slug="da-anchor"
      package_path="$REHEARSAL_DIR/session-da-anchor-package.json"
      verifier_source="$REHEARSAL_DIR/da-anchor-carrier.cell"
      verifier_role="${CKB_TESTNET_DA_VERIFIER_ROLE:-carrier}"
      verifier_code_hash="${CKB_TESTNET_DA_VERIFIER_CODE_HASH:-}"
      verifier_dep_tx_hash="${CKB_TESTNET_DA_VERIFIER_DEP_TX_HASH:-}"
      verifier_dep_index="${CKB_TESTNET_DA_VERIFIER_DEP_INDEX:-}"
      witness="${CKB_TESTNET_DA_WITNESS:-}"
      input_tx_hash="$(env_or CKB_TESTNET_DA_INPUT_TX_HASH CKB_TESTNET_FUNDING_TX_HASH)"
      input_index="$(env_or CKB_TESTNET_DA_INPUT_INDEX CKB_TESTNET_FUNDING_INDEX)"
      input_capacity="$(env_or CKB_TESTNET_DA_INPUT_CAPACITY_SHANNONS CKB_TESTNET_FUNDING_CAPACITY_SHANNONS)"
      carrier_capacity="$(env_or CKB_TESTNET_DA_CARRIER_CAPACITY_SHANNONS CKB_TESTNET_CARRIER_CAPACITY_SHANNONS)"
      fee_shannons="$(env_or CKB_TESTNET_DA_FEE_SHANNONS CKB_TESTNET_FEE_SHANNONS)"
      ;;
    settlement)
      role_slug="settlement"
      package_path="$REHEARSAL_DIR/session-settlement-package.json"
      verifier_source="$REHEARSAL_DIR/settlement-carrier.cell"
      verifier_role="${CKB_TESTNET_SETTLEMENT_VERIFIER_ROLE:-carrier}"
      verifier_code_hash="${CKB_TESTNET_SETTLEMENT_VERIFIER_CODE_HASH:-}"
      verifier_dep_tx_hash="${CKB_TESTNET_SETTLEMENT_VERIFIER_DEP_TX_HASH:-}"
      verifier_dep_index="${CKB_TESTNET_SETTLEMENT_VERIFIER_DEP_INDEX:-}"
      witness="${CKB_TESTNET_SETTLEMENT_WITNESS:-}"
      input_tx_hash="${CKB_TESTNET_SETTLEMENT_INPUT_TX_HASH:-}"
      input_index="${CKB_TESTNET_SETTLEMENT_INPUT_INDEX:-}"
      input_capacity="${CKB_TESTNET_SETTLEMENT_INPUT_CAPACITY_SHANNONS:-}"
      carrier_capacity="${CKB_TESTNET_SETTLEMENT_CARRIER_CAPACITY_SHANNONS:-${CKB_TESTNET_CARRIER_CAPACITY_SHANNONS:-}}"
      fee_shannons="${CKB_TESTNET_SETTLEMENT_FEE_SHANNONS:-${CKB_TESTNET_FEE_SHANNONS:-}}"
      ;;
    *)
      echo "unsupported rehearsal role: ${role}; expected da-anchor or settlement" >&2
      exit 1
      ;;
  esac
}

run_role() {
  local role="$1"
  role_config "$role"

  require_file "$package_path"
  require_file "$verifier_source"
  require_value "${role}_input_tx_hash" "$input_tx_hash"
  require_value "${role}_input_index" "$input_index"
  require_value "${role}_input_capacity_shannons" "$input_capacity"
  require_value "${role}_carrier_capacity_shannons" "$carrier_capacity"
  require_value "${role}_fee_shannons" "$fee_shannons"
  require_value "${role}_verifier_code_hash" "$verifier_code_hash"
  require_value "${role}_verifier_dep_tx_hash" "$verifier_dep_tx_hash"
  require_value "${role}_verifier_dep_index" "$verifier_dep_index"
  require_value "${role}_witness" "$witness"

  local submission="$REHEARSAL_DIR/${role_slug}-carrier-submission.json"
  local context="$REHEARSAL_DIR/${role_slug}-context.json"
  local economics="$REHEARSAL_DIR/${role_slug}-economics.json"
  local inclusion="$REHEARSAL_DIR/${role_slug}-inclusion.json"
  local stability="$REHEARSAL_DIR/${role_slug}-stability.json"
  local finality="$REHEARSAL_DIR/${role_slug}-finality.json"
  local readiness="$REHEARSAL_DIR/${role_slug}-readiness.json"

  run_step "Phase 4: submit ${role_slug} carrier to public testnet" \
    myelin session carrier-submission \
      --package "$package_path" \
      --input-tx-hash "$input_tx_hash" \
      --input-index "$input_index" \
      --input-capacity-shannons "$input_capacity" \
      --carrier-capacity-shannons "$carrier_capacity" \
      --fee-shannons "$fee_shannons" \
      --lock-code-hash "$CKB_TESTNET_LOCK_CODE_HASH" \
      --lock-hash-type "${CKB_TESTNET_LOCK_HASH_TYPE:-type}" \
      --lock-args "${CKB_TESTNET_LOCK_ARGS:-0x}" \
      --lock-code-dep-tx-hash "$CKB_TESTNET_LOCK_DEP_TX_HASH" \
      --lock-code-dep-index "$CKB_TESTNET_LOCK_DEP_INDEX" \
      --verifier-code-hash "$verifier_code_hash" \
      --verifier-code-dep-tx-hash "$verifier_dep_tx_hash" \
      --verifier-code-dep-index "$verifier_dep_index" \
      --verifier-source "$verifier_source" \
      --verifier-role "$verifier_role" \
      --witness "$witness" \
      --rpc-url "$CKB_TESTNET_RPC" \
      --submit \
      --require-accepted \
      --out "$submission"

  run_step "Phase 5: observe ${role_slug} context" \
    myelin session verify-submission-context \
      --submission "$submission" \
      --rpc-url "$CKB_TESTNET_RPC" \
      --out "$context"
  run_step "Phase 5: observe ${role_slug} economics" \
    myelin session verify-submission-economics \
      --submission "$submission" \
      --rpc-url "$CKB_TESTNET_RPC" \
      --min-fee-shannons "$MIN_FEE_SHANNONS" \
      --min-fee-rate-shannons-per-kb "$MIN_FEE_RATE_SHANNONS_PER_KB" \
      --max-fee-shannons "$MAX_FEE_SHANNONS" \
      --out "$economics"
  run_step "Phase 5: observe ${role_slug} inclusion" \
    myelin session verify-submission-inclusion \
      --submission "$submission" \
      --rpc-url "$CKB_TESTNET_RPC" \
      --min-status "$MIN_STATUS" \
      --out "$inclusion"
  run_step "Phase 5: observe ${role_slug} stability" \
    myelin session verify-submission-stability \
      --inclusion "$inclusion" \
      --rpc-url "$CKB_TESTNET_RPC" \
      --out "$stability"
  run_step "Phase 5: observe ${role_slug} finality" \
    myelin session verify-submission-finality \
      --inclusion "$inclusion" \
      --rpc-url "$CKB_TESTNET_RPC" \
      --min-confirmations "$MIN_CONFIRMATIONS" \
      --out "$finality"
  run_step "Phase 5: aggregate ${role_slug} readiness" \
    myelin session verify-submission-readiness \
      --context "$context" \
      --economics "$economics" \
      --inclusion "$inclusion" \
      --stability "$stability" \
      --finality "$finality" \
      --require-live-submission \
      --operator-custody-policy "$REHEARSAL_DIR/operator-custody-policy.json" \
      --operator-runbook "$REHEARSAL_DIR/operator-runbook.json" \
      --out "$readiness"

  local tmp_summary="${SUMMARY_PATH}.tmp"
  jq \
    --arg role "$role_slug" \
    --slurpfile submission "$submission" \
    --slurpfile readiness "$readiness" \
    '.roles += [{
      role: $role,
      submission: $submission[0],
      readiness: $readiness[0]
    }]' "$SUMMARY_PATH" >"$tmp_summary"
  mv "$tmp_summary" "$SUMMARY_PATH"
}

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  usage
  exit 0
fi

if [[ "${MYELIN_REHEARSAL_LIVE_SUBMIT:-0}" != "1" ]]; then
  echo "refusing live submission; set MYELIN_REHEARSAL_LIVE_SUBMIT=1 to broadcast public-testnet transactions" >&2
  exit 1
fi

require_cmd jq
require_env CKB_TESTNET_RPC
require_env CKB_TESTNET_LOCK_CODE_HASH
require_env CKB_TESTNET_LOCK_DEP_TX_HASH
require_env CKB_TESTNET_LOCK_DEP_INDEX

if [[ -z "$REHEARSAL_DIR" ]]; then
  echo "missing required environment variable: MYELIN_REHEARSAL_DIR" >&2
  exit 1
fi
if [[ ! -d "$REHEARSAL_DIR" ]]; then
  echo "rehearsal directory does not exist: ${REHEARSAL_DIR}" >&2
  exit 1
fi
require_file "$REHEARSAL_DIR/operator-custody-policy.json"
require_file "$REHEARSAL_DIR/operator-runbook.json"

cd "$ROOT"
SUMMARY_PATH="${SUMMARY_PATH:-"${REHEARSAL_DIR}/public-testnet-live-summary.json"}"
jq -n \
  --arg schema "myelin-public-testnet-live-rehearsal-summary-v1" \
  --arg dir "$REHEARSAL_DIR" \
  --arg rpc "$CKB_TESTNET_RPC" \
  --arg min_status "$MIN_STATUS" \
  --argjson min_confirmations "$MIN_CONFIRMATIONS" \
  '{
    schema: $schema,
    rehearsal_dir: $dir,
    rpc_url: $rpc,
    min_status: $min_status,
    min_confirmations: $min_confirmations,
    public_testnet_submitted: true,
    roles: []
  }' >"$SUMMARY_PATH"

for role in $ROLES; do
  run_role "$role"
done

printf '\nPublic-testnet live rehearsal summary: %s\n' "$SUMMARY_PATH" >&2
