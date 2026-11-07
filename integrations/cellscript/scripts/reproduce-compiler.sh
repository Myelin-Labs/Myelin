#!/usr/bin/env bash
# Build the exact CellScript revision locked by this independent integration.

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
INTEGRATION_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
TOOLCHAIN_ROOT="${CELLSCRIPT_TOOLCHAIN_ROOT:-/tmp/myelin-cellscript-toolchain-5fd4c469}"
SOURCE_ROOT="${TOOLCHAIN_ROOT}/CellScript"
CKB_SDK_ROOT="${TOOLCHAIN_ROOT}/ckb-sdk-rust"
EVIDENCE_ROOT="${CELLSCRIPT_EVIDENCE_ROOT:-${TOOLCHAIN_ROOT}/myelin-evidence}"
ATTESTATION="${EVIDENCE_ROOT}/compiler-attestation.json"
LOCK_FILE="${INTEGRATION_ROOT}/adapter/cellscript-toolchain.lock.json"

CELLSCRIPT_REPOSITORY="$(jq -r '.repository' "${LOCK_FILE}").git"
CELLSCRIPT_REVISION="$(jq -r '.source_revision' "${LOCK_FILE}")"
CKB_SDK_REPOSITORY="$(jq -r '.ckb_sdk_repository' "${LOCK_FILE}").git"
CKB_SDK_TAG="$(jq -r '.ckb_sdk_release_tag' "${LOCK_FILE}")"

mkdir -p "${TOOLCHAIN_ROOT}" "${EVIDENCE_ROOT}/artifacts"

if [[ ! -d "${SOURCE_ROOT}/.git" ]]; then
  git clone --no-checkout --recurse-submodules "${CELLSCRIPT_REPOSITORY}" "${SOURCE_ROOT}"
  git -C "${SOURCE_ROOT}" checkout --detach "${CELLSCRIPT_REVISION}"
fi
if [[ ! -d "${CKB_SDK_ROOT}/.git" ]]; then
  git clone --branch "${CKB_SDK_TAG}" --single-branch "${CKB_SDK_REPOSITORY}" "${CKB_SDK_ROOT}"
fi

git -C "${SOURCE_ROOT}" submodule update --init --recursive

cd "${INTEGRATION_ROOT}"
# The adapter validates both revisions, release tags, origins and clean checkouts.
cargo run --locked -q -p myelin-cellscript-adapter -- build-attest "${SOURCE_ROOT}" "${ATTESTATION}" >/dev/null

CELLC="${SOURCE_ROOT}/target/release/cellc"

(cd "${SOURCE_ROOT}" && cargo test --locked --test entry_witness_abi)

for fixture in da-anchor-carrier settlement-carrier da-anchor-final settlement-final; do
  cargo run --locked -q -p myelin-cellscript-adapter -- compile \
    "${CELLC}" \
    "${ATTESTATION}" \
    "${INTEGRATION_ROOT}/fixtures/${fixture}.cell" \
    "${EVIDENCE_ROOT}/artifacts/${fixture}.elf" \
    >"${EVIDENCE_ROOT}/artifacts/${fixture}.compile-receipt.json"
  test -s "${EVIDENCE_ROOT}/artifacts/${fixture}.elf"
  test -s "${EVIDENCE_ROOT}/artifacts/${fixture}.elf.meta.json"
  test -s "${EVIDENCE_ROOT}/artifacts/${fixture}.artifact.json"
done

printf '%s\n' "${ATTESTATION}"
