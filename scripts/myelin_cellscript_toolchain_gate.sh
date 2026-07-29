#!/usr/bin/env bash
# Build and exercise the exact CellScript release locked by Myelin.

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
MYELIN_ROOT="$(cd -- "${SCRIPT_DIR}/.." && pwd)"
TOOLCHAIN_ROOT="${CELLSCRIPT_TOOLCHAIN_ROOT:-/tmp/myelin-cellscript-toolchain-v0.22.0}"
SOURCE_ROOT="${TOOLCHAIN_ROOT}/CellScript"
CKB_SDK_ROOT="${TOOLCHAIN_ROOT}/ckb-sdk-rust"
EVIDENCE_ROOT="${CELLSCRIPT_EVIDENCE_ROOT:-${TOOLCHAIN_ROOT}/myelin-evidence}"
ATTESTATION="${EVIDENCE_ROOT}/compiler-attestation.json"

CELLSCRIPT_REPOSITORY="https://github.com/CellScript-Labs/CellScript.git"
CELLSCRIPT_TAG="v0.22.0"
CELLSCRIPT_REVISION="830b5971237401a74dd7848b200f48b4d2ed79f4"
CKB_SDK_REPOSITORY="https://github.com/nervosnetwork/ckb-sdk-rust.git"
CKB_SDK_TAG="v5.1.0"
CKB_SDK_REVISION="1fbf3d4c9b35ef90bdb9e6621a8d26edde6325ce"

mkdir -p "${TOOLCHAIN_ROOT}" "${EVIDENCE_ROOT}/artifacts"

if [[ ! -d "${SOURCE_ROOT}/.git" ]]; then
  git clone --branch "${CELLSCRIPT_TAG}" --single-branch --recurse-submodules \
    "${CELLSCRIPT_REPOSITORY}" "${SOURCE_ROOT}"
fi
if [[ ! -d "${CKB_SDK_ROOT}/.git" ]]; then
  git clone --branch "${CKB_SDK_TAG}" --single-branch "${CKB_SDK_REPOSITORY}" "${CKB_SDK_ROOT}"
fi

[[ "$(git -C "${SOURCE_ROOT}" rev-parse HEAD)" == "${CELLSCRIPT_REVISION}" ]]
[[ "$(git -C "${SOURCE_ROOT}" rev-parse "${CELLSCRIPT_TAG}^{commit}")" == "${CELLSCRIPT_REVISION}" ]]
[[ "$(git -C "${CKB_SDK_ROOT}" rev-parse HEAD)" == "${CKB_SDK_REVISION}" ]]
[[ "$(git -C "${CKB_SDK_ROOT}" rev-parse "${CKB_SDK_TAG}^{commit}")" == "${CKB_SDK_REVISION}" ]]
git -C "${SOURCE_ROOT}" submodule update --init --recursive

cd "${MYELIN_ROOT}"
cargo run --locked -q -p myelin-cellscript-adapter -- build-attest "${SOURCE_ROOT}" "${ATTESTATION}" >/dev/null

CELLC="${SOURCE_ROOT}/target/release/cellc"
cargo run --locked -q -p myelin-cellscript-adapter -- verify "${CELLC}" "${ATTESTATION}"

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
