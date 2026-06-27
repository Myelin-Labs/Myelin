# NovaSeal

NovaSeal is a collection of CellScript protocol profiles for sealing, transferring, and proving CKB state with Bitcoin-aware evidence.

Use this repository if you want to inspect or build from the protocol packages themselves: the `.cell` contracts, schema notes, fixtures, proof plans, and devnet evidence tooling are all here. The main CellScript compiler repository consumes this repo as a submodule, but NovaSeal is now maintained as its own project.

## Core Model

NovaSeal treats a seal as a typed state claim, not just as a signature check. A profile defines the CKB Cell shape, the intent or receipt data that must be committed, and the authority evidence that makes a transition acceptable.

Most profiles follow the same pattern:

```text
old sealed state + intent/evidence + authorised action = new state or receipt
```

The Bitcoin-facing profiles add an explicit cross-chain boundary. They do not ask CKB to become Bitcoin; they bind CKB transitions to Bitcoin transaction, UTXO, SPV, wallet-signing, or attestation evidence that can be reviewed separately. That is why schema files, fixtures, proof-plan mappings, and devnet scripts live beside the CellScript source instead of being treated as after-the-fact examples.

## What Is Inside

| Path | What it gives you |
| --- | --- |
| `v0-mvp-skeleton/` | The original NovaSeal v0 MVP shape: BTC-authorised CKB Cell transitions, receipt proofs, and verifier shell work. |
| `agreement-profile-v0/` | A CKB-native agreement profile for pre-agreed terminal settlement paths. |
| `fungible-xudt-profile-v0/` | xUDT issue, transfer, and settlement lifecycle examples. |
| `rwa-receipt-profile-v0/` | Receipt materialisation and claim/settlement lifecycle examples for RWA-style flows. |
| `btc-transaction-commitment-profile-v0/` | A public BTC transaction commitment seal profile. |
| `btc-utxo-seal-profile-v0/` | A single-use BTC UTXO closure seal profile. |
| `dual-seal-profile-v0/` | A combined BTC UTXO closure plus CKB maturity profile. |
| `fiber-candidate-profile-v0/` | Candidate settlement modelling for Fiber-facing flows. |
| `scripts/` | Local evidence, wallet-vector, SPV, attestation, and devnet tooling. |

## Quick Start

Install the CellScript compiler first:

```bash
cargo install cellscript
```

Then build or check an individual profile:

```bash
cd v0-mvp-skeleton
cellc check --target-profile ckb
cellc build --target riscv64-asm --target-profile ckb
```

For a package-level integrity check, run:

```bash
cellc package verify --json
```

Some profiles include CKB VM harnesses and devnet scripts. Those checks need a local CKB toolchain, a local node, or the environment variables described in the relevant profile docs.

## Evidence and Safety Notes

NovaSeal is deliberately evidence-heavy. The profile directories include positive and negative fixtures, schema documents, proof-plan mappings, and audit status notes because protocol claims are only useful when they can be checked.

Treat the repository as source and local evidence, not as a blanket mainnet deployment claim. Public deployment, live-cell identity, Bitcoin SPV evidence, and external attestation requirements are documented separately in the profile docs.

## Relationship to CellScript

CellScript is the compiler and package toolchain. NovaSeal is a set of protocol packages built with it.

When checked out inside the CellScript compiler repository, this repo normally appears at:

```text
proposals/novaseal
```

When working on NovaSeal alone, use this repository directly and keep the CellScript compiler available as `cellc`.

## Useful Next Reads

- `v0-mvp-skeleton/README.md`
- `v0-mvp-skeleton/docs/AUDIT_STATUS.md`
- `agreement-profile-v0/docs/AGREEMENT_PROFILE.md`
- `DEVNET_FULL_ACCEPTANCE_RUNBOOK.md`
