# Myelin Architecture Seed

Myelin is a CKB-style isomorphic session runtime for typed Cell execution and
single-chunk L1 adjudication. It narrows the repository to an off-chain finite
Cell ledger built around typed Cell execution.

It is not a CKB full-node fork, not a new L1, and not a permissionless L2 in its
current phase. Phase one uses static closed-committee finality for session
benchmarking and pressure testing; CKB-style projection reports and the future
court path are the CKB-aligned parts of the design.

The public claim boundary is:

```text
Myelin currently uses static committee finality for session benchmarking and
pressure testing; the L1 court/projection path is what makes it CKB-aligned.
```

## Kept

- Typed Cell contract authoring through local CellScript `typed-cell`.
- Cell transaction types with lock/type/data fields.
- CKB-VM-style script verification and syscall adapters.
- Scheduler-visible conflict metadata:
  - typed-cell conflict keys
  - conflict hashes
  - typed data hashes
  - read/write source/index records
- CellDAG dependency and conflict scheduling.
- Live Cell state roots and segment proofs.
- A small Cell transaction pool for local sequencing experiments.

## Removed

- PoW.
- Mining.
- inherited L1 consensus pipeline.
- Full-node daemon entry points.
- P2P block propagation.
- RPC services tied to the old node.
- Integration tests and devnet bootstrap code tied to the old L1.

## Direction

The next protocol layer should define session, challenge, settlement, and exit
objects around this retained Cell execution kernel. The intended invariant is:

```text
accepted Myelin transition == replayable typed Cell transition
```

The repository does not yet implement the full L2 protocol. It is a prepared
base for that work.

Early evidence should prefer:

```text
semantic_profile = "ckb-compatible"
ckb_projection_possible = true
```

`myelin-native` remains useful for experiments, but it should not be the default
evidence path while the project is trying to prove CKB isomorphism. Before a
transition has a projection report, Myelin can only claim that it is designed to
stay close to CKB semantics; after the report, it can claim that the transition
is projectable into a CKB-style transaction/context, or list exact deviation
flags.

The project should keep this claim ladder visible:

```text
static committee finality -> useful session fast path and benchmark harness
projection report         -> concrete CKB-style semantic evidence
court bundle              -> executable input shape for disputed chunks
future court script       -> actual L1 adjudication path
```

Until the final step is implemented and exercised, Myelin should be described
as an experimental CKB-style isomorphic session runtime, not as a finished
trustless L2.

The immediate implementation path is deliberately narrow:

```text
simple CellTx -> execution report
simple CellTx -> CKB projection report
Teeworlds fixture -> measured benchmark JSON with per-chunk CKB projection status
```
