#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
MYELIN_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"

if [[ -n "${HOME:-}" ]]; then
  DEFAULT_TEEWORLDS_ROOT="${HOME}/RustroverProjects/teeworlds"
else
  DEFAULT_TEEWORLDS_ROOT="$(cd -- "${MYELIN_ROOT}/.." && pwd)/teeworlds"
fi
TEEWORLDS_ROOT="${TEEWORLDS_ROOT:-${DEFAULT_TEEWORLDS_ROOT}}"
OUTPUT_DIR="${OUTPUT_DIR:-/tmp/myelin-teeworlds-acceptance}"
TICKS="${TICKS:-300}"
CLIENTS="${CLIENTS:-1}"
INPUT_EVERY="${INPUT_EVERY:-5}"
SEED="${SEED:-1}"
RUNS="${RUNS:-3}"
CHUNK_BYTES="${CHUNK_BYTES:-262144}"
MAX_CYCLES="${MAX_CYCLES:-70000000}"

REPLAYER="${REPLAYER:-${TEEWORLDS_ROOT}/ckb/build/replayer_stripped}"
MAP="${MAP:-${TEEWORLDS_ROOT}/build/data/maps/dm1.map}"
CONFIG="${CONFIG:-${TEEWORLDS_ROOT}/build/myelin_replay_40265.cfg}"

TAPE="${OUTPUT_DIR}/scripted-tape.bin"
MOCK_TX="${OUTPUT_DIR}/teeworlds-mock-tx.json"
BUILD_FIXTURE_REPORT="${OUTPUT_DIR}/build-fixture.json"
VM_PROBE_REPORT="${OUTPUT_DIR}/vm-probe.json"
COURT_BUNDLE="${OUTPUT_DIR}/court-bundle.json"
COURT_VERIFY_REPORT="${OUTPUT_DIR}/court-bundle-verify.json"

require_file() {
  local path="$1"
  local label="$2"
  if [[ ! -f "${path}" ]]; then
    printf 'missing %s: %s\n' "${label}" "${path}" >&2
    exit 1
  fi
}

run_step() {
  printf '\n==> %s\n' "$1"
  shift
  "$@"
}

mkdir -p "${OUTPUT_DIR}"

require_file "${TEEWORLDS_ROOT}/rust-tools/Cargo.toml" "Teeworlds rust-tools manifest"
require_file "${REPLAYER}" "Teeworlds CKB replayer"
require_file "${MAP}" "Teeworlds map"
require_file "${CONFIG}" "Teeworlds config"

run_step "Build deterministic scripted Teeworlds tape" \
  cargo run \
    --manifest-path "${TEEWORLDS_ROOT}/rust-tools/Cargo.toml" \
    --bin teeworlds-cli \
    -- utils build-scripted-tape \
    --ticks "${TICKS}" \
    --clients "${CLIENTS}" \
    --input-every "${INPUT_EVERY}" \
    --seed "${SEED}" \
    --output "${TAPE}"

run_step "Build Myelin fixture from xxuejie's Teeworlds tooling" \
  cargo run -p myelin-cli -- teeworlds build-fixture \
    --teeworlds-root "${TEEWORLDS_ROOT}" \
    --replayer "${REPLAYER}" \
    --tape "${TAPE}" \
    --map "${MAP}" \
    --config "${CONFIG}" \
    --mock-tx-output "${MOCK_TX}" \
    --chunk-bytes "${CHUNK_BYTES}" \
    --runs "${RUNS}" \
    --out "${BUILD_FIXTURE_REPORT}"

run_step "Probe CKB-strict VM replay path" \
  cargo run -p myelin-cli -- teeworlds vm-probe \
    --replayer "${REPLAYER}" \
    --tape "${TAPE}" \
    --map "${MAP}" \
    --config "${CONFIG}" \
    --max-cycles "${MAX_CYCLES}" \
    --out "${VM_PROBE_REPORT}"

run_step "Build disputed-chunk court input bundle" \
  cargo run -p myelin-cli -- teeworlds court-bundle \
    --mock-tx "${MOCK_TX}" \
    --chunk-bytes "${CHUNK_BYTES}" \
    --chunk-index 0 \
    --out "${COURT_BUNDLE}"

run_step "Verify disputed-chunk court input bundle" \
  cargo run -p myelin-cli -- teeworlds verify-court-bundle \
    --bundle "${COURT_BUNDLE}" \
    --out "${COURT_VERIFY_REPORT}"

run_step "Validate Myelin acceptance evidence" \
  python3 - "${BUILD_FIXTURE_REPORT}" "${VM_PROBE_REPORT}" "${COURT_BUNDLE}" "${COURT_VERIFY_REPORT}" <<'PY'
import json
import sys
from pathlib import Path

build_path, vm_path, bundle_path, verify_path = [Path(arg) for arg in sys.argv[1:]]

def load(path):
    with path.open("r", encoding="utf-8") as handle:
        return json.load(handle)

def require(condition, message):
    if not condition:
        raise SystemExit(f"acceptance check failed: {message}")

build = load(build_path)
vm = load(vm_path)
bundle = load(bundle_path)
verify = load(verify_path)

fixture = build["benchmark"]["fixture"]
chunks = fixture["chunks"]
require(fixture["ckb_projection_possible"] is True, "fixture projection must be possible")
require(fixture["finality"]["finalised"] is True, "fixture block must be finalised")
require(len(chunks) > 0, "fixture must contain at least one chunk")
for index, chunk in enumerate(chunks):
    projection = chunk["ckb_projection"]
    require(projection["semantic_profile"] == "ckb-compatible", f"chunk {index} must use ckb-compatible profile")
    require(projection["ckb_projection_possible"] is True, f"chunk {index} projection must be possible")
    require(projection["ckb_raw_tx_hash"], f"chunk {index} must expose CKB raw tx hash")
    require(projection["ckb_wtx_hash"], f"chunk {index} must expose CKB witness tx hash")

require(vm["success"] is True, "VM probe must succeed")
require(vm["vm_profile"] in ("ckb-strict-basic", "ckb-strict-spawn-ipc"), "VM probe must declare a CKB-strict profile")
require(vm["ckb_strict"] is True, "VM probe must use CKB-strict semantics")
require(isinstance(vm["ckb_spawn_ipc_enabled"], bool), "VM probe must report the spawn/IPC build flag")
require(isinstance(vm["cycles"], int) and vm["cycles"] > 0, "VM probe must report positive cycles")

require(bundle["court_verifiable"] is True, "court bundle must be verifiable")
require(bundle["l1_court_implemented"] is False, "bundle must keep unfinished L1 court status explicit")
require(bundle["vm_profile"] == "ckb-strict-basic", "court bundle must use the minimal CKB-strict profile")
require(bundle["ckb_spawn_ipc_required"] is False, "court bundle must not require spawn/IPC")
require(bundle["ckb_projection"]["semantic_profile"] == "ckb-compatible", "court bundle must use ckb-compatible profile")
require(bundle["ckb_projection"]["ckb_projection_possible"] is True, "court bundle projection must be possible")
require(bundle["static_committee_evidence"]["finalised"] is True, "court bundle must include finalised committee evidence")

require(verify["valid"] is True, "court bundle verifier must pass")
checks = verify["checks"]
require(len(checks) > 0, "court bundle verifier must emit checks")
failed = [check["name"] for check in checks if not check["ok"]]
require(not failed, f"court bundle verifier failed checks: {failed}")

summary = {
    "fixture_chunks": len(chunks),
    "tape_bytes": fixture["tape_bytes"],
    "average_elapsed_ns": build["benchmark"]["average_elapsed_ns"],
	"vm_cycles": vm["cycles"],
	"vm_profile": vm["vm_profile"],
	"ckb_spawn_ipc_enabled": vm["ckb_spawn_ipc_enabled"],
	"court_checks": len(checks),
    "semantic_profile": chunks[0]["ckb_projection"]["semantic_profile"],
    "static_committee_finalised": fixture["finality"]["finalised"],
}
print(json.dumps(summary, indent=2, sort_keys=True))
PY

printf '\nMyelin Teeworlds acceptance passed.\n'
printf 'Reports written under: %s\n' "${OUTPUT_DIR}"
