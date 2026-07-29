# Anatomy of a Myelin transition

One admitted transition contains four distinct objects:

1. `CellTx`: version-zero CKB-shaped raw transaction plus witnesses.
2. `SchedulerPlan`: raw-tx-bound sidecar of resolved conflict hashes and READ/WRITE modes.
3. `VerifiedStateTransaction`: VM verification result, fee/cycles, and exact pre/post roots.
4. `CkbProjectionReport`: `rejected` or `wire-encoded` evidence for the exact transaction.

They are intentionally not merged. Scheduler information is not a transaction witness; a projection hash is not a VM receipt; a VM success without the exact pre-root is not authority to mutate state.

`ConflictKeySpec` selects a stable logical domain from a full typed Cell:

- `CellId` for one physical Cell lineage;
- `Field` for one schema field such as `session_id`;
- `Composite` for a canonical tuple;
- `None` disables logical conflict resolution.

READ/READ may execute in parallel. READ/WRITE and WRITE/WRITE are ordered. Two spends of the same input are rejected as a physical double spend regardless of their logical keys.

The projection report's `wire-encoded` stage only proves canonical CKB bytes and hashes. It does not assert contextual validity or node acceptance.
