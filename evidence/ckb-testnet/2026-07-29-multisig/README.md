# Public CKB testnet multisig rehearsal — 2026-07-29

This directory records a real public `ckb_testnet` lock rehearsal. It proves
standard sighash funding, legacy genesis multisig compatibility, and the
currently recommended multisig-v2 2-of-3 create/spend path. It does **not**
prove that Myelin's CellScript verifier, court, settlement, or DA path is
deployed on public testnet.

## Network and lock identities

| Item | Value |
| --- | --- |
| Chain | `ckb_testnet` |
| Genesis | `0x10639e0895502b5688a6be8cf69460d76541bfa4821629d86d62ba0aae3f9606` |
| Standard sighash lock | `0x9bd7e06f3ecf4be0f2fcd2188b23f1b9fcc88e5d4b65a8637b17723bbda3cce8/type` |
| Legacy genesis multisig | `0x5c5069eb0857efc65e1bca0c07df34c31663b3622fd3876c876320fc9634e2a8/type` |
| Recommended multisig v2 | `0x36c971b8d41fbd94aabca77dc75e826729ac98447b46f91e00796155dddb0d29/data1` |
| Multisig-v2 DepGroup | `0x2eefdeb21f3a3edf697c28a52601b4419806ed60bb427420455cc29a090b26d5:0` |
| 2-of-3 lock args | `0x50e0ad9cebbf06d20fc6b72f3bea26917998b31d` |

The ordered participant Blake160 hashes and complete config commitment are in
[`multisig-config.json`](multisig-config.json). Secret keys are disposable,
stored outside the repository, and absent from every tracked artefact.

## On-chain result

| Transition | Transaction | Committed block | Height | Finalized evidence |
| --- | --- | --- | ---: | --- |
| Standard → legacy 2-of-3 | `0x2be3685bdb90dc7f79aade0cf239d43037d029f68a77a866e700ad4dc1f30d2e` | `0x7dffb4d638ee893caa776a54d1e8208bef2b66f91269f261b0398413f780b2d5` | 21,909,343 | RPC admission/commitment report |
| Spend legacy 2-of-3 | `0xe0ddab558cda05a8a4cab8ec5dcc5323520a2e6e3c70ee84d2071a61d95e4c38` | `0xd5ff08e7f8b8085c534a7df9afab50b206c97943ecaeb03fac86f0519a198bad` | 21,909,363 | RPC admission/commitment report |
| Standard → multisig v2 | `0xb6141a89ce9e80edb997c6b6635cee4ac98dfa1b9af91fdeca564a5aa00c55a3` | `0x7afe430832733bcfd916abf4586e80153252a11168d78c8753163cadae3b7ab6` | 21,909,602 | 6 confirmations observed |
| Spend multisig v2 | `0xd338df676eb97c917566a7006517f8dd14521979e00c90b0cc47cd3392268242` | `0x2977576af6635b90832d9ffaadc285ca8f7ebc3e0ad85426ef7d484666ec12a0` | 21,909,618 | 6 confirmations observed |

The v2 spend used participants 2 and 0 in caller order. Myelin recovered their
public identities, sorted signatures into participant order, verified the
canonical witness locally, passed CKB `test_tx_pool_accept`, observed the exact
submitted hash, verified the CKB transaction proof, and re-queried the
canonical block after the configured confirmation depth.

## Verify the archived receipt chain

The full resolved context contains CKB's secp256k1 data Cell. The JSON files are
therefore stored as lossless gzip archives to avoid roughly 34 MiB of
uncompressed byte arrays.

```bash
gzip -dc evidence/ckb-testnet/2026-07-29-multisig/spend-multisig-v2-evidence-backed.json.gz \
  > /tmp/myelin-spend-multisig-v2-evidence-backed.json
gzip -dc evidence/ckb-testnet/2026-07-29-multisig/spend-multisig-v2-finalized.json.gz \
  > /tmp/myelin-spend-multisig-v2-finalized.json

cargo run --locked -q -p myelin-cli -- ckb verify \
  --transaction /tmp/myelin-spend-multisig-v2-evidence-backed.json \
  --evidence /tmp/myelin-spend-multisig-v2-finalized.json
```

Expected result: `stage = "finalized"` and `valid = true` for raw transaction
hash `d338df676eb97c917566a7006517f8dd14521979e00c90b0cc47cd3392268242`.

## Verifier deployment capacity blocker

[`settlement-final.elf`](settlement-final.elf) is the exact 38,628-byte artifact
reproduced from the pinned CellScript v0.22.0 toolchain. The conservative
[`settlement-final-capacity-plan.json`](settlement-final-capacity-plan.json)
requires 3,882,200,100,000 shannons (38,822.001 CKB) for the code Cell, change,
and fee. The available 9,399.996 CKB input is short by 29,422.005 CKB.

Consequently, no public-testnet verifier deployment, DA publication, court
execution, or final settlement is claimed by this rehearsal.
