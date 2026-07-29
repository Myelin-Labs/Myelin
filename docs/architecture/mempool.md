# Mempool and RBF

`CellPool` is a deterministic, atomic admission layer over already verified state transitions.

Each entry is identified by raw txid and carries its conflict accesses, fee, cycles, pre-state root, and post-state root. Admission enforces:

- root entry pre-root equals the pool base root;
- a single-parent child pre-root equals that parent's post-root;
- physical double-spend and logical conflict policy;
- no duplicate raw transaction;
- multi-parent packages are rejected until a combined-overlay proof exists.

RBF scores the complete affected package by fee density and unlockability, then removes conflicting entries and their descendants under the same lock before inserting the replacement. Readers never observe a half-evicted package.

The mempool does not trust compiler binding hashes or compiler-emitted scheduling blobs. Conflict domains arrive only through a validated `SchedulerPlan` resolved from concrete typed Cells.
