# Architecture

This section is the architectural truth of the Myelin workspace —
what each crate is for, what runs where, and how the pieces fit
together. If you read [Concepts](../concepts/index.md) first, the
crate names will already mean something.

## Pages

<div class="grid cards" markdown>

-   [System overview](overview.md)

    ---

    The whole system in one diagram: from CellScript source to
    committee-finalised block, with the projection layer attached.

-   [CellScript & typed-cell metadata](cellscript.md)

    ---

    The compiler contract that hands the runtime typed Cell
    identities, conflict hashes, scheduler witnesses, and proof
    obligations.

-   [Execution pipeline](exec-pipeline.md)

    ---

    The `myelin-exec` crate: CellTx shape, script groups, VM/syscall
    glue, Molecule serialisation, and the execution report.

-   [CellDAG scheduler](scheduler.md)

    ---

    How Myelin admits, parallelises, and rejects CellTxs without a
    fee market.

-   [State & data availability](state.md)

    ---

    Live/consumed/created Cells, the state root, DA manifests,
    segment proofs, and the boundary between local-only and
    production-ready DA.

-   [Mempool & admission](mempool.md)

    ---

    The CellTx pool: deterministic conflict scoring, RBF, dependency
    tracking, and the rejection taxonomy.

-   [Consensus engines](consensus.md)

    ---

    `StaticClosedCommittee` and `Tendermint`-style weighted
    precommit — same trait, selectable from TOML.

-   [CKB-style projection](projection.md)

    ---

    The credibility hinge: every chunk ships a projection report
    showing whether it's projectable into a CKB-style context.

</div>

## Crate map

The workspace is organised as one **first-party kernel** plus three
**support crates**, with a CLI and a vendored compiler:

```text
cellscript/                vendored CellScript compiler (typed-cell profile)
exec/              -> myelin-exec          CellTx, scripts, VM/syscall, scheduler
state/             -> myelin-state         live/consumed/created Cells, state root, DA proofs
mempool/           -> myelin-mempool       CellTx pool, deterministic conflict scoring
consensus/         -> myelin-consensus     static committee + Tendermint BFT
cli/               -> myelin-cli           executable command surface
core-utils/        -> myelin-core-utils    deterministic hot-path helpers
crypto/            -> myelin-hashes        hashing primitives
math/              -> myelin-math          integer + accumulator support
```

Support crates never depend on the kernel; the kernel never depends
on the CLI. The CLI is the only thing that knows about report
formats. This keeps the kernel auditable.

## Where to start

If you're new to Myelin:

1. Read [System overview](overview.md) for the picture.
2. Read [Execution pipeline](exec-pipeline.md) and [CKB-style
   projection](projection.md) for the spine.
3. Read [Consensus engines](consensus.md) for the finality model.

If you're a CKB expert landing in this codebase:

1. Read [CellScript & typed-cell metadata](cellscript.md) — the
   compiler contract is the part most likely to surprise you.
2. Read [State & data availability](state.md) for how the state root
   and DA manifest fit together.
3. Read [CKB-style projection](projection.md) for what Myelin does
   (and doesn't) try to mirror from CKB.