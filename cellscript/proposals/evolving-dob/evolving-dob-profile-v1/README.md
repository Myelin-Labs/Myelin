# DOB-EVO

DOB-EVO is a production-oriented CellScript profile for evolving Digital Object state on CKB without mutating the original Spore DOB Cell.

The core idea is simple:

```text
immutable Spore DOB + live DobEvolutionStateV1 + decoder = current render state
```

The base DOB remains stable. Evolution happens through a typed successor state Cell with explicit actions, fixtures, schemas, and proof evidence.

This repository is standalone. It is also consumed by the main CellScript compiler repository as a submodule at:

```text
proposals/evolving-dob/evolving-dob-profile-v1
```

## How The Profile Works

DOB-EVO separates identity from change. The Spore DOB stays the canonical object, while `DobEvolutionStateV1` records the live evolution line for that object. Renderers combine the immutable DOB, the latest live evolution state, and the committed decoder to produce the current view.

The CellScript profile enforces this as a state machine:

- `initialise_dob_state` starts the state line for one Spore/Cluster identity.
- `evolve_dob_state` replaces the live state with generation `+1` and a new event commitment.
- `finalise_dob_state` closes the line so later evolution is rejected.

The important invariant is continuity. The profile preserves identity fields, advances generation monotonically, binds each accepted action into `DobEvolutionEventV1`, and rejects stale state, replayed generations, legacy encodings, and mismatched rule or decoder commitments.

## What Is Included

| Path | Purpose |
| --- | --- |
| `src/evolving_dob_type.cell` | The CellScript source for initialise, evolve, and finalise actions. |
| `schemas/` | State, intent, and event layout notes. |
| `fixtures/` | Positive and negative scenario labels for builders and VM harnesses. |
| `proofs/` | Invariant matrix and proof-plan material. |
| `docs/` | Profile, security, production-readiness, and registry-pressure notes. |
| `scripts/evolving_dob_registry_pressure.py` | Local package and registry pressure check. |
| `scripts/evolving_dob_devnet_workflow.py` | Local CKB devnet workflow for deployment and live registry evidence. |

## Quick Start

Install or build `cellc`, then run the local package checks:

```bash
cellc build --release --target riscv64-elf --target-profile ckb
cellc check --target-profile ckb --primitive-strict 0.16
cellc package verify --json
cellc publish --dry-run
python3 scripts/evolving_dob_registry_pressure.py
```

These commands are intended to prove that the source package is internally coherent before you wire it into a registry or deployment process.

## Local Devnet Evidence

If you have a CKB checkout or `CKB_BIN` available, run:

```bash
python3 scripts/evolving_dob_devnet_workflow.py --pretty
```

That workflow starts a local integration node, deploys the built type script as a live code Cell, writes deployment metadata, verifies the registry identity including `--live`, emits action build plans, generates the TypeScript builder, and runs the generated builder tests.

This is local-node evidence. It is not a claim that the package has already been deployed to public Aggron or mainnet infrastructure.

## Production Boundary

DOB-EVO/1 intentionally does not support legacy evolving-DOB encodings. It is scoped to the `DobEvolutionStateV1` model and the invariants documented under `docs/` and `proofs/`.

Start with:

- `docs/PROFILE.md`
- `docs/PRODUCTION_READINESS.md`
- `docs/SECURITY.md`
- `proofs/invariant_matrix.json`
