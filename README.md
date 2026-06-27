# Myelin

Myelin is an experimental, CKB-aligned session runtime for finite Cell
execution.

It is designed for applications that need fast off-chain state transitions, but
still want those transitions to stay close to CKB's Cell Model and CKB-VM
verification semantics.

Myelin is not a CKB full-node fork, not a new L1, and not yet a finished
permissionless L2. The current repository is a protocol seed: it keeps the
execution, state, evidence, and session-finality pieces needed to test the
shape of an off-chain Cell ledger.

## The Short Version

Myelin runs high-frequency Cell transitions off-chain, keeps them finite and
typed, and emits evidence that can be inspected and, where possible, projected
towards CKB-style transaction contexts.

The current public claim should stay precise:

```text
Myelin currently uses selectable closed-validator finality for session
benchmarking and pressure testing. The CKB-style projection and future court
path is what keeps it aligned with CKB semantics.
```

## Why CKB Alignment Matters

CKB uses the Cell Model, not an account model. A transaction consumes live
Cells and creates new Cells; state changes happen through Cell replacement.
Cells can carry data, a lock script, and an optional type script, and scripts
run in CKB-VM.

Myelin follows that mental model. It does not try to hide session state inside
an account-style contract. Instead, it treats off-chain execution as a finite
Cell session that should be able to report:

- what Cells were consumed or created,
- which lock/type-script-like rules were checked,
- which VM/profile assumptions were used,
- whether the transition can be projected into a CKB-style context,
- and which evidence would be relevant during a dispute.

Official CKB references:

- [CKB documentation map](https://docs.nervos.org/llms.txt)
- [Cell Model](https://docs.nervos.org/docs/ckb-fundamentals/cell-model)
- [CKB-VM](https://docs.nervos.org/docs/ckb-fundamentals/ckb-vm)

## What Is In This Repository

| Path | Role |
| --- | --- |
| `cellscript/` | Local CellScript fork, including the `typed-cell` target profile. |
| `exec/` | Cell transactions, script verification, VM/syscall glue, scheduler witnesses, and CellDAG scheduling. |
| `state/` | Live Cell state roots and data-availability proof primitives. |
| `mempool/` | Cell transaction pool and deterministic conflict scoring. |
| `consensus/` | Static closed committee and Tendermint-style weighted precommit finality over session block hashes. |
| `cli/` | Command-line fixtures and report generation for CellTx, session, DA, settlement, and submission flows. |
| `website/` | Myelin marketing/docs landing site built with Astro. |
| `docs/` and `MYELIN_*.md` | Architecture notes, evidence reports, positioning, and rehearsal records. |

Support crates live under `core-utils/`, `crypto/`, and `math/`.

## Protocol Shape

```text
CellScript source
  -> typed-cell metadata + VM artefact
  -> CellTx delta
  -> CellDAG conflict scheduling
  -> deterministic VM verification
  -> committed session Cell state root
  -> evidence bundle for projection, DA, court, and settlement checks
```

The default semantic profile for public demos should be:

```text
semantic_profile = "ckb-compatible"
ckb_projection_possible = true
```

`myelin-native` is allowed for experiments, but it should not be the default
evidence path.

## Current Security Boundary

Myelin's current fast paths use closed-validator finality. That is useful for
benchmarking and pressure testing, but it is not a permissionless security
claim.

The claim ladder is:

```text
no projection report      -> designed to stay close to CKB semantics
successful projection     -> projectable into a CKB-style transaction/context
future exercised court    -> disputed chunk adjudicable by the CKB-aligned path
```

Static committee finality alone must not be marketed as permissionless L2
security.

## Quick Start

Prerequisites:

- Rust toolchain compatible with the workspace `Cargo.toml`.
- Python 3 for validation scripts.
- Node.js/npm if you want to build the website.

Run the focused Rust checks:

```bash
cargo check --locked --workspace --all-targets
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
```

Run the full local production gate:

```bash
scripts/myelin_production_gate.sh
```

That gate is intentionally broad. It checks Rust formatting and linting,
executes focused workspace tests, exercises runtime smoke flows, runs Session L2
open/commit/court/DA/settlement/package paths for both consensus engines, and
then runs the Teeworlds acceptance gate when the Teeworlds checkout is present.

Run the narrower Teeworlds integration gate:

```bash
scripts/myelin_teeworlds_acceptance.sh
```

## Useful CLI Entry Points

Generate a simple CellTx report:

```bash
cargo run -p myelin-cli -- celltx simple-report
```

Generate a static-committee session fixture:

```bash
cargo run -p myelin-cli -- session open-fixture \
  --consensus static-closed-committee \
  --out reports/session-open.json
```

Commit and build a court bundle:

```bash
cargo run -p myelin-cli -- session commit-fixture \
  --session reports/session-open.json \
  --out reports/session-commit.json

cargo run -p myelin-cli -- session court-bundle \
  --commit reports/session-commit.json \
  --chunk-index 0 \
  --out reports/session-court-bundle.json

cargo run -p myelin-cli -- session verify-court-bundle \
  --bundle reports/session-court-bundle.json \
  --out reports/session-court-verify.json
```

Create and verify DA evidence:

```bash
cargo run -p myelin-cli -- session da-manifest \
  --bundle reports/session-court-bundle.json \
  --storage-dir reports/session-da-store \
  --out reports/session-da-manifest.json

cargo run -p myelin-cli -- session verify-da-manifest \
  --manifest reports/session-da-manifest.json \
  --bundle reports/session-court-bundle.json \
  --storage-dir reports/session-da-store \
  --out reports/session-da-verify.json
```

## Website

The Myelin website is a separate Astro project:

```bash
cd website
npm install
npm run dev
npm run build
```

The site uses a blue-purple visual system inspired by the parent CellScript
website, but it is a separate Myelin surface. Image areas are real, replaceable
slots under `website/public/media/`.

## Evidence And Reports

Start with these documents when reviewing the protocol state:

- `MYELIN_PRODUCTION_GATE.md`
- `MYELIN_PRODUCTION_REHEARSAL_REPORT.md`
- `MYELIN_SESSION_L2_PLAN.md`
- `MYELIN_TEEWORLDS_REPRODUCIBILITY.md`
- `MYELIN_USE_CASE_POSITIONING.md`
- `docs/MYELIN_ARCHITECTURE.md`
- `docs/TEEWORLDS_FIXTURE.md`

For CellScript upstream parity, run:

```bash
scripts/check_cellscript_parent_parity.py
```

It compares the vendored `cellscript/` tree against the parent `../CellScript`
checkout, including nested CellScript repositories that Myelin vendors as flat
directories.

## Development Notes

- Keep CKB-related claims aligned with the official CKB docs.
- Prefer `ckb-compatible` evidence for public demos.
- Do not describe closed-validator fast paths as permissionless L2 security.
- Keep generated reports out of commits unless they are intentional evidence
  artefacts.
- Keep `cellscript/` changes auditable against the parent checkout.

## Licence

MIT. See `LICENSE`.
