#!/usr/bin/env bash
# Build and exercise the exact CellScript revision locked by Myelin.

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
MYELIN_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
TOOLCHAIN_ROOT="${CELLSCRIPT_TOOLCHAIN_ROOT:-/tmp/myelin-cellscript-toolchain-4c02e213}"
SOURCE_ROOT="${TOOLCHAIN_ROOT}/CellScript"
CKB_SDK_ROOT="${TOOLCHAIN_ROOT}/ckb-sdk-rust"
EVIDENCE_ROOT="${CELLSCRIPT_EVIDENCE_ROOT:-${TOOLCHAIN_ROOT}/myelin-evidence}"
ATTESTATION="${EVIDENCE_ROOT}/compiler-attestation.json"
LOCK_FILE="${MYELIN_ROOT}/cellscript-adapter/cellscript-toolchain.lock.json"

CELLSCRIPT_REPOSITORY="$(jq -r '.repository' "${LOCK_FILE}").git"
CELLSCRIPT_RELEASE_BASE_TAG="$(jq -r '.release_base_tag' "${LOCK_FILE}")"
CELLSCRIPT_RELEASE_BASE_REVISION="$(jq -r '.release_base_revision' "${LOCK_FILE}")"
CELLSCRIPT_REVISION="$(jq -r '.source_revision' "${LOCK_FILE}")"
CKB_SDK_REPOSITORY="$(jq -r '.ckb_sdk_repository' "${LOCK_FILE}").git"
CKB_SDK_TAG="$(jq -r '.ckb_sdk_release_tag' "${LOCK_FILE}")"
CKB_SDK_REVISION="$(jq -r '.ckb_sdk_source_revision' "${LOCK_FILE}")"

mkdir -p "${TOOLCHAIN_ROOT}" "${EVIDENCE_ROOT}/artifacts"

if [[ ! -d "${SOURCE_ROOT}/.git" ]]; then
  git clone --no-checkout --recurse-submodules "${CELLSCRIPT_REPOSITORY}" "${SOURCE_ROOT}"
  git -C "${SOURCE_ROOT}" checkout --detach "${CELLSCRIPT_REVISION}"
fi
if [[ ! -d "${CKB_SDK_ROOT}/.git" ]]; then
  git clone --branch "${CKB_SDK_TAG}" --single-branch "${CKB_SDK_REPOSITORY}" "${CKB_SDK_ROOT}"
fi

[[ "$(git -C "${SOURCE_ROOT}" rev-parse HEAD)" == "${CELLSCRIPT_REVISION}" ]]
[[ "$(git -C "${SOURCE_ROOT}" rev-parse "${CELLSCRIPT_RELEASE_BASE_TAG}^{commit}")" == "${CELLSCRIPT_RELEASE_BASE_REVISION}" ]]
git -C "${SOURCE_ROOT}" merge-base --is-ancestor "${CELLSCRIPT_RELEASE_BASE_REVISION}" "${CELLSCRIPT_REVISION}"
[[ "$(git -C "${CKB_SDK_ROOT}" rev-parse HEAD)" == "${CKB_SDK_REVISION}" ]]
[[ "$(git -C "${CKB_SDK_ROOT}" rev-parse "${CKB_SDK_TAG}^{commit}")" == "${CKB_SDK_REVISION}" ]]
git -C "${SOURCE_ROOT}" submodule update --init --recursive

cd "${MYELIN_ROOT}"
cargo run --locked -q -p myelin-cellscript-adapter -- build-attest "${SOURCE_ROOT}" "${ATTESTATION}" >/dev/null

CELLC="${SOURCE_ROOT}/target/release/cellc"
cargo run --locked -q -p myelin-cellscript-adapter -- verify "${CELLC}" "${ATTESTATION}"

(cd "${SOURCE_ROOT}" && cargo test --locked --test entry_witness_abi)

for fixture in da-anchor-carrier settlement-carrier da-anchor-final settlement-final; do
  cargo run --locked -q -p myelin-cellscript-adapter -- compile \
    "${CELLC}" \
    "${ATTESTATION}" \
    "${MYELIN_ROOT}/fixtures/cellscript/${fixture}.cell" \
    "${EVIDENCE_ROOT}/artifacts/${fixture}.elf" \
    >"${EVIDENCE_ROOT}/artifacts/${fixture}.compile-receipt.json"
  test -s "${EVIDENCE_ROOT}/artifacts/${fixture}.elf"
  test -s "${EVIDENCE_ROOT}/artifacts/${fixture}.elf.meta.json"
done

printf '%s\n' "${ATTESTATION}"
