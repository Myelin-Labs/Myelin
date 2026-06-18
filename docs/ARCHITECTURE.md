# Myelin Architecture Seed

Myelin starts from Spora's `spora-typed` work and narrows it to an L2 execution
kernel.

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
- GHOSTDAG and L1 consensus pipeline.
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
