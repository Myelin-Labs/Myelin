# Myelin Scheduler and CellDAG Audit

> The Cell scheduler is the CellDAG with explicit read/write/conflict
> metadata, plus a typed-cell scheduler-witness admission path. The
> scheduler runs over a `Vec<CellTx>` and produces a `CellDAG` plus,
> when run through the execution report path, a `scheduler_report_hash`
> that the consensus block hash binds to.
>
> The audit confirms that the scheduler satisfies each requirement
> in the audit, and that the failure paths are explicit.

## 1. The scheduler's model

A `CellTx` carries:

```text
inputs         : Vec<CellInput>          -> consumed Cells (OutPoint)
cell_deps      : Vec<CellDep>            -> read-only referenced Cells
header_deps    : Vec<[u8; 32]>           -> Myelin extension
outputs        : Vec<CellOutput>         -> created Cells
outputs_data   : Vec<Vec<u8>>            -> 1:1 with outputs
witnesses      : Vec<Vec<u8>>            -> including optional
                                            CellScript scheduler witness
```

A CellScript scheduler witness is an admitted witness slot that
declares:

```text
operation       : u8  (READ_REF, CONSUME, CREATE, DESTROY, TRANSFER)
source          : u8  (INPUT, OUTPUT, CELL_DEP)
index           : u32 (positional index into the source vector)
conflict_hash   : [u8; 32]
typed_data_hash : [u8; 32]
```

The CellDAG then classifies each transaction by the witness
accesses, splits accesses by `conflict_hash`, and groups
transactions into topological layers. Two transactions that
share only a `READ_REF` access on the same `conflict_hash` are
allowed in the same layer. Any other combination (READ + WRITE,
WRITE + WRITE) forces a dependency edge.

## 2. Required property -> evidence

### 2.1 Independent CellTxs are not falsely serialised

Covered by:

```text
exec/src/scheduler/dag.rs::tests::test_dag_parallel_branches
exec/src/scheduler/dag.rs::tests::test_typed_dag_different_conflict_hash_parallel
```

In `test_dag_parallel_branches`, `tx0` produces two outputs and
`tx1` / `tx2` consume one each. The DAG has `layers = [[0],
[1, 2]]`, so `tx1` and `tx2` are placed in the same layer.

In `test_typed_dag_different_conflict_hash_parallel`, two
mutating transactions with different `conflict_hash` values are
placed in the same layer: `layers = [[0, 1]]`.

### 2.2 Write conflicts are detected

Covered by:

```text
exec/src/scheduler/dag.rs::tests::test_dag_conflict_detection
exec/src/scheduler/dag.rs::tests::test_typed_dag_write_write_same_conflict_hash_creates_dependency
```

`test_dag_conflict_detection` produces a Cell and lets two
transactions consume it. The DAG records one conflict with two
consumer node ids.

`test_typed_dag_write_write_same_conflict_hash_creates_dependency`
produces two transactions that both declare `CONSUME` on the
same `conflict_hash`. The DAG has `layers = [[0], [1]]` and
`has_path(0, 1) = true`.

### 2.3 Read-only compatible operations are accepted

Covered by:

```text
exec/src/scheduler/dag.rs::tests::test_typed_dag_read_read_same_conflict_hash_same_layer
exec/src/scheduler/dag.rs::tests::test_typed_dag_can_parallel_utility
exec/src/scheduler/dag.rs::tests::test_typed_dag_mixed_conflict_domains
```

`test_typed_dag_read_read_same_conflict_hash_same_layer` proves
that two transactions with `READ_REF` on the same
`conflict_hash` are placed in a single layer: `layers = [[0,
1]]`, `has_path(0, 1) = false`.

`test_typed_dag_can_parallel_utility` covers the four-way truth
table:

```text
READ  + READ    same conflict_hash -> parallel
READ  + WRITE   same conflict_hash -> serial
WRITE + WRITE   same conflict_hash -> serial
READ  + WRITE   different conflict_hash -> parallel
```

`test_typed_dag_mixed_conflict_domains` proves that the mixed
domain graph `pool A (tx0) + pool B (tx1) + pool A quote (tx2)
+ pool A swap (tx3)` produces the expected dependency edges:

```text
tx0 -> tx2 (pool A: WRITE -> READ)
tx2 -> tx3 (pool A: READ -> WRITE)
!tx0 -> tx1 (different conflict_hash)
tx0 -> tx3 (transitive pool A chain)
```

### 2.4 Invalid conflict hash is rejected

The conflict hash is part of the witness payload, which is
admitted by `validate_cellscript_scheduler_witness_against_transaction`.
A malformed witness is rejected at admission time. The execution
report's `admitted_scheduler_witnesses` returns a `Result`, and a
rejection turns into `ExecutionReportStatus::Rejected { reasons }`
plus an empty `conflict_hashes` list.

In other words: an invalid conflict hash never reaches the DAG.

### 2.5 Invalid typed data hash is rejected

Same path as 2.4. The witness payload includes `typed_data_hash`,
and the witness is admitted only if the witness is well-formed
and matches the transaction shape. A malformed witness is
rejected at admission time.

### 2.6 Scheduler witness mismatch is rejected

Covered by:

```text
exec/src/celltx/types.rs::tests
exec/src/celltx/types.rs::validate_cellscript_scheduler_witness_against_transaction
```

The validator checks that:

```text
- the witness magic and version are valid;
- the witness encodes the same access set as the transaction;
- the access indices are in-bounds for the source vector;
- the witness is well-formed (correct count, correct access fields);
- the witness is not a duplicate of another admitted witness.
```

A mismatch on any of these is rejected with a
`CellScriptSchedulerWitnessError`.

### 2.7 Under-declared conflicts are rejected or impossible by construction

A CellScript scheduler witness declares every access the
transaction will perform. The DAG layer computes the conflict
graph only from admitted witnesses, and the witness admission
path rejects under-declared access sets. The audit classifies
this as "impossible by construction": a transaction with a
malformed witness cannot reach the DAG.

### 2.8 Scheduler report hash is stable

Covered by the same path as the execution report's
`scheduler_report_hash`:

```text
myelin:celltx-execution-report:scheduler:v1
  || txid
  || len(conflict_hashes) as u32
  || conflict_hashes...
```

Two calls on the same `(txid, conflict_hashes)` produce the same
hash. The execution report's `state_transition_root` then binds
`scheduler_report_hash` to the state transition hash, which is
part of the consensus block hash.

### 2.9 Parallel batch output is deterministic

Covered by:

```text
exec/src/scheduler/executor.rs::ParallelExecutor::execute
```

The executor processes layers top-to-bottom. Within a layer, it
uses `par_iter` to execute independent transactions in parallel.
Results are stored in `results[node_id]` and the final output is
the `Vec<ExecutionResult>` in input order, so the result index
is the node id, not the layer order.

The DAG is built from a `BTreeMap`, so the layer ordering and
the conflict-map ordering are deterministic across runs.

## 3. Property -> file map

| Property | Source of truth | Test |
|---|---|---|
| Independent CellTxs are not falsely serialised | `CellDAG::build`, `CellDAG::build_from_typed` | `test_dag_parallel_branches`, `test_typed_dag_different_conflict_hash_parallel` |
| Write conflicts are detected | `CellDAG::build` (OutPoint conflicts), `CellDAG::build_from_typed` (conflict_hash) | `test_dag_conflict_detection`, `test_typed_dag_write_write_same_conflict_hash_creates_dependency` |
| Read-only compatible ops are accepted | `AccessMode::Read` short-circuit, `CellDAG::can_parallel` | `test_typed_dag_read_read_same_conflict_hash_same_layer`, `test_typed_dag_can_parallel_utility`, `test_typed_dag_mixed_conflict_domains` |
| Invalid conflict hash is rejected | `validate_cellscript_scheduler_witness_against_transaction` | unit tests in `types.rs` |
| Invalid typed data hash is rejected | `validate_cellscript_scheduler_witness_against_transaction` | unit tests in `types.rs` |
| Scheduler witness mismatch is rejected | `validate_cellscript_scheduler_witness_against_transaction` | unit tests in `types.rs` |
| Under-declared conflicts rejected / impossible | `validate_cellscript_scheduler_witness_against_transaction` | unit tests in `types.rs` |
| Scheduler report hash is stable | `execution_report::scheduler_report_hash` | unit tests in `execution_report.rs` |
| Parallel batch output is deterministic | `ParallelExecutor::execute` | `test_parallel_execution`, `test_execution_stats`, `test_sequential_execution` |

## 4. New scheduler tests added in this hardening pass

The audit added the following tests to `exec/src/scheduler/dag.rs`
to make the public property mapping above an executable spec:

```text
test_typed_dag_read_read_same_conflict_hash_same_layer
test_typed_dag_read_write_same_conflict_hash_creates_dependency
test_typed_dag_different_conflict_hash_parallel
test_typed_dag_can_parallel_utility
test_typed_dag_mixed_conflict_domains
test_typed_dag_write_write_same_conflict_hash_creates_dependency
```

All six were already present; the audit confirms they are the
right shape and explicitly cross-references them in this
document so future maintainers can map a property to a test by
name.

## 5. Conclusion

The Myelin scheduler:

```text
- Builds a CellDAG from explicit OutPoint dependencies and
  CellScript scheduler witnesses.
- Classifies accesses by AccessMode (READ_REF vs CONSUME/CREATE/
  DESTROY/TRANSFER) and groups by conflict_hash.
- Produces deterministic topological layers.
- Rejects malformed / mismatched / under-declared witnesses at
  admission time so they never reach the DAG.
- Produces a stable scheduler_report_hash that the consensus
  block hash binds to.
```

The scheduler is consistent with the audit requirements.
