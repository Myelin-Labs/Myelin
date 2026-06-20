#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CKB_ROOT="${CKB_ROOT:-"$ROOT/../ckb"}"
CKB_BIN="${CKB_BIN:-"$CKB_ROOT/target/debug/ckb"}"
RPC_PORT="${RPC_PORT:-18314}"
P2P_PORT="${P2P_PORT:-18315}"
RPC_URL="http://127.0.0.1:${RPC_PORT}"
WORKDIR="${WORKDIR:-$(mktemp -d /tmp/myelin-ckb-devnet.XXXXXX)}"
REPORT="${REPORT:-"$WORKDIR/myelin-ckb-devnet-smoke.json"}"
ALWAYS_SUCCESS_CODE_HASH="0x28e83a1277d48add8e72fadaa9248559e1b632bab2bd60b27955ebc4c03800a5"
GENESIS_ALWAYS_SUCCESS_DEP_INDEX="${GENESIS_ALWAYS_SUCCESS_DEP_INDEX:-0x5}"
INITIAL_MINING_BLOCKS="${INITIAL_MINING_BLOCKS:-48}"
COMMIT_MINING_BLOCKS="${COMMIT_MINING_BLOCKS:-8}"
FEE_SHANNONS="${FEE_SHANNONS:-2000}"
DEPLOY_FEE_SHANNONS="${DEPLOY_FEE_SHANNONS:-100000}"
MIN_FUNDING_CELL_CAPACITY_SHANNONS="${MIN_FUNDING_CELL_CAPACITY_SHANNONS:-10000000000}"
CARRIER_CELL_CAPACITY_SHANNONS="${CARRIER_CELL_CAPACITY_SHANNONS:-40000000000}"
SETTLEMENT_AUTHORITY_CELL_CAPACITY_SHANNONS="${SETTLEMENT_AUTHORITY_CELL_CAPACITY_SHANNONS:-30000000000}"
CKB_SHANNONS_PER_BYTE=100000000

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing required command: $1" >&2
    exit 1
  }
}

rpc() {
  curl -fsS -H "content-type: application/json" -d "$1" "$RPC_URL"
}

wait_for_rpc() {
  for _ in $(seq 1 60); do
    if rpc '{"id":1,"jsonrpc":"2.0","method":"get_tip_header","params":[]}' >/dev/null 2>&1; then
      return 0
    fi
    sleep 1
  done
  echo "CKB RPC did not become ready at $RPC_URL" >&2
  return 1
}

mine() {
  local limit="$1"
  local label="${2:-$1}"
  "$CKB_BIN" -C "$WORKDIR" miner --limit "$limit" >"$WORKDIR/ckb-miner-$label.log" 2>&1
}

cleanup() {
  if [[ -n "${CKB_PID:-}" ]] && kill -0 "$CKB_PID" >/dev/null 2>&1; then
    kill "$CKB_PID" >/dev/null 2>&1 || true
    wait "$CKB_PID" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

require_cmd curl
require_cmd jq
require_cmd python3

if [[ ! -x "$CKB_BIN" ]]; then
  echo "CKB binary not found or not executable: $CKB_BIN" >&2
  exit 1
fi
if [[ ! -f "$CKB_ROOT/test/template/specs/integration.toml" ]]; then
  echo "CKB integration dev spec not found under $CKB_ROOT" >&2
  exit 1
fi

mkdir -p "$WORKDIR/myelin" "$WORKDIR/specs/cells"

file_hex() {
  od -An -tx1 -v "$1" | tr -d ' \n'
}

ckb_hash_hex() {
  local hex_value="${1#0x}"
  echo "0x$(cargo run -q --manifest-path "$ROOT/cellscript/Cargo.toml" --bin cellc -- ckb-hash --hex "$hex_value" --json | jq -r '.hash')"
}

carrier_identity_hex() {
  local payload="${1#0x}"
  echo "0x${payload:0:64}"
}

carrier_type_args_hex() {
  local payload_hash="$1"
  local identity="${2#0x}"
  echo "${payload_hash}${identity}"
}

entry_witness_hex() {
  local source_path="$1"
  local action="$2"
  shift 2
  local witness_hex
  local cmd=(
    cargo run -q --manifest-path "$ROOT/cellscript/Cargo.toml" --bin cellc --
    entry-witness "$source_path"
    --action "$action"
  )
  local arg_hex
  for arg_hex in "$@"; do
    cmd+=(--arg "$arg_hex")
  done
  cmd+=(--target-profile ckb --json)
  witness_hex="$("${cmd[@]}" | jq -r '.witness_hex')"
  echo "0x${witness_hex#0x}"
}

compile_carrier_verifiers() {
  cp "$ROOT/cellscript/examples/myelin/da-anchor-carrier.cell" "$WORKDIR/myelin/da-anchor-carrier.cell"
  cp "$ROOT/cellscript/examples/myelin/settlement-carrier.cell" "$WORKDIR/myelin/settlement-carrier.cell"
  cp "$ROOT/cellscript/examples/myelin/da-anchor-final.cell" "$WORKDIR/myelin/da-anchor-final.cell"
  cp "$ROOT/cellscript/examples/myelin/settlement-final.cell" "$WORKDIR/myelin/settlement-final.cell"
  cargo run -q --manifest-path "$ROOT/cellscript/Cargo.toml" --bin cellc -- \
    "$WORKDIR/myelin/da-anchor-carrier.cell" \
    -t riscv64-elf \
    --target-profile typed-cell \
    --primitive-compat 0.18 \
    --entry-action verify_da_anchor_carrier \
    -o "$WORKDIR/myelin/da-anchor-carrier.typed-cell.elf"
  cargo run -q --manifest-path "$ROOT/cellscript/Cargo.toml" --bin cellc -- \
    "$WORKDIR/myelin/settlement-carrier.cell" \
    -t riscv64-elf \
    --target-profile typed-cell \
    --primitive-compat 0.18 \
    --entry-action verify_settlement_carrier \
    -o "$WORKDIR/myelin/settlement-carrier.typed-cell.elf"
  cargo run -q --manifest-path "$ROOT/cellscript/Cargo.toml" --bin cellc -- \
    "$WORKDIR/myelin/da-anchor-final.cell" \
    -t riscv64-elf \
    --target-profile typed-cell \
    --primitive-compat 0.18 \
    --entry-action verify_final_da_publication \
    -o "$WORKDIR/myelin/da-anchor-final.typed-cell.elf"
  cargo run -q --manifest-path "$ROOT/cellscript/Cargo.toml" --bin cellc -- \
    "$WORKDIR/myelin/settlement-final.cell" \
    -t riscv64-elf \
    --target-profile typed-cell \
    --primitive-compat 0.18 \
    --entry-action verify_final_settlement \
    -o "$WORKDIR/myelin/settlement-final.typed-cell.elf"
  cargo run -q --manifest-path "$ROOT/cellscript/Cargo.toml" --bin cellc -- \
    "$WORKDIR/myelin/da-anchor-carrier.cell" \
    -t riscv64-elf \
    --target-profile ckb \
    --primitive-compat 0.18 \
    --entry-action verify_da_anchor_carrier \
    -o "$WORKDIR/myelin/da-anchor-carrier.elf"
  cargo run -q --manifest-path "$ROOT/cellscript/Cargo.toml" --bin cellc -- \
    "$WORKDIR/myelin/settlement-carrier.cell" \
    -t riscv64-elf \
    --target-profile ckb \
    --primitive-compat 0.18 \
    --entry-action verify_settlement_carrier \
    -o "$WORKDIR/myelin/settlement-carrier.elf"
  cargo run -q --manifest-path "$ROOT/cellscript/Cargo.toml" --bin cellc -- \
    "$WORKDIR/myelin/da-anchor-final.cell" \
    -t riscv64-elf \
    --target-profile ckb \
    --primitive-compat 0.18 \
    --entry-action verify_final_da_publication \
    -o "$WORKDIR/myelin/da-anchor-final.elf"
  cargo run -q --manifest-path "$ROOT/cellscript/Cargo.toml" --bin cellc -- \
    "$WORKDIR/myelin/settlement-final.cell" \
    -t riscv64-elf \
    --target-profile ckb \
    --primitive-compat 0.18 \
    --entry-action verify_final_settlement \
    -o "$WORKDIR/myelin/settlement-final.elf"

  da_verifier_code_hash="0x$(cargo run -q --manifest-path "$ROOT/cellscript/Cargo.toml" --bin cellc -- \
    ckb-hash --file "$WORKDIR/myelin/da-anchor-carrier.elf" --json | jq -r '.hash')"
  da_verifier_elf_hex="0x$(file_hex "$WORKDIR/myelin/da-anchor-carrier.elf")"
  da_verifier_elf_size="$(wc -c <"$WORKDIR/myelin/da-anchor-carrier.elf" | tr -d ' ')"
  da_verifier_code_capacity="$(((da_verifier_elf_size + 1000) * CKB_SHANNONS_PER_BYTE))"
  da_verifier_code_capacity_hex="$(printf '0x%x' "$da_verifier_code_capacity")"

  settlement_verifier_code_hash="0x$(cargo run -q --manifest-path "$ROOT/cellscript/Cargo.toml" --bin cellc -- \
    ckb-hash --file "$WORKDIR/myelin/settlement-carrier.elf" --json | jq -r '.hash')"
  settlement_verifier_elf_hex="0x$(file_hex "$WORKDIR/myelin/settlement-carrier.elf")"
  settlement_verifier_elf_size="$(wc -c <"$WORKDIR/myelin/settlement-carrier.elf" | tr -d ' ')"
  settlement_verifier_code_capacity="$(((settlement_verifier_elf_size + 1000) * CKB_SHANNONS_PER_BYTE))"
  settlement_verifier_code_capacity_hex="$(printf '0x%x' "$settlement_verifier_code_capacity")"

  da_final_verifier_code_hash="0x$(cargo run -q --manifest-path "$ROOT/cellscript/Cargo.toml" --bin cellc -- \
    ckb-hash --file "$WORKDIR/myelin/da-anchor-final.elf" --json | jq -r '.hash')"
  da_final_verifier_elf_hex="0x$(file_hex "$WORKDIR/myelin/da-anchor-final.elf")"
  da_final_verifier_elf_size="$(wc -c <"$WORKDIR/myelin/da-anchor-final.elf" | tr -d ' ')"
  da_final_verifier_code_capacity="$(((da_final_verifier_elf_size + 1000) * CKB_SHANNONS_PER_BYTE))"
  da_final_verifier_code_capacity_hex="$(printf '0x%x' "$da_final_verifier_code_capacity")"

  settlement_final_verifier_code_hash="0x$(cargo run -q --manifest-path "$ROOT/cellscript/Cargo.toml" --bin cellc -- \
    ckb-hash --file "$WORKDIR/myelin/settlement-final.elf" --json | jq -r '.hash')"
  settlement_final_verifier_elf_hex="0x$(file_hex "$WORKDIR/myelin/settlement-final.elf")"
  settlement_final_verifier_elf_size="$(wc -c <"$WORKDIR/myelin/settlement-final.elf" | tr -d ' ')"
  settlement_final_verifier_code_capacity="$(((settlement_final_verifier_elf_size + 1000) * CKB_SHANNONS_PER_BYTE))"
  settlement_final_verifier_code_capacity_hex="$(printf '0x%x' "$settlement_final_verifier_code_capacity")"

  total_verifier_code_capacity="$((da_verifier_code_capacity + settlement_verifier_code_capacity + da_final_verifier_code_capacity + settlement_final_verifier_code_capacity))"
}


cargo run -q -p myelin-cli -- session open-fixture \
  --consensus static-closed-committee \
  --out "$WORKDIR/myelin/session-open.json"
cargo run -q -p myelin-cli -- session commit-fixture \
  --session "$WORKDIR/myelin/session-open.json" \
  --out "$WORKDIR/myelin/session-commit.json"
cargo run -q -p myelin-cli -- session court-bundle \
  --commit "$WORKDIR/myelin/session-commit.json" \
  --chunk-index 0 \
  --out "$WORKDIR/myelin/session-court.json"
cargo run -q -p myelin-cli -- session da-manifest \
  --bundle "$WORKDIR/myelin/session-court.json" \
  --storage-dir "$WORKDIR/myelin/session-da-store" \
  --out "$WORKDIR/myelin/session-da.json"
cargo run -q -p myelin-cli -- session da-anchor-package \
  --manifest "$WORKDIR/myelin/session-da.json" \
  --bundle "$WORKDIR/myelin/session-court.json" \
  --out "$WORKDIR/myelin/session-da-anchor.json"
cargo run -q -p myelin-cli -- session verify-da-anchor-package \
  --package "$WORKDIR/myelin/session-da-anchor.json" \
  --manifest "$WORKDIR/myelin/session-da.json" \
  --bundle "$WORKDIR/myelin/session-court.json" \
  --out "$WORKDIR/myelin/session-da-anchor-verify.json"
cargo run -q -p myelin-cli -- session settlement-intent \
  --bundle "$WORKDIR/myelin/session-court.json" \
  --da-manifest "$WORKDIR/myelin/session-da.json" \
  --kind disputed-close \
  --current-time-ms 60000 \
  --challenge-window-ms 60000 \
  --out "$WORKDIR/myelin/session-settlement.json"
cargo run -q -p myelin-cli -- session verify-settlement-intent \
  --intent "$WORKDIR/myelin/session-settlement.json" \
  --bundle "$WORKDIR/myelin/session-court.json" \
  --da-manifest "$WORKDIR/myelin/session-da.json" \
  --out "$WORKDIR/myelin/session-settlement-verify.json"
cargo run -q -p myelin-cli -- session settlement-package \
  --intent "$WORKDIR/myelin/session-settlement.json" \
  --bundle "$WORKDIR/myelin/session-court.json" \
  --da-manifest "$WORKDIR/myelin/session-da.json" \
  --out "$WORKDIR/myelin/session-settlement-package.json"
cargo run -q -p myelin-cli -- session verify-settlement-package \
  --package "$WORKDIR/myelin/session-settlement-package.json" \
  --intent "$WORKDIR/myelin/session-settlement.json" \
  --bundle "$WORKDIR/myelin/session-court.json" \
  --da-manifest "$WORKDIR/myelin/session-da.json" \
  --out "$WORKDIR/myelin/session-settlement-package-verify.json"

anchor_valid="$(jq -r '.valid' "$WORKDIR/myelin/session-da-anchor-verify.json")"
if [[ "$anchor_valid" != "true" ]]; then
  echo "generated DA anchor package did not verify" >&2
  exit 1
fi
settlement_valid="$(jq -r '.valid' "$WORKDIR/myelin/session-settlement-package-verify.json")"
if [[ "$settlement_valid" != "true" ]]; then
  echo "generated settlement package did not verify" >&2
  exit 1
fi
court_economics_mode="$(jq -r '.court_economics.mode' "$WORKDIR/myelin/session-settlement.json")"
court_economics_min_bond="$(jq -r '.court_economics.minimum_dispute_bond_shannons' "$WORKDIR/myelin/session-settlement.json")"
court_economics_challenger_reward_bps="$(jq -r '.court_economics.challenger_reward_bps' "$WORKDIR/myelin/session-settlement.json")"
court_economics_loser_slash_bps="$(jq -r '.court_economics.loser_slash_bps' "$WORKDIR/myelin/session-settlement.json")"
court_economics_honest_refund_bps="$(jq -r '.court_economics.honest_party_refund_bps' "$WORKDIR/myelin/session-settlement.json")"
court_economics_unresolved_remainder_bps="$(jq -r '.court_economics.unresolved_remainder_bps' "$WORKDIR/myelin/session-settlement.json")"
court_economics_payout_balance_bps="$(jq -r '.court_economics.payout_balance_bps' "$WORKDIR/myelin/session-settlement.json")"
court_economics_deadline_only="$(jq -r '.court_economics.settlement_after_deadline_only' "$WORKDIR/myelin/session-settlement.json")"
court_economics_da_required="$(jq -r '.court_economics.da_evidence_required' "$WORKDIR/myelin/session-settlement.json")"
court_economics_invariant_checked="$(jq -r '.court_economics.economics_invariant_checked' "$WORKDIR/myelin/session-settlement.json")"
court_economics_checked="$(jq -r '.court_economics.court_economics_checked' "$WORKDIR/myelin/session-settlement.json")"
court_economics_testnet_ready="$(jq -r '.court_economics.testnet_beta_ready' "$WORKDIR/myelin/session-settlement.json")"
court_economics_production_ready="$(jq -r '.court_economics.production_ready' "$WORKDIR/myelin/session-settlement.json")"
if [[ "$court_economics_mode" != "disputed-close-explicit-policy-v1" ]]; then
  echo "court economics must expose the explicit disputed-close policy" >&2
  exit 1
fi
if (( court_economics_min_bond != 100000000 || court_economics_challenger_reward_bps != 5000 || court_economics_loser_slash_bps != 10000 || court_economics_honest_refund_bps != 5000 || court_economics_unresolved_remainder_bps != 0 )); then
  echo "court economics numeric policy does not match the expected disputed-close terms" >&2
  exit 1
fi
if (( court_economics_payout_balance_bps != court_economics_loser_slash_bps || court_economics_challenger_reward_bps + court_economics_honest_refund_bps + court_economics_unresolved_remainder_bps != court_economics_payout_balance_bps )); then
  echo "court economics payout basis points must balance" >&2
  exit 1
fi
if [[ "$court_economics_deadline_only" != "true" || "$court_economics_da_required" != "true" || "$court_economics_invariant_checked" != "true" || "$court_economics_checked" != "true" ]]; then
  echo "court economics invariant checks must be explicit and true" >&2
  exit 1
fi
if [[ "$court_economics_testnet_ready" != "false" || "$court_economics_production_ready" != "false" ]]; then
  echo "court economics must remain non-testnet and non-production until CKB court enforcement exists" >&2
  exit 1
fi
da_availability_schema="$(jq -r '.availability.schema' "$WORKDIR/myelin/session-da.json")"
da_availability_mode="$(jq -r '.availability.mode' "$WORKDIR/myelin/session-da.json")"
da_availability_signature_scheme="$(jq -r '.availability.signature_scheme' "$WORKDIR/myelin/session-da.json")"
da_availability_checked="$(jq -r '.availability.availability_checked' "$WORKDIR/myelin/session-da.json")"
da_availability_testnet_ready="$(jq -r '.availability.testnet_beta_ready' "$WORKDIR/myelin/session-da.json")"
da_availability_production_ready="$(jq -r '.availability.production_ready' "$WORKDIR/myelin/session-da.json")"
da_availability_required_attestations="$(jq -r '.availability.required_attestations' "$WORKDIR/myelin/session-da.json")"
da_availability_pubkey_hash_count="$(jq -r '.availability.attester_pubkey_hashes | length' "$WORKDIR/myelin/session-da.json")"
da_availability_signature_count="$(jq -r '.availability.attestation_signatures | length' "$WORKDIR/myelin/session-da.json")"
da_availability_signature_verified="$(jq -r '.availability.attestation_signature_verified' "$WORKDIR/myelin/session-da.json")"
da_availability_payload_hash="$(jq -r '.availability.payload_hash' "$WORKDIR/myelin/session-da.json")"
da_availability_segment_root="$(jq -r '.availability.segment_root' "$WORKDIR/myelin/session-da.json")"
da_availability_commitment="$(jq -r '.availability.availability_commitment' "$WORKDIR/myelin/session-da.json")"
da_manifest_molecule_hash="$(jq -r '.molecule_transaction_hash' "$WORKDIR/myelin/session-da.json")"
da_manifest_segment_root="$(jq -r '.segment_root' "$WORKDIR/myelin/session-da.json")"
if [[ "$da_availability_schema" != "myelin-da-availability-v1" || "$da_availability_mode" != "replicated-da-committee" ]]; then
  echo "DA manifest did not expose replicated DA availability evidence" >&2
  exit 1
fi
if [[ "$da_availability_signature_scheme" != "secp256k1-recoverable-blake3-pubkey-hash20" ]]; then
  echo "DA availability evidence must expose the secp256k1 recoverable signature scheme" >&2
  exit 1
fi
if (( da_availability_pubkey_hash_count < da_availability_required_attestations || da_availability_signature_count < da_availability_required_attestations )); then
  echo "DA availability evidence must expose threshold pubkey hashes and signatures" >&2
  exit 1
fi
if [[ "$da_availability_signature_verified" != "true" ]]; then
  echo "DA availability signatures must verify locally" >&2
  exit 1
fi
if [[ "$da_availability_checked" != "true" || "$da_availability_testnet_ready" != "false" || "$da_availability_production_ready" != "false" ]]; then
  echo "DA availability evidence must be checked while explicitly remaining non-testnet and non-production" >&2
  exit 1
fi
if [[ "$da_availability_payload_hash" != "$da_manifest_molecule_hash" || "$da_availability_segment_root" != "$da_manifest_segment_root" ]]; then
  echo "DA availability evidence must bind the manifest payload hash and segment root" >&2
  exit 1
fi
if [[ "$da_availability_commitment" == "null" || ${#da_availability_commitment} -ne 64 ]]; then
  echo "DA availability evidence must expose a 32-byte availability commitment" >&2
  exit 1
fi
compile_carrier_verifiers

settlement_authority_data="$(jq -r '.settlement_authority.data' "$WORKDIR/myelin/session-settlement-package.json")"
settlement_authority_data_hash="$(jq -r '.settlement_authority.data_hash' "$WORKDIR/myelin/session-settlement-package.json")"
settlement_package_session_id="$(jq -r '.session_id' "$WORKDIR/myelin/session-settlement-package.json")"
settlement_package_participant_set_hash="$(jq -r '.participant_set_hash' "$WORKDIR/myelin/session-settlement-package.json")"
settlement_package_escrow_input_cells_hash="$(jq -r '.escrow_input_cells_hash' "$WORKDIR/myelin/session-settlement-package.json")"
settlement_package_session_lineage_commitment="$(jq -r '.session_lineage_commitment' "$WORKDIR/myelin/session-settlement-package.json")"
settlement_authority_session_id="$(jq -r '.settlement_authority.session_id' "$WORKDIR/myelin/session-settlement-package.json")"
settlement_authority_participant_set_hash="$(jq -r '.settlement_authority.participant_set_hash' "$WORKDIR/myelin/session-settlement-package.json")"
settlement_authority_escrow_input_cells_hash="$(jq -r '.settlement_authority.escrow_input_cells_hash' "$WORKDIR/myelin/session-settlement-package.json")"
settlement_authority_session_lineage_commitment="$(jq -r '.settlement_authority.session_lineage_commitment' "$WORKDIR/myelin/session-settlement-package.json")"
settlement_authority_session_binding="$(jq -r '.settlement_authority.session_binding' "$WORKDIR/myelin/session-settlement-package.json")"
settlement_authority_lineage_commitment="$(jq -r '.settlement_authority.session_authority_commitment' "$WORKDIR/myelin/session-settlement-package.json")"
settlement_authority_input_index="$(jq -r '.settlement_authority.consumed_input_index' "$WORKDIR/myelin/session-settlement-package.json")"
settlement_authority_lock_binding="$(jq -r '.settlement_authority.required_lock_binding' "$WORKDIR/myelin/session-settlement-package.json")"
settlement_authority_authentication="$(jq -c '.settlement_authority.authority_authentication' "$WORKDIR/myelin/session-settlement-package.json")"
settlement_authority_authentication_schema="$(jq -r '.settlement_authority.authority_authentication.schema' "$WORKDIR/myelin/session-settlement-package.json")"
settlement_authority_authentication_mode="$(jq -r '.settlement_authority.authority_authentication.mode' "$WORKDIR/myelin/session-settlement-package.json")"
settlement_authority_authentication_signature_scheme="$(jq -r '.settlement_authority.authority_authentication.signature_scheme' "$WORKDIR/myelin/session-settlement-package.json")"
settlement_authority_authentication_participant_set_hash="$(jq -r '.settlement_authority.authority_authentication.participant_set_hash' "$WORKDIR/myelin/session-settlement-package.json")"
settlement_authority_authentication_threshold="$(jq -r '.settlement_authority.authority_authentication.threshold' "$WORKDIR/myelin/session-settlement-package.json")"
settlement_authority_authentication_signer_count="$(jq -r '.settlement_authority.authority_authentication.signer_count' "$WORKDIR/myelin/session-settlement-package.json")"
settlement_authority_authentication_pubkey_hash_count="$(jq -r '.settlement_authority.authority_authentication.signer_pubkey_hashes | length' "$WORKDIR/myelin/session-settlement-package.json")"
settlement_authority_authentication_signature_count="$(jq -r '.settlement_authority.authority_authentication.signatures | length' "$WORKDIR/myelin/session-settlement-package.json")"
settlement_authority_authentication_signature_verified="$(jq -r '.settlement_authority.authority_authentication.signature_verified' "$WORKDIR/myelin/session-settlement-package.json")"
settlement_authority_authentication_hash="$(jq -r '.settlement_authority.authority_authentication.attestation_hash' "$WORKDIR/myelin/session-settlement-package.json")"
settlement_authority_authentication_enforceable="$(jq -r '.settlement_authority.authority_authentication.ckb_enforceable' "$WORKDIR/myelin/session-settlement-package.json")"
settlement_authority_authentication_testnet_ready="$(jq -r '.settlement_authority.authority_authentication.testnet_beta_ready' "$WORKDIR/myelin/session-settlement-package.json")"
settlement_authority_authentication_production_ready="$(jq -r '.settlement_authority.authority_authentication.production_ready' "$WORKDIR/myelin/session-settlement-package.json")"
if [[ "$settlement_authority_data" == "null" || ${#settlement_authority_data} -ne 386 ]]; then
  echo "settlement package did not expose a 192-byte settlement authority lineage data field" >&2
  exit 1
fi
if [[ "$settlement_authority_data_hash" == "null" || ${#settlement_authority_data_hash} -ne 66 ]]; then
  echo "settlement package did not expose a 32-byte settlement authority data hash" >&2
  exit 1
fi
if [[ "$settlement_authority_session_id" != "$settlement_package_session_id" || ${#settlement_authority_session_id} -ne 64 ]]; then
  echo "settlement package authority must bind to the package session id" >&2
  exit 1
fi
if [[ "$settlement_authority_participant_set_hash" != "$settlement_package_participant_set_hash" || ${#settlement_authority_participant_set_hash} -ne 64 ]]; then
  echo "settlement package authority must bind to the participant-set digest" >&2
  exit 1
fi
if [[ "$settlement_authority_escrow_input_cells_hash" != "$settlement_package_escrow_input_cells_hash" || ${#settlement_authority_escrow_input_cells_hash} -ne 64 ]]; then
  echo "settlement package authority must bind to the escrow input-cell digest" >&2
  exit 1
fi
if [[ "$settlement_authority_session_lineage_commitment" != "$settlement_package_session_lineage_commitment" || ${#settlement_authority_session_lineage_commitment} -ne 64 ]]; then
  echo "settlement package authority must bind to the session lineage commitment" >&2
  exit 1
fi
if [[ "$settlement_authority_session_binding" != "session-id-and-lineage-commit-participants-and-escrow" ]]; then
  echo "settlement package authority session binding must state participant/escrow lineage" >&2
  exit 1
fi
if [[ "$settlement_authority_lineage_commitment" == "null" || ${#settlement_authority_lineage_commitment} -ne 64 ]]; then
  echo "settlement package authority must expose a 32-byte session lineage commitment" >&2
  exit 1
fi
if [[ "$settlement_authority_input_index" != "1" ]]; then
  echo "settlement package authority must be consumed at input index 1" >&2
  exit 1
fi
if [[ "$settlement_authority_lock_binding" != "final-da-publication-lock-hash" ]]; then
  echo "settlement package authority lock binding must target the final DA publication lock" >&2
  exit 1
fi
if [[ "$settlement_authority_authentication_schema" != "myelin-session-settlement-authority-auth-v1" || "$settlement_authority_authentication_mode" != "ckb-threshold-lock" ]]; then
  echo "settlement authority must expose CKB threshold-lock authentication evidence" >&2
  exit 1
fi
if [[ "$settlement_authority_authentication_signature_scheme" != "secp256k1-recoverable-blake3-pubkey-hash20" ]]; then
  echo "settlement authority authentication must expose the secp256k1 recoverable signature scheme" >&2
  exit 1
fi
if [[ "$settlement_authority_authentication_participant_set_hash" != "$settlement_authority_participant_set_hash" ]]; then
  echo "settlement authority authentication must bind the authority participant-set digest" >&2
  exit 1
fi
if (( settlement_authority_authentication_signer_count < settlement_authority_authentication_threshold )); then
  echo "settlement authority authentication signer count must satisfy threshold" >&2
  exit 1
fi
if (( settlement_authority_authentication_pubkey_hash_count < settlement_authority_authentication_threshold || settlement_authority_authentication_signature_count < settlement_authority_authentication_threshold )); then
  echo "settlement authority authentication must expose threshold pubkey hashes and signatures" >&2
  exit 1
fi
if [[ "$settlement_authority_authentication_signature_verified" != "true" ]]; then
  echo "settlement authority authentication signatures must verify locally" >&2
  exit 1
fi
if [[ "$settlement_authority_authentication_hash" == "null" || ${#settlement_authority_authentication_hash} -ne 64 ]]; then
  echo "settlement authority authentication must expose a 32-byte attestation hash" >&2
  exit 1
fi
if [[ "$settlement_authority_authentication_enforceable" != "false" || "$settlement_authority_authentication_testnet_ready" != "false" || "$settlement_authority_authentication_production_ready" != "false" ]]; then
  echo "settlement authority authentication must remain commitment-only until participant-authenticated CKB enforcement exists" >&2
  exit 1
fi
settlement_authority_capacity="$SETTLEMENT_AUTHORITY_CELL_CAPACITY_SHANNONS"
settlement_authority_capacity_hex="$(printf '0x%x' "$settlement_authority_capacity")"

sed \
  -e "s|file = { file = \"cells/always_success\" }|file = { file = \"$CKB_ROOT/test/template/specs/cells/always_success\" }|" \
  -e "s|file = { file = \"cells/always_failure\" }|file = { file = \"$CKB_ROOT/test/template/specs/cells/always_failure\" }|" \
  "$CKB_ROOT/test/template/specs/integration.toml" >"$WORKDIR/import-spec.toml"

"$CKB_BIN" -C "$WORKDIR" init \
  --chain dev \
  --import-spec "$WORKDIR/import-spec.toml" \
  --rpc-port "$RPC_PORT" \
  --p2p-port "$P2P_PORT" \
  --ba-code-hash "$ALWAYS_SUCCESS_CODE_HASH" \
  --ba-hash-type data \
  --ba-arg 0x \
  --force \
  --log-to stdout >"$WORKDIR/ckb-init.log" 2>&1 || {
    cat "$WORKDIR/ckb-init.log" >&2
    exit 1
  }

cp "$CKB_ROOT/test/template/specs/cells/always_success" "$WORKDIR/specs/cells/always_success"
cp "$CKB_ROOT/test/template/specs/cells/always_failure" "$WORKDIR/specs/cells/always_failure"

"$CKB_BIN" -C "$WORKDIR" run --indexer --ba-advanced >"$WORKDIR/ckb-run.log" 2>&1 &
CKB_PID="$!"
wait_for_rpc

mine "$INITIAL_MINING_BLOCKS" "initial"

tip_hex="$(rpc '{"id":1,"jsonrpc":"2.0","method":"get_tip_header","params":[]}' | jq -r '.result.number')"
tip_number="$((tip_hex))"
required_reward_capacity="$((total_verifier_code_capacity + settlement_authority_capacity + DEPLOY_FEE_SHANNONS + MIN_FUNDING_CELL_CAPACITY_SHANNONS + (4 * CARRIER_CELL_CAPACITY_SHANNONS) + (4 * FEE_SHANNONS)))"
reward_cells=""
reward_capacity=0
for block_number in $(seq 1 "$tip_number"); do
  block_hex="$(printf '0x%x' "$block_number")"
  block_json="$(rpc "{\"id\":1,\"jsonrpc\":\"2.0\",\"method\":\"get_block_by_number\",\"params\":[\"$block_hex\"]}")"
  reward_row="$(
    jq -r --arg lock "$ALWAYS_SUCCESS_CODE_HASH" '
      .result.transactions[0] as $tx
      | select(($tx.outputs | length) > 0)
      | select($tx.outputs[0].lock.code_hash == $lock)
      | select($tx.outputs[0].lock.hash_type == "data")
      | select($tx.outputs[0].lock.args == "0x")
      | [$tx.hash, $tx.outputs[0].capacity]
      | @tsv
    ' <<<"$block_json"
  )"
  if [[ -n "$reward_row" ]]; then
    reward_cells+="$reward_row"$'\n'
    reward_capacity="$((reward_capacity + $(awk '{print $2}' <<<"$reward_row")))"
  fi
  if ((reward_capacity >= required_reward_capacity)); then
    break
  fi
done

if ((reward_capacity < required_reward_capacity)); then
  echo "could not find enough spendable always-success reward capacity for verifier deployment" >&2
  echo "required: $required_reward_capacity, found: $reward_capacity" >&2
  exit 1
fi

genesis_tx_hash="$(
  rpc '{"id":1,"jsonrpc":"2.0","method":"get_block_by_number","params":["0x0"]}' \
    | jq -r '.result.transactions[0].hash'
)"

reward_inputs_json="$(
  jq -Rn '
    [
      inputs
      | select(length > 0)
      | split("\t")
      | {
          previous_output: { tx_hash: .[0], index: "0x0" },
          since: "0x0"
        }
    ]
  ' <<<"$reward_cells"
)"
funding_capacity="$((reward_capacity - total_verifier_code_capacity - settlement_authority_capacity - DEPLOY_FEE_SHANNONS))"
funding_capacity_hex="$(printf '0x%x' "$funding_capacity")"
deploy_request_path="$WORKDIR/verifier-deploy-send-transaction.json"
deploy_response_path="$WORKDIR/verifier-deploy-send-transaction-response.json"
deploy_get_tx_path="$WORKDIR/verifier-deploy-get-transaction-response.json"

jq -n \
  --argjson inputs "$reward_inputs_json" \
  --arg funding_capacity "$funding_capacity_hex" \
  --arg da_verifier_code_capacity "$da_verifier_code_capacity_hex" \
  --arg settlement_verifier_code_capacity "$settlement_verifier_code_capacity_hex" \
  --arg da_final_verifier_code_capacity "$da_final_verifier_code_capacity_hex" \
  --arg settlement_final_verifier_code_capacity "$settlement_final_verifier_code_capacity_hex" \
  --arg settlement_authority_capacity "$settlement_authority_capacity_hex" \
  --arg da_verifier_elf "$da_verifier_elf_hex" \
  --arg settlement_verifier_elf "$settlement_verifier_elf_hex" \
  --arg da_final_verifier_elf "$da_final_verifier_elf_hex" \
  --arg settlement_final_verifier_elf "$settlement_final_verifier_elf_hex" \
  --arg settlement_authority_data "$settlement_authority_data" \
  --arg lock "$ALWAYS_SUCCESS_CODE_HASH" \
  --arg genesis_tx "$genesis_tx_hash" \
  --arg dep_index "$GENESIS_ALWAYS_SUCCESS_DEP_INDEX" \
  '{
    id: 1,
    jsonrpc: "2.0",
    method: "send_transaction",
    params: [
      {
        version: "0x0",
        cell_deps: [
          {
            out_point: { tx_hash: $genesis_tx, index: $dep_index },
            dep_type: "code"
          }
        ],
        header_deps: [],
        inputs: $inputs,
        outputs: [
          {
            capacity: $funding_capacity,
            lock: { code_hash: $lock, hash_type: "data", args: "0x" },
            type: null
          },
          {
            capacity: $da_verifier_code_capacity,
            lock: { code_hash: $lock, hash_type: "data", args: "0x" },
            type: null
          },
          {
            capacity: $settlement_verifier_code_capacity,
            lock: { code_hash: $lock, hash_type: "data", args: "0x" },
            type: null
          },
          {
            capacity: $da_final_verifier_code_capacity,
            lock: { code_hash: $lock, hash_type: "data", args: "0x" },
            type: null
          },
          {
            capacity: $settlement_final_verifier_code_capacity,
            lock: { code_hash: $lock, hash_type: "data", args: "0x" },
            type: null
          },
          {
            capacity: $settlement_authority_capacity,
            lock: { code_hash: $lock, hash_type: "data", args: "0x" },
            type: null
          }
        ],
        outputs_data: ["0x", $da_verifier_elf, $settlement_verifier_elf, $da_final_verifier_elf, $settlement_final_verifier_elf, $settlement_authority_data],
        witnesses: ($inputs | map("0x"))
      },
      "passthrough"
    ]
  }' >"$deploy_request_path"

deploy_response="$(curl -fsS -H "content-type: application/json" -d @"$deploy_request_path" "$RPC_URL")"
printf '%s\n' "$deploy_response" >"$deploy_response_path"
verifier_deploy_tx_hash="$(jq -r '.result // empty' <<<"$deploy_response")"
if [[ -z "$verifier_deploy_tx_hash" || "$verifier_deploy_tx_hash" == "null" ]]; then
  jq . "$deploy_response_path" >&2
  exit 1
fi

deploy_status="pending"
for attempt in $(seq 1 "$COMMIT_MINING_BLOCKS"); do
  mine 1 "verifier-deploy-$attempt"
  deploy_tx_response="$(rpc "{\"id\":1,\"jsonrpc\":\"2.0\",\"method\":\"get_transaction\",\"params\":[\"$verifier_deploy_tx_hash\"]}")"
  printf '%s\n' "$deploy_tx_response" >"$deploy_get_tx_path"
  deploy_status="$(jq -r '.result.tx_status.status // "unknown"' <<<"$deploy_tx_response")"
  if [[ "$deploy_status" == "committed" ]]; then
    break
  fi
done

if [[ "$deploy_status" != "committed" ]]; then
  jq . "$deploy_get_tx_path" >&2
  echo "verifier deployment transaction did not commit after $COMMIT_MINING_BLOCKS mined blocks" >&2
  exit 1
fi

committed_da_verifier_data="$(jq -r '.result.transaction.outputs_data[1]' "$deploy_get_tx_path")"
if [[ "$committed_da_verifier_data" != "$da_verifier_elf_hex" ]]; then
  echo "committed DA verifier code cell data does not match compiled CellScript ELF" >&2
  exit 1
fi
committed_settlement_verifier_data="$(jq -r '.result.transaction.outputs_data[2]' "$deploy_get_tx_path")"
if [[ "$committed_settlement_verifier_data" != "$settlement_verifier_elf_hex" ]]; then
  echo "committed settlement verifier code cell data does not match compiled CellScript ELF" >&2
  exit 1
fi
committed_da_final_verifier_data="$(jq -r '.result.transaction.outputs_data[3]' "$deploy_get_tx_path")"
if [[ "$committed_da_final_verifier_data" != "$da_final_verifier_elf_hex" ]]; then
  echo "committed final DA verifier code cell data does not match compiled CellScript ELF" >&2
  exit 1
fi
committed_settlement_final_verifier_data="$(jq -r '.result.transaction.outputs_data[4]' "$deploy_get_tx_path")"
if [[ "$committed_settlement_final_verifier_data" != "$settlement_final_verifier_elf_hex" ]]; then
  echo "committed final settlement verifier code cell data does not match compiled CellScript ELF" >&2
  exit 1
fi
committed_settlement_authority_data="$(jq -r '.result.transaction.outputs_data[5]' "$deploy_get_tx_path")"
if [[ "$committed_settlement_authority_data" != "$settlement_authority_data" ]]; then
  echo "committed settlement authority cell data does not match settlement authority lineage payload" >&2
  exit 1
fi

da_verifier_code_dep_index="0x1"
settlement_verifier_code_dep_index="0x2"
da_final_verifier_code_dep_index="0x3"
settlement_final_verifier_code_dep_index="0x4"
settlement_authority_tx_hash="$verifier_deploy_tx_hash"
settlement_authority_output_index="0x5"
settlement_authority_output_capacity_hex="$settlement_authority_capacity_hex"
funding_tx_hash="$verifier_deploy_tx_hash"
funding_output_capacity_hex="$funding_capacity_hex"

submit_and_verify_carrier() {
  local label="$1"
  local package_kind="$2"
  local package_path="$3"
  local input_tx_hash="$4"
  local input_index="$5"
  local input_capacity_hex="$6"
  local verifier_role="${7:-carrier}"
  local evidence_cell_dep_tx_hash="${8:-}"
  local evidence_cell_dep_index="${9:-}"
  local evidence_cell_dep_capacity_hex="${10:-}"
  local authority_input_tx_hash="${11:-}"
  local authority_input_index="${12:-}"
  local authority_input_capacity_hex="${13:-}"

  local verifier_source verifier_action verifier_code_hash_for_carrier verifier_code_dep_index_for_carrier
  case "$verifier_role:$package_kind" in
    carrier:myelin-session-da-anchor-package-v1)
      verifier_source="da-anchor-carrier.cell"
      verifier_action="verify_da_anchor_carrier"
      verifier_code_hash_for_carrier="$da_verifier_code_hash"
      verifier_code_dep_index_for_carrier="$da_verifier_code_dep_index"
      ;;
    carrier:myelin-session-settlement-package-v1)
      verifier_source="settlement-carrier.cell"
      verifier_action="verify_settlement_carrier"
      verifier_code_hash_for_carrier="$settlement_verifier_code_hash"
      verifier_code_dep_index_for_carrier="$settlement_verifier_code_dep_index"
      ;;
    final-l1-script:myelin-session-da-anchor-package-v1)
      verifier_source="da-anchor-final.cell"
      verifier_action="verify_final_da_publication"
      verifier_code_hash_for_carrier="$da_final_verifier_code_hash"
      verifier_code_dep_index_for_carrier="$da_final_verifier_code_dep_index"
      ;;
    final-l1-script:myelin-session-settlement-package-v1)
      verifier_source="settlement-final.cell"
      verifier_action="verify_final_settlement"
      verifier_code_hash_for_carrier="$settlement_final_verifier_code_hash"
      verifier_code_dep_index_for_carrier="$settlement_final_verifier_code_dep_index"
      ;;
    *)
      echo "unsupported verifier role/package kind for verifier selection: $verifier_role / $package_kind" >&2
      exit 1
      ;;
  esac
  local summary_suffix expected_readiness_mode expected_live_carrier_ready expected_final_l1_ready
  if [[ "$verifier_role" == "final-l1-script" ]]; then
    summary_suffix="final-script"
    expected_readiness_mode="final-l1-script"
    expected_live_carrier_ready="false"
    expected_final_l1_ready="true"
  else
    summary_suffix="carrier"
    expected_readiness_mode="live-ckb-carrier"
    expected_live_carrier_ready="true"
    expected_final_l1_ready="false"
  fi
  local input_capacity="$((input_capacity_hex))"
  local evidence_cell_dep_capacity=0
  local evidence_args=()
  if [[ -n "$evidence_cell_dep_tx_hash" ]]; then
    if [[ -z "$evidence_cell_dep_index" || -z "$evidence_cell_dep_capacity_hex" ]]; then
      echo "$label evidence CellDep requires tx hash, index, and capacity" >&2
      exit 1
    fi
    evidence_cell_dep_capacity="$((evidence_cell_dep_capacity_hex))"
    evidence_args+=(
      --evidence-cell-dep-tx-hash "$evidence_cell_dep_tx_hash"
      --evidence-cell-dep-index "$evidence_cell_dep_index"
      --evidence-cell-dep-capacity-shannons "$evidence_cell_dep_capacity"
    )
  fi
  local authority_input_capacity=0
  local authority_args=()
  if [[ -n "$authority_input_tx_hash" ]]; then
    if [[ -z "$authority_input_index" || -z "$authority_input_capacity_hex" ]]; then
      echo "$label authority input requires tx hash, index, and capacity" >&2
      exit 1
    fi
    authority_input_capacity="$((authority_input_capacity_hex))"
    authority_args+=(
      --authority-input-tx-hash "$authority_input_tx_hash"
      --authority-input-index "$authority_input_index"
      --authority-input-capacity-shannons "$authority_input_capacity"
    )
  fi
  local carrier_capacity="$CARRIER_CELL_CAPACITY_SHANNONS"
  local change_capacity="$((input_capacity + authority_input_capacity - carrier_capacity - FEE_SHANNONS))"
  if ((change_capacity <= 0)); then
    echo "$label input capacity is too small for carrier plus change" >&2
    echo "input: $input_capacity, evidence_cell_dep: $evidence_cell_dep_capacity, carrier: $carrier_capacity, fee: $FEE_SHANNONS" >&2
    exit 1
  fi
  local carrier_capacity_hex change_capacity_hex
  carrier_capacity_hex="$(printf '0x%x' "$carrier_capacity")"
  change_capacity_hex="$(printf '0x%x' "$change_capacity")"

  local request_path="$WORKDIR/${label}-send-transaction.json"
  local dry_run_submission_path="$WORKDIR/${label}-${summary_suffix}-dry-run-submission.json"
  local submission_path="$WORKDIR/${label}-${summary_suffix}-submission.json"
  local context_path="$WORKDIR/${label}-${summary_suffix}-context.json"
  local economics_path="$WORKDIR/${label}-${summary_suffix}-economics.json"
  local get_tx_path="$WORKDIR/${label}-get-transaction-response.json"
  local inclusion_path="$WORKDIR/${label}-${summary_suffix}-inclusion.json"
  local stability_path="$WORKDIR/${label}-${summary_suffix}-stability.json"
  local finality_path="$WORKDIR/${label}-${summary_suffix}-finality.json"
  local readiness_path="$WORKDIR/${label}-${summary_suffix}-readiness.json"
  local summary_path="$WORKDIR/${label}-${summary_suffix}-summary.json"
  local verifier_source_path="$WORKDIR/myelin/$verifier_source"

  cargo run -q -p myelin-cli -- session carrier-submission \
    --package "$package_path" \
    --input-tx-hash "$input_tx_hash" \
    --input-index "$input_index" \
    --input-capacity-shannons "$input_capacity" \
    ${evidence_args[@]+"${evidence_args[@]}"} \
    ${authority_args[@]+"${authority_args[@]}"} \
    --carrier-capacity-shannons "$carrier_capacity" \
    --fee-shannons "$FEE_SHANNONS" \
    --lock-code-hash "$ALWAYS_SUCCESS_CODE_HASH" \
    --lock-hash-type data \
    --lock-args 0x \
    --lock-code-dep-tx-hash "$genesis_tx_hash" \
    --lock-code-dep-index "$GENESIS_ALWAYS_SUCCESS_DEP_INDEX" \
    --verifier-code-hash "$verifier_code_hash_for_carrier" \
    --verifier-code-dep-tx-hash "$verifier_deploy_tx_hash" \
    --verifier-code-dep-index "$verifier_code_dep_index_for_carrier" \
    --verifier-source "$verifier_source_path" \
    --verifier-role "$verifier_role" \
    --witness 0x \
    --outputs-validator passthrough \
    --out "$dry_run_submission_path"

  local package_commitment carrier_payload_kind carrier_payload carrier_payload_data_hash carrier_identity carrier_type_args
  package_commitment="$(jq -r '.package_commitment' "$dry_run_submission_path")"
  carrier_payload_kind="$(jq -r '.carrier_payload_kind' "$dry_run_submission_path")"
  carrier_payload="$(jq -r '.carrier_payload' "$dry_run_submission_path")"
  carrier_payload_data_hash="$(jq -r '.carrier_payload_data_hash' "$dry_run_submission_path")"
  carrier_identity="$(jq -r '.carrier_identity' "$dry_run_submission_path")"
  carrier_type_args="$(jq -r '.carrier_type_args' "$dry_run_submission_path")"

  local witness_args=("$carrier_type_args")
  if [[ "$verifier_role:$package_kind" == "final-l1-script:myelin-session-settlement-package-v1" ]]; then
    if [[ -z "${da_anchor_final_type_args:-}" ]]; then
      echo "$label final settlement witness requires da_anchor_final_type_args" >&2
      exit 1
    fi
    witness_args+=("$da_final_verifier_code_hash" "$da_anchor_final_type_args")
  fi
  local witness_hex
  witness_hex="$(entry_witness_hex "$verifier_source_path" "$verifier_action" "${witness_args[@]}")"

  if [[ "$verifier_role:$package_kind" == "final-l1-script:myelin-session-settlement-package-v1" ]]; then
    local session_id_hash settlement_identity_hash competing_payload competing_identity_hash competing_type_args
    local competing_output_capacity competing_change_capacity competing_change_capacity_hex
    session_id_hash="$(jq -r '.session_id_hash' "$dry_run_submission_path")"
    settlement_identity_hash="$(jq -r '.settlement_identity_hash' "$dry_run_submission_path")"
    if [[ -z "$session_id_hash" || "$session_id_hash" == "null" || ${#session_id_hash} -ne 66 ]]; then
      jq . "$dry_run_submission_path" >&2
      echo "$label final settlement dry run did not expose session_id_hash" >&2
      exit 1
    fi
    if [[ -z "$settlement_identity_hash" || "$settlement_identity_hash" == "null" || ${#settlement_identity_hash} -ne 66 ]]; then
      jq . "$dry_run_submission_path" >&2
      echo "$label final settlement dry run did not expose settlement_identity_hash" >&2
      exit 1
    fi
    if [[ "$carrier_type_args" != "${session_id_hash}${settlement_identity_hash#0x}" ]]; then
      jq . "$dry_run_submission_path" >&2
      echo "$label final settlement type args must be session_id_hash || settlement_identity_hash" >&2
      exit 1
    fi
    competing_payload="$(
      python3 - "$carrier_payload" <<'PY'
import sys

raw = bytearray.fromhex(sys.argv[1][2:])
if len(raw) != 160:
    raise SystemExit(f"expected 160-byte final settlement payload, got {len(raw)} bytes")
raw[128] ^= 1
print("0x" + raw.hex())
PY
    )"
    competing_identity_hash="$(ckb_hash_hex "$competing_payload")"
    competing_type_args="${session_id_hash}${competing_identity_hash#0x}"
    competing_output_capacity="$carrier_capacity"
    competing_change_capacity="$((input_capacity + authority_input_capacity - (2 * competing_output_capacity) - FEE_SHANNONS))"
    if ((competing_change_capacity <= 0)); then
      echo "$label input capacity is too small for competing final settlement rejection probe" >&2
      exit 1
    fi
    competing_change_capacity_hex="$(printf '0x%x' "$competing_change_capacity")"
    local competing_request_path="$WORKDIR/${label}-competing-final-settlement-send-transaction.json"
    local competing_response_path="$WORKDIR/${label}-competing-final-settlement-send-transaction-response.json"
    local competing_summary_path="$WORKDIR/${label}-competing-final-settlement-rejection-summary.json"
    jq \
      --arg witness "$witness_hex" \
      --arg change_capacity "$competing_change_capacity_hex" \
      --arg carrier_capacity "$carrier_capacity_hex" \
      --arg verifier_code_hash "$verifier_code_hash_for_carrier" \
      --arg competing_type_args "$competing_type_args" \
      --arg competing_payload "$competing_payload" \
      '{
        id: 1,
        jsonrpc: "2.0",
        method: "send_transaction",
        params: [
          (.send_transaction_request_json.params[0]
            | .witnesses = [$witness]
            | .outputs[1].capacity = $change_capacity
            | .outputs += [{
                capacity: $carrier_capacity,
                lock: .outputs[0].lock,
                type: { code_hash: $verifier_code_hash, hash_type: "data2", args: $competing_type_args }
              }]
            | .outputs_data += [$competing_payload]),
          "passthrough"
        ]
      }' "$dry_run_submission_path" >"$competing_request_path"
    local competing_response
    competing_response="$(curl -fsS -H "content-type: application/json" -d @"$competing_request_path" "$RPC_URL")"
    printf '%s\n' "$competing_response" >"$competing_response_path"
    if jq -e '.result and (.result != null)' "$competing_response_path" >/dev/null; then
      jq . "$competing_response_path" >&2
      echo "$label competing final settlement was unexpectedly accepted by CKB" >&2
      exit 1
    fi
    if ! jq -e '.error and (.error.message | test("TransactionFailedToVerify|Script|Validation"; "i"))' "$competing_response_path" >/dev/null; then
      jq . "$competing_response_path" >&2
      echo "$label competing final settlement did not produce the expected script-verification rejection" >&2
      exit 1
    fi
    jq -n \
      --arg label "$label" \
      --arg request "$competing_request_path" \
      --arg response "$competing_response_path" \
      --arg session_id_hash "$session_id_hash" \
      --arg settlement_identity_hash "$settlement_identity_hash" \
      --arg competing_settlement_identity_hash "$competing_identity_hash" \
      --arg expected_type_args "$carrier_type_args" \
      --arg competing_type_args "$competing_type_args" \
      --arg verifier_code_hash "$verifier_code_hash_for_carrier" \
      --arg error_code "$(jq -r '.error.code' "$competing_response_path")" \
      --arg error_message "$(jq -r '.error.message' "$competing_response_path")" \
      '{
        schema: "myelin-ckb-devnet-settlement-replay-rejection-v1",
        label: $label,
        replay_probe: "competing-final-settlement-output-same-transaction",
        session_id_hash: $session_id_hash,
        settlement_identity_hash: $settlement_identity_hash,
        competing_settlement_identity_hash: $competing_settlement_identity_hash,
        expected_type_args: $expected_type_args,
        competing_type_args: $competing_type_args,
        verifier: {
          cellscript_source: "settlement-final.cell",
          code_hash: $verifier_code_hash,
          output_type_script_hash_type: "data2"
        },
        send_transaction_request: $request,
        send_transaction_response: $response,
        rejected_by_rpc: true,
        rejection_error_code: $error_code,
        rejection_error_message: $error_message
      }' >"$competing_summary_path"
  fi

  cargo run -q -p myelin-cli -- session carrier-submission \
    --package "$package_path" \
    --input-tx-hash "$input_tx_hash" \
    --input-index "$input_index" \
    --input-capacity-shannons "$input_capacity" \
    ${evidence_args[@]+"${evidence_args[@]}"} \
    ${authority_args[@]+"${authority_args[@]}"} \
    --carrier-capacity-shannons "$carrier_capacity" \
    --fee-shannons "$FEE_SHANNONS" \
    --lock-code-hash "$ALWAYS_SUCCESS_CODE_HASH" \
    --lock-hash-type data \
    --lock-args 0x \
    --lock-code-dep-tx-hash "$genesis_tx_hash" \
    --lock-code-dep-index "$GENESIS_ALWAYS_SUCCESS_DEP_INDEX" \
    --verifier-code-hash "$verifier_code_hash_for_carrier" \
    --verifier-code-dep-tx-hash "$verifier_deploy_tx_hash" \
    --verifier-code-dep-index "$verifier_code_dep_index_for_carrier" \
    --verifier-source "$verifier_source_path" \
    --verifier-role "$verifier_role" \
    --witness "$witness_hex" \
    --outputs-validator passthrough \
    --rpc-url "$RPC_URL" \
    --submit \
    --require-accepted \
    --out "$submission_path"
  jq '.send_transaction_request_json' "$submission_path" >"$request_path"

  local pre_submit_evidence_cell_dep_lock_matches
  local pre_submit_authority_input_data_hash_matches
  local pre_submit_authority_input_lock_matches
  local settlement_uniqueness_checked
  local settlement_identity_hash
  local session_id_hash
  local duplicate_settlement_detected
  local replay_protection_mode
  pre_submit_evidence_cell_dep_lock_matches="$(jq -r '.pre_submit_evidence_cell_dep_lock_matches' "$submission_path")"
  pre_submit_authority_input_data_hash_matches="$(jq -r '.pre_submit_authority_input_data_hash_matches' "$submission_path")"
  pre_submit_authority_input_lock_matches="$(jq -r '.pre_submit_authority_input_lock_matches' "$submission_path")"
  settlement_uniqueness_checked="$(jq -r '.settlement_uniqueness_checked' "$submission_path")"
  settlement_identity_hash="$(jq -r '.settlement_identity_hash // empty' "$submission_path")"
  session_id_hash="$(jq -r '.session_id_hash // empty' "$submission_path")"
  duplicate_settlement_detected="$(jq -r '.duplicate_settlement_detected' "$submission_path")"
  replay_protection_mode="$(jq -r '.replay_protection_mode // empty' "$submission_path")"
  if [[ "$verifier_role:$package_kind" == "final-l1-script:myelin-session-settlement-package-v1" ]]; then
    if [[ "$pre_submit_evidence_cell_dep_lock_matches" != "true" ]] \
      || [[ "$pre_submit_authority_input_data_hash_matches" != "true" ]] \
      || [[ "$pre_submit_authority_input_lock_matches" != "true" ]] \
      || [[ "$settlement_uniqueness_checked" != "true" ]] \
      || [[ "$duplicate_settlement_detected" != "false" ]] \
      || [[ "$replay_protection_mode" != "authority-cell-single-use-plus-transaction-local-final-script-singleton" ]]; then
      jq . "$submission_path" >&2
      echo "$label final settlement pre-submit authority/final-DA/uniqueness checks failed" >&2
      exit 1
    fi
  fi

  local submitted_tx_hash
  submitted_tx_hash="$(jq -r '.ckb_raw_tx_hash // empty' "$submission_path")"
  if [[ -z "$submitted_tx_hash" || "$submitted_tx_hash" == "null" ]]; then
    jq . "$submission_path" >&2
    exit 1
  fi

  cargo run -q -p myelin-cli -- session verify-submission-context \
    --submission "$submission_path" \
    --rpc-url "$RPC_URL" \
    --out "$context_path"
  cargo run -q -p myelin-cli -- session verify-submission-economics \
    --submission "$submission_path" \
    --rpc-url "$RPC_URL" \
    --min-fee-shannons 1 \
    --min-fee-rate-shannons-per-kb 1000 \
    --max-fee-shannons "$FEE_SHANNONS" \
    --out "$economics_path"

  local status="pending"
  local block_hash="null"
  for attempt in $(seq 1 "$COMMIT_MINING_BLOCKS"); do
    mine 1 "$label-$attempt"
    tx_response="$(rpc "{\"id\":1,\"jsonrpc\":\"2.0\",\"method\":\"get_transaction\",\"params\":[\"$submitted_tx_hash\"]}")"
    printf '%s\n' "$tx_response" >"$get_tx_path"
    status="$(jq -r '.result.tx_status.status // "unknown"' <<<"$tx_response")"
    block_hash="$(jq -r '.result.tx_status.block_hash // "null"' <<<"$tx_response")"
    if [[ "$status" == "committed" ]]; then
      break
    fi
  done

  if [[ "$status" != "committed" ]]; then
    jq . "$get_tx_path" >&2
    echo "$label carrier transaction did not commit after $COMMIT_MINING_BLOCKS mined blocks" >&2
    exit 1
  fi

  local committed_output_data
  committed_output_data="$(jq -r '.result.transaction.outputs_data[0]' "$get_tx_path")"
  if [[ "$committed_output_data" != "$carrier_payload" ]]; then
    echo "$label committed output data does not match carrier payload" >&2
    exit 1
  fi

  cargo run -q -p myelin-cli -- session verify-submission-inclusion \
    --submission "$submission_path" \
    --rpc-url "$RPC_URL" \
    --min-status committed \
    --out "$inclusion_path"
  cargo run -q -p myelin-cli -- session verify-submission-stability \
    --inclusion "$inclusion_path" \
    --rpc-url "$RPC_URL" \
    --out "$stability_path"
  cargo run -q -p myelin-cli -- session verify-submission-finality \
    --inclusion "$inclusion_path" \
    --rpc-url "$RPC_URL" \
    --min-confirmations 1 \
    --out "$finality_path"
  cargo run -q -p myelin-cli -- session verify-submission-readiness \
    --context "$context_path" \
    --economics "$economics_path" \
    --inclusion "$inclusion_path" \
    --stability "$stability_path" \
    --finality "$finality_path" \
    --require-live-submission \
    --out "$readiness_path"

  local production_submission_ready
  production_submission_ready="$(jq -r '.production_submission_ready' "$readiness_path")"
  local strict_production_submission_ready
  strict_production_submission_ready="$(jq -r '.strict_production_submission_ready' "$readiness_path")"
  local readiness_evidence_mode
  readiness_evidence_mode="$(jq -r '.readiness_evidence_mode' "$readiness_path")"
  local live_carrier_submission_ready
  live_carrier_submission_ready="$(jq -r '.live_carrier_submission_ready' "$readiness_path")"
  local final_l1_script_submission_ready
  final_l1_script_submission_ready="$(jq -r '.final_l1_script_submission_ready' "$readiness_path")"
  local operational_policy
  operational_policy="$(jq -c '.operational_policy' "$readiness_path")"
  local operational_policy_schema
  operational_policy_schema="$(jq -r '.operational_policy.schema' "$readiness_path")"
  local operational_public_chain_ready
  operational_public_chain_ready="$(jq -r '.operational_policy.public_chain_ready' "$readiness_path")"
  local operational_testnet_beta_ready
  operational_testnet_beta_ready="$(jq -r '.operational_policy.testnet_beta_ready' "$readiness_path")"
  local operational_production_ready
  operational_production_ready="$(jq -r '.operational_policy.production_ready' "$readiness_path")"
  local operational_policy_commitment
  operational_policy_commitment="$(jq -r '.operational_policy.policy_commitment' "$readiness_path")"
  if [[ "$production_submission_ready" != "true" ]] \
    || [[ "$strict_production_submission_ready" != "$expected_final_l1_ready" ]] \
    || [[ "$readiness_evidence_mode" != "$expected_readiness_mode" ]] \
    || [[ "$live_carrier_submission_ready" != "$expected_live_carrier_ready" ]] \
    || [[ "$final_l1_script_submission_ready" != "$expected_final_l1_ready" ]]; then
    jq . "$readiness_path" >&2
    exit 1
  fi
  if [[ "$operational_policy_schema" != "myelin-public-chain-operational-policy-v1" ]] \
    || [[ "$operational_public_chain_ready" != "true" ]] \
    || [[ "$operational_testnet_beta_ready" != "true" ]] \
    || [[ "$operational_production_ready" != "false" ]] \
    || [[ "$operational_policy_commitment" == "null" ]] \
    || [[ ${#operational_policy_commitment} -ne 64 ]]; then
    jq . "$readiness_path" >&2
    exit 1
  fi
  local verifier_source_hash
  verifier_source_hash="$(jq -r '.carrier_verifier.cellscript_source_hash // empty' "$submission_path")"

  jq -n \
    --arg label "$label" \
    --arg package_kind "$package_kind" \
    --arg verifier_role "$verifier_role" \
    --arg package "$package_path" \
    --arg commitment "$package_commitment" \
    --arg carrier_payload_kind "$carrier_payload_kind" \
    --arg carrier_payload "$carrier_payload" \
    --arg carrier_payload_data_hash "$carrier_payload_data_hash" \
    --arg carrier_identity "$carrier_identity" \
    --arg carrier_type_args "$carrier_type_args" \
    --arg input_tx_hash "$input_tx_hash" \
    --arg input_index "$input_index" \
    --arg input_capacity "$input_capacity_hex" \
    --arg authority_input_tx_hash "$authority_input_tx_hash" \
    --arg authority_input_index "$authority_input_index" \
    --arg authority_input_capacity "$authority_input_capacity_hex" \
    --arg submitted_tx_hash "$submitted_tx_hash" \
    --arg block_hash "$block_hash" \
    --arg carrier_capacity "$carrier_capacity_hex" \
    --arg change_capacity "$change_capacity_hex" \
    --arg submission_report "$submission_path" \
    --arg context_report "$context_path" \
    --arg economics_report "$economics_path" \
    --arg inclusion_report "$inclusion_path" \
    --arg stability_report "$stability_path" \
    --arg finality_report "$finality_path" \
    --arg readiness_report "$readiness_path" \
    --arg readiness_evidence_mode "$readiness_evidence_mode" \
    --arg verifier_code_hash "$verifier_code_hash_for_carrier" \
    --arg verifier_tx "$verifier_deploy_tx_hash" \
    --arg verifier_dep_index "$verifier_code_dep_index_for_carrier" \
    --arg verifier_source "$verifier_source" \
    --arg verifier_source_path "$verifier_source_path" \
    --arg verifier_source_hash "$verifier_source_hash" \
    --arg settlement_identity_hash "$settlement_identity_hash" \
    --arg session_id_hash "$session_id_hash" \
    --arg replay_protection_mode "$replay_protection_mode" \
    --argjson operational_policy "$operational_policy" \
    --argjson pre_submit_evidence_cell_dep_lock_matches "$pre_submit_evidence_cell_dep_lock_matches" \
    --argjson pre_submit_authority_input_data_hash_matches "$pre_submit_authority_input_data_hash_matches" \
    --argjson pre_submit_authority_input_lock_matches "$pre_submit_authority_input_lock_matches" \
    --argjson settlement_uniqueness_checked "$settlement_uniqueness_checked" \
    --argjson duplicate_settlement_detected "$duplicate_settlement_detected" \
    --argjson live_carrier_submission_ready "$live_carrier_submission_ready" \
    --argjson final_l1_script_submission_ready "$final_l1_script_submission_ready" \
    --argjson strict_production_submission_ready "$strict_production_submission_ready" \
    --argjson fee "$FEE_SHANNONS" \
    '{
      label: $label,
      package_kind: $package_kind,
      verifier_role: $verifier_role,
      package: $package,
      package_commitment_algorithm: "sha256",
      package_commitment: $commitment,
      carrier_payload_kind: $carrier_payload_kind,
      carrier_payload: $carrier_payload,
      carrier_payload_data_hash: $carrier_payload_data_hash,
      carrier_identity: $carrier_identity,
      carrier_transaction: {
        input_tx_hash: $input_tx_hash,
        input_index: $input_index,
        input_capacity: $input_capacity,
        authority_input: (if $authority_input_tx_hash == "" then null else {
          tx_hash: $authority_input_tx_hash,
          index: $authority_input_index,
          capacity: $authority_input_capacity,
          input_position: 1
        } end),
        output_capacity: $carrier_capacity,
        fee_shannons: $fee,
        submitted_tx_hash: $submitted_tx_hash,
        status: "committed",
        block_hash: $block_hash,
        output_data_matches_carrier_payload: true
      },
      pre_submit_checks: {
        evidence_cell_dep_lock_matches: $pre_submit_evidence_cell_dep_lock_matches,
        authority_input_data_hash_matches: $pre_submit_authority_input_data_hash_matches,
        authority_input_lock_matches: $pre_submit_authority_input_lock_matches,
        settlement_uniqueness_checked: $settlement_uniqueness_checked,
        duplicate_settlement_detected: $duplicate_settlement_detected,
        replay_protection_mode: $replay_protection_mode
      },
      settlement_uniqueness: {
        checked: $settlement_uniqueness_checked,
        settlement_identity_hash: $settlement_identity_hash,
        session_id_hash: $session_id_hash,
        duplicate_settlement_detected: $duplicate_settlement_detected,
        replay_protection_mode: $replay_protection_mode
      },
      carrier_output: {
        tx_hash: $submitted_tx_hash,
        index: "0x0",
        capacity: $carrier_capacity
      },
      change_output: {
        tx_hash: $submitted_tx_hash,
        index: "0x1",
        capacity: $change_capacity
      },
      carrier_verifier: {
        cellscript_source: $verifier_source,
        cellscript_source_path: $verifier_source_path,
        cellscript_source_hash_algorithm: "sha256",
        cellscript_source_hash: $verifier_source_hash,
        code_hash: $verifier_code_hash,
        code_dep: {
          out_point: { tx_hash: $verifier_tx, index: $verifier_dep_index },
          dep_type: "code"
        },
        output_type_script_hash_type: "data2",
        output_type_script_args: $carrier_type_args
      },
      live_readiness: {
        submission_report: $submission_report,
        context_report: $context_report,
        economics_report: $economics_report,
        inclusion_report: $inclusion_report,
        stability_report: $stability_report,
        finality_report: $finality_report,
        readiness_report: $readiness_report,
        production_submission_ready: true,
        strict_production_submission_ready: $strict_production_submission_ready,
        live_carrier_submission_ready: $live_carrier_submission_ready,
        final_l1_script_submission_ready: $final_l1_script_submission_ready,
        readiness_evidence_mode: $readiness_evidence_mode,
        operational_policy: $operational_policy
      }
    }' >"$summary_path"
}

assert_tampered_carrier_rejected() {
  local label="$1"
  local package_kind="$2"
  local input_tx_hash="$3"
  local input_index="$4"
  local input_capacity_hex="$5"
  local expected_payload="$6"

  local verifier_source verifier_action verifier_code_hash_for_carrier verifier_code_dep_index_for_carrier
  case "$package_kind" in
    myelin-session-da-anchor-package-v1)
      verifier_source="da-anchor-carrier.cell"
      verifier_action="verify_da_anchor_carrier"
      verifier_code_hash_for_carrier="$da_verifier_code_hash"
      verifier_code_dep_index_for_carrier="$da_verifier_code_dep_index"
      ;;
    myelin-session-settlement-package-v1)
      verifier_source="settlement-carrier.cell"
      verifier_action="verify_settlement_carrier"
      verifier_code_hash_for_carrier="$settlement_verifier_code_hash"
      verifier_code_dep_index_for_carrier="$settlement_verifier_code_dep_index"
      ;;
    *)
      echo "unsupported carrier package kind for tamper verifier selection: $package_kind" >&2
      exit 1
      ;;
  esac

  local input_capacity="$((input_capacity_hex))"
  local output_capacity="$((input_capacity - FEE_SHANNONS))"
  local output_capacity_hex
  output_capacity_hex="$(printf '0x%x' "$output_capacity")"
  local expected_data_hash
  expected_data_hash="$(ckb_hash_hex "$expected_payload")"
  local expected_identity
  expected_identity="$(carrier_identity_hex "$expected_payload")"
  local expected_type_args
  expected_type_args="$(carrier_type_args_hex "$expected_data_hash" "$expected_identity")"

  local tampered_payload
  tampered_payload="$(
    python3 - "$expected_payload" <<'PY'
import sys

raw = bytearray.fromhex(sys.argv[1][2:])
if len(raw) != 160:
    raise SystemExit(f"expected 160-byte carrier payload, got {len(raw)} bytes")
raw[64:96] = bytes(32)
print("0x" + raw.hex())
PY
  )"
  local tampered_data_hash
  tampered_data_hash="$(ckb_hash_hex "$tampered_payload")"
  local tampered_identity
  tampered_identity="$(carrier_identity_hex "$tampered_payload")"
  local tampered_type_args
  tampered_type_args="$(carrier_type_args_hex "$tampered_data_hash" "$tampered_identity")"
  local witness_hex
  witness_hex="$(entry_witness_hex "$WORKDIR/myelin/$verifier_source" "$verifier_action" "$tampered_type_args")"

  local request_path="$WORKDIR/${label}-tampered-send-transaction.json"
  local response_path="$WORKDIR/${label}-tampered-send-transaction-response.json"
  local summary_path="$WORKDIR/${label}-tamper-rejection-summary.json"

  jq -n \
    --arg payload "$tampered_payload" \
    --arg expected_payload "$expected_payload" \
    --arg expected_data_hash "$expected_data_hash" \
    --arg expected_identity "$expected_identity" \
    --arg expected_type_args "$expected_type_args" \
    --arg tampered_data_hash "$tampered_data_hash" \
    --arg tampered_identity "$tampered_identity" \
    --arg tampered_type_args "$tampered_type_args" \
    --arg input_tx "$input_tx_hash" \
    --arg input_index "$input_index" \
    --arg output_capacity "$output_capacity_hex" \
    --arg lock "$ALWAYS_SUCCESS_CODE_HASH" \
    --arg genesis_tx "$genesis_tx_hash" \
    --arg dep_index "$GENESIS_ALWAYS_SUCCESS_DEP_INDEX" \
    --arg verifier_code_hash "$verifier_code_hash_for_carrier" \
    --arg verifier_tx "$verifier_deploy_tx_hash" \
    --arg verifier_dep_index "$verifier_code_dep_index_for_carrier" \
    --arg witness "$witness_hex" \
    '{
      id: 1,
      jsonrpc: "2.0",
      method: "send_transaction",
      params: [
        {
          version: "0x0",
          cell_deps: [
            {
              out_point: { tx_hash: $genesis_tx, index: $dep_index },
              dep_type: "code"
            },
            {
              out_point: { tx_hash: $verifier_tx, index: $verifier_dep_index },
              dep_type: "code"
            }
          ],
          header_deps: [],
          inputs: [
            {
              previous_output: { tx_hash: $input_tx, index: $input_index },
              since: "0x0"
            }
          ],
          outputs: [
            {
              capacity: $output_capacity,
              lock: { code_hash: $lock, hash_type: "data", args: "0x" },
              type: { code_hash: $verifier_code_hash, hash_type: "data2", args: $tampered_type_args }
            }
          ],
          outputs_data: [$payload],
          witnesses: [$witness]
        },
        "passthrough"
      ]
    }' >"$request_path"

  local send_response
  send_response="$(curl -fsS -H "content-type: application/json" -d @"$request_path" "$RPC_URL")"
  printf '%s\n' "$send_response" >"$response_path"

  local accepted_tx_hash
  accepted_tx_hash="$(jq -r '.result // empty' <<<"$send_response")"
  if [[ -n "$accepted_tx_hash" && "$accepted_tx_hash" != "null" ]]; then
    jq . "$response_path" >&2
    echo "$label tampered carrier was unexpectedly accepted by CKB" >&2
    exit 1
  fi
  if ! jq -e '.error and (.error.message | test("TransactionFailedToVerify|Script|Validation"; "i"))' "$response_path" >/dev/null; then
    jq . "$response_path" >&2
    echo "$label tampered carrier did not produce the expected script-verification rejection" >&2
    exit 1
  fi

  jq -n \
    --arg label "$label" \
    --arg input_tx_hash "$input_tx_hash" \
    --arg input_index "$input_index" \
    --arg expected_payload "$expected_payload" \
    --arg expected_data_hash "$expected_data_hash" \
    --arg expected_identity "$expected_identity" \
    --arg expected_type_args "$expected_type_args" \
    --arg tampered_payload "$tampered_payload" \
    --arg tampered_data_hash "$tampered_data_hash" \
    --arg tampered_identity "$tampered_identity" \
    --arg tampered_type_args "$tampered_type_args" \
    --arg request "$request_path" \
    --arg response "$response_path" \
    --arg verifier_source "$verifier_source" \
    --arg verifier_code_hash "$verifier_code_hash_for_carrier" \
    --arg error_code "$(jq -r '.error.code' "$response_path")" \
    --arg error_message "$(jq -r '.error.message' "$response_path")" \
    '{
      schema: "myelin-ckb-devnet-tamper-rejection-v1",
      label: $label,
      input_tx_hash: $input_tx_hash,
      input_index: $input_index,
      expected_carrier_payload: $expected_payload,
      expected_carrier_payload_data_hash: $expected_data_hash,
      expected_carrier_identity: $expected_identity,
      expected_carrier_type_args: $expected_type_args,
      tampered_carrier_payload: $tampered_payload,
      tampered_carrier_payload_data_hash: $tampered_data_hash,
      tampered_carrier_identity: $tampered_identity,
      tampered_carrier_type_args: $tampered_type_args,
      tamper_mode: "self-consistent-zero-hash-field",
      send_transaction_request: $request,
      send_transaction_response: $response,
      verifier: {
        cellscript_source: $verifier_source,
        code_hash: $verifier_code_hash,
        output_type_script_hash_type: "data2"
      },
      rejected_by_rpc: true,
      rejection_error_code: $error_code,
      rejection_error_message: $error_message
    }' >"$summary_path"
}

submit_and_verify_carrier \
  "da-anchor" \
  "myelin-session-da-anchor-package-v1" \
  "$WORKDIR/myelin/session-da-anchor.json" \
  "$funding_tx_hash" \
  "0x0" \
  "$funding_output_capacity_hex"

da_anchor_change_tx_hash="$(jq -r '.change_output.tx_hash' "$WORKDIR/da-anchor-carrier-summary.json")"
da_anchor_change_index="$(jq -r '.change_output.index' "$WORKDIR/da-anchor-carrier-summary.json")"
da_anchor_change_capacity="$(jq -r '.change_output.capacity' "$WORKDIR/da-anchor-carrier-summary.json")"
da_anchor_tx_hash="$(jq -r '.carrier_output.tx_hash' "$WORKDIR/da-anchor-carrier-summary.json")"
da_anchor_output_index="$(jq -r '.carrier_output.index' "$WORKDIR/da-anchor-carrier-summary.json")"
da_anchor_output_capacity="$(jq -r '.carrier_output.capacity' "$WORKDIR/da-anchor-carrier-summary.json")"
da_anchor_carrier_payload="$(jq -r '.carrier_payload' "$WORKDIR/da-anchor-carrier-summary.json")"

assert_tampered_carrier_rejected \
  "da-anchor" \
  "myelin-session-da-anchor-package-v1" \
  "$da_anchor_tx_hash" \
  "$da_anchor_output_index" \
  "$da_anchor_output_capacity" \
  "$da_anchor_carrier_payload"

submit_and_verify_carrier \
  "settlement" \
  "myelin-session-settlement-package-v1" \
  "$WORKDIR/myelin/session-settlement-package.json" \
  "$da_anchor_change_tx_hash" \
  "$da_anchor_change_index" \
  "$da_anchor_change_capacity"

settlement_tx_hash="$(jq -r '.carrier_output.tx_hash' "$WORKDIR/settlement-carrier-summary.json")"
settlement_output_index="$(jq -r '.carrier_output.index' "$WORKDIR/settlement-carrier-summary.json")"
settlement_output_capacity="$(jq -r '.carrier_output.capacity' "$WORKDIR/settlement-carrier-summary.json")"
settlement_change_tx_hash="$(jq -r '.change_output.tx_hash' "$WORKDIR/settlement-carrier-summary.json")"
settlement_change_index="$(jq -r '.change_output.index' "$WORKDIR/settlement-carrier-summary.json")"
settlement_change_capacity="$(jq -r '.change_output.capacity' "$WORKDIR/settlement-carrier-summary.json")"
settlement_carrier_payload="$(jq -r '.carrier_payload' "$WORKDIR/settlement-carrier-summary.json")"

assert_tampered_carrier_rejected \
  "settlement" \
  "myelin-session-settlement-package-v1" \
  "$settlement_tx_hash" \
  "$settlement_output_index" \
  "$settlement_output_capacity" \
  "$settlement_carrier_payload"

submit_and_verify_carrier \
  "da-anchor-final" \
  "myelin-session-da-anchor-package-v1" \
  "$WORKDIR/myelin/session-da-anchor.json" \
  "$settlement_change_tx_hash" \
  "$settlement_change_index" \
  "$settlement_change_capacity" \
  "final-l1-script"

da_anchor_final_change_tx_hash="$(jq -r '.change_output.tx_hash' "$WORKDIR/da-anchor-final-final-script-summary.json")"
da_anchor_final_change_index="$(jq -r '.change_output.index' "$WORKDIR/da-anchor-final-final-script-summary.json")"
da_anchor_final_change_capacity="$(jq -r '.change_output.capacity' "$WORKDIR/da-anchor-final-final-script-summary.json")"
da_anchor_final_tx_hash="$(jq -r '.carrier_output.tx_hash' "$WORKDIR/da-anchor-final-final-script-summary.json")"
da_anchor_final_output_index="$(jq -r '.carrier_output.index' "$WORKDIR/da-anchor-final-final-script-summary.json")"
da_anchor_final_output_capacity="$(jq -r '.carrier_output.capacity' "$WORKDIR/da-anchor-final-final-script-summary.json")"
da_anchor_final_type_args="$(jq -r '.carrier_verifier.output_type_script_args' "$WORKDIR/da-anchor-final-final-script-summary.json")"

submit_and_verify_carrier \
  "settlement-final" \
  "myelin-session-settlement-package-v1" \
  "$WORKDIR/myelin/session-settlement-package.json" \
  "$da_anchor_final_change_tx_hash" \
  "$da_anchor_final_change_index" \
  "$da_anchor_final_change_capacity" \
  "final-l1-script" \
  "$da_anchor_final_tx_hash" \
  "$da_anchor_final_output_index" \
  "$da_anchor_final_output_capacity" \
  "$settlement_authority_tx_hash" \
  "$settlement_authority_output_index" \
  "$settlement_authority_output_capacity_hex"

jq -n \
  --arg schema "myelin-ckb-devnet-smoke-v1" \
  --arg ckb_root "$CKB_ROOT" \
  --arg ckb_version "$("$CKB_BIN" --version)" \
  --arg rpc_url "$RPC_URL" \
  --arg workdir "$WORKDIR" \
  --arg da_anchor_package "$WORKDIR/myelin/session-da-anchor.json" \
  --arg settlement_package "$WORKDIR/myelin/session-settlement-package.json" \
  --arg da_verifier_source "$WORKDIR/myelin/da-anchor-carrier.cell" \
  --arg da_verifier_elf "$WORKDIR/myelin/da-anchor-carrier.elf" \
  --arg da_verifier_typed_cell_elf "$WORKDIR/myelin/da-anchor-carrier.typed-cell.elf" \
  --arg da_verifier_typed_cell_metadata "$WORKDIR/myelin/da-anchor-carrier.typed-cell.elf.meta.json" \
  --arg da_verifier_code_hash "$da_verifier_code_hash" \
  --arg da_verifier_dep_index "$da_verifier_code_dep_index" \
  --arg da_verifier_code_capacity "$da_verifier_code_capacity_hex" \
  --arg settlement_verifier_source "$WORKDIR/myelin/settlement-carrier.cell" \
  --arg settlement_verifier_elf "$WORKDIR/myelin/settlement-carrier.elf" \
  --arg settlement_verifier_typed_cell_elf "$WORKDIR/myelin/settlement-carrier.typed-cell.elf" \
  --arg settlement_verifier_typed_cell_metadata "$WORKDIR/myelin/settlement-carrier.typed-cell.elf.meta.json" \
  --arg settlement_verifier_code_hash "$settlement_verifier_code_hash" \
  --arg settlement_verifier_dep_index "$settlement_verifier_code_dep_index" \
  --arg settlement_verifier_code_capacity "$settlement_verifier_code_capacity_hex" \
  --arg da_final_verifier_source "$WORKDIR/myelin/da-anchor-final.cell" \
  --arg da_final_verifier_elf "$WORKDIR/myelin/da-anchor-final.elf" \
  --arg da_final_verifier_typed_cell_elf "$WORKDIR/myelin/da-anchor-final.typed-cell.elf" \
  --arg da_final_verifier_typed_cell_metadata "$WORKDIR/myelin/da-anchor-final.typed-cell.elf.meta.json" \
  --arg da_final_verifier_code_hash "$da_final_verifier_code_hash" \
  --arg da_final_verifier_dep_index "$da_final_verifier_code_dep_index" \
  --arg da_final_verifier_code_capacity "$da_final_verifier_code_capacity_hex" \
  --arg settlement_final_verifier_source "$WORKDIR/myelin/settlement-final.cell" \
  --arg settlement_final_verifier_elf "$WORKDIR/myelin/settlement-final.elf" \
  --arg settlement_final_verifier_typed_cell_elf "$WORKDIR/myelin/settlement-final.typed-cell.elf" \
  --arg settlement_final_verifier_typed_cell_metadata "$WORKDIR/myelin/settlement-final.typed-cell.elf.meta.json" \
  --arg settlement_final_verifier_code_hash "$settlement_final_verifier_code_hash" \
  --arg settlement_final_verifier_dep_index "$settlement_final_verifier_code_dep_index" \
  --arg settlement_final_verifier_code_capacity "$settlement_final_verifier_code_capacity_hex" \
  --arg settlement_authority_data "$settlement_authority_data" \
  --arg settlement_authority_data_hash "$settlement_authority_data_hash" \
  --arg settlement_authority_session_id "$settlement_authority_session_id" \
  --arg settlement_authority_participant_set_hash "$settlement_authority_participant_set_hash" \
  --arg settlement_authority_escrow_input_cells_hash "$settlement_authority_escrow_input_cells_hash" \
  --arg settlement_authority_session_lineage_commitment "$settlement_authority_session_lineage_commitment" \
  --arg settlement_authority_session_binding "$settlement_authority_session_binding" \
  --arg settlement_authority_lineage_commitment "$settlement_authority_lineage_commitment" \
  --arg settlement_authority_capacity "$settlement_authority_capacity_hex" \
  --arg settlement_authority_tx "$settlement_authority_tx_hash" \
  --arg settlement_authority_index "$settlement_authority_output_index" \
  --argjson settlement_authority_authentication "$settlement_authority_authentication" \
  --arg verifier_deploy_tx "$verifier_deploy_tx_hash" \
  --arg funding_capacity "$funding_output_capacity_hex" \
  --argjson da_anchor "$(jq . "$WORKDIR/da-anchor-carrier-summary.json")" \
  --argjson settlement "$(jq . "$WORKDIR/settlement-carrier-summary.json")" \
  --argjson da_anchor_final "$(jq . "$WORKDIR/da-anchor-final-final-script-summary.json")" \
  --argjson settlement_final "$(jq . "$WORKDIR/settlement-final-final-script-summary.json")" \
  --argjson da_anchor_tamper_rejection "$(jq . "$WORKDIR/da-anchor-tamper-rejection-summary.json")" \
  --argjson settlement_tamper_rejection "$(jq . "$WORKDIR/settlement-tamper-rejection-summary.json")" \
  --argjson settlement_replay_rejection "$(jq . "$WORKDIR/settlement-final-competing-final-settlement-rejection-summary.json")" \
  '{
    schema: $schema,
    ckb_root: $ckb_root,
    ckb_version: $ckb_version,
    rpc_url: $rpc_url,
    workdir: $workdir,
    myelin_da_anchor_package: $da_anchor_package,
    myelin_settlement_package: $settlement_package,
    carrier_verifiers: {
      da_anchor: {
        cellscript_source: $da_verifier_source,
        typed_cell_profile_checked: true,
        typed_cell_elf: $da_verifier_typed_cell_elf,
        typed_cell_metadata: $da_verifier_typed_cell_metadata,
        elf: $da_verifier_elf,
        code_hash: $da_verifier_code_hash,
        deployment_tx_hash: $verifier_deploy_tx,
        code_dep: {
          out_point: { tx_hash: $verifier_deploy_tx, index: $da_verifier_dep_index },
          dep_type: "code"
        },
        code_cell_capacity: $da_verifier_code_capacity
      },
      settlement: {
        cellscript_source: $settlement_verifier_source,
        typed_cell_profile_checked: true,
        typed_cell_elf: $settlement_verifier_typed_cell_elf,
        typed_cell_metadata: $settlement_verifier_typed_cell_metadata,
        elf: $settlement_verifier_elf,
        code_hash: $settlement_verifier_code_hash,
        deployment_tx_hash: $verifier_deploy_tx,
        code_dep: {
          out_point: { tx_hash: $verifier_deploy_tx, index: $settlement_verifier_dep_index },
          dep_type: "code"
        },
        code_cell_capacity: $settlement_verifier_code_capacity
      },
      funding_cell_capacity_after_deploy: $funding_capacity
    },
    final_script_verifiers: {
      da_anchor: {
        cellscript_source: $da_final_verifier_source,
        typed_cell_profile_checked: true,
        typed_cell_elf: $da_final_verifier_typed_cell_elf,
        typed_cell_metadata: $da_final_verifier_typed_cell_metadata,
        elf: $da_final_verifier_elf,
        code_hash: $da_final_verifier_code_hash,
        deployment_tx_hash: $verifier_deploy_tx,
        code_dep: {
          out_point: { tx_hash: $verifier_deploy_tx, index: $da_final_verifier_dep_index },
          dep_type: "code"
        },
        code_cell_capacity: $da_final_verifier_code_capacity
      },
      settlement: {
        cellscript_source: $settlement_final_verifier_source,
        typed_cell_profile_checked: true,
        typed_cell_elf: $settlement_final_verifier_typed_cell_elf,
        typed_cell_metadata: $settlement_final_verifier_typed_cell_metadata,
        elf: $settlement_final_verifier_elf,
        code_hash: $settlement_final_verifier_code_hash,
        deployment_tx_hash: $verifier_deploy_tx,
        code_dep: {
          out_point: { tx_hash: $verifier_deploy_tx, index: $settlement_final_verifier_dep_index },
          dep_type: "code"
        },
        code_cell_capacity: $settlement_final_verifier_code_capacity
      },
      settlement_authority_cell: {
        tx_hash: $settlement_authority_tx,
        index: $settlement_authority_index,
        capacity: $settlement_authority_capacity,
        data: $settlement_authority_data,
        data_hash: $settlement_authority_data_hash,
        data_semantics: "settlement-authority-lineage-v1",
        session_id: $settlement_authority_session_id,
        participant_set_hash: $settlement_authority_participant_set_hash,
        escrow_input_cells_hash: $settlement_authority_escrow_input_cells_hash,
        session_lineage_commitment: $settlement_authority_session_lineage_commitment,
        session_binding: $settlement_authority_session_binding,
        session_authority_commitment: $settlement_authority_lineage_commitment,
        authority_authentication: $settlement_authority_authentication,
        consumed_by: "settlement-final"
      }
    },
    package_commitment_algorithm: "sha256",
    package_commitment: $da_anchor.package_commitment,
    carrier_transaction: $da_anchor.carrier_transaction,
    live_readiness: $da_anchor.live_readiness,
    da_anchor_carrier: $da_anchor,
    settlement_carrier: $settlement,
    da_anchor_final_script: $da_anchor_final,
    settlement_final_script: $settlement_final,
    settlement_replay_rejection: $settlement_replay_rejection,
    da_anchor_tamper_rejection: $da_anchor_tamper_rejection,
    settlement_tamper_rejection: $settlement_tamper_rejection,
    tamper_rejections: {
      da_anchor: $da_anchor_tamper_rejection,
      settlement: $settlement_tamper_rejection
    },
    all_live_readiness_passed: (
      $da_anchor.live_readiness.production_submission_ready
      and $settlement.live_readiness.production_submission_ready
      and $da_anchor.live_readiness.live_carrier_submission_ready
      and $settlement.live_readiness.live_carrier_submission_ready
      and $da_anchor_final.live_readiness.production_submission_ready
      and $settlement_final.live_readiness.production_submission_ready
      and ($da_anchor_final.live_readiness.live_carrier_submission_ready | not)
      and ($settlement_final.live_readiness.live_carrier_submission_ready | not)
      and $da_anchor_final.live_readiness.final_l1_script_submission_ready
      and $settlement_final.live_readiness.final_l1_script_submission_ready
      and ($da_anchor.live_readiness.final_l1_script_submission_ready | not)
      and ($settlement.live_readiness.final_l1_script_submission_ready | not)
      and $da_anchor.live_readiness.readiness_evidence_mode == "live-ckb-carrier"
      and $settlement.live_readiness.readiness_evidence_mode == "live-ckb-carrier"
      and $da_anchor_final.live_readiness.readiness_evidence_mode == "final-l1-script"
      and $settlement_final.live_readiness.readiness_evidence_mode == "final-l1-script"
      and $settlement_final.settlement_uniqueness.checked
      and ($settlement_final.settlement_uniqueness.duplicate_settlement_detected | not)
      and $settlement_replay_rejection.rejected_by_rpc
    ),
    all_live_checks_passed: (
      $da_anchor.live_readiness.production_submission_ready
      and $settlement.live_readiness.production_submission_ready
      and $da_anchor.live_readiness.live_carrier_submission_ready
      and $settlement.live_readiness.live_carrier_submission_ready
      and $da_anchor_final.live_readiness.production_submission_ready
      and $settlement_final.live_readiness.production_submission_ready
      and ($da_anchor_final.live_readiness.live_carrier_submission_ready | not)
      and ($settlement_final.live_readiness.live_carrier_submission_ready | not)
      and $da_anchor_final.live_readiness.final_l1_script_submission_ready
      and $settlement_final.live_readiness.final_l1_script_submission_ready
      and ($da_anchor.live_readiness.final_l1_script_submission_ready | not)
      and ($settlement.live_readiness.final_l1_script_submission_ready | not)
      and $da_anchor.live_readiness.readiness_evidence_mode == "live-ckb-carrier"
      and $settlement.live_readiness.readiness_evidence_mode == "live-ckb-carrier"
      and $da_anchor_final.live_readiness.readiness_evidence_mode == "final-l1-script"
      and $settlement_final.live_readiness.readiness_evidence_mode == "final-l1-script"
      and $da_anchor_tamper_rejection.rejected_by_rpc
      and $settlement_tamper_rejection.rejected_by_rpc
      and $settlement_final.settlement_uniqueness.checked
      and ($settlement_final.settlement_uniqueness.duplicate_settlement_detected | not)
      and $settlement_replay_rejection.rejected_by_rpc
    ),
    notes: [
      "This smoke deploys separate CellScript DA-anchor and settlement compact carrier verifiers plus final-script verifier artefacts, then submits live CKB devnet transactions through both roles.",
      "The smoke also submits tampered compact-payload carriers under both deployed verifiers and requires CKB script verification to reject them.",
      "The smoke submits a competing final settlement output probe before the valid final settlement and requires CKB script verification to reject it.",
      "The carrier submissions prove backwards-compatible live carrier evidence; the final-script submissions prove strict readiness under deployed final verifier artefacts."
    ]
  }' >"$REPORT"

cat "$REPORT"
echo "Myelin CKB devnet smoke passed. Report: $REPORT" >&2
