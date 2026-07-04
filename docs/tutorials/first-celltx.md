# Tutorial: your first CellTx

This tutorial builds, verifies, and projects a CellTx through Myelin
with step-by-step Rust snippets. By the end you'll have a real
report on disk and you'll understand the path the CellTx takes
through the runtime.

## What we're building

A **trivial CellTx** that:

1. Consumes one Cell (the "input").
2. Creates two Cells (the "outputs").
3. Runs through Myelin's CKB-strict VM verifier.
4. Produces an execution report and a CKB projection report.

The CellTx will be deliberately small — just enough to exercise the
spine.

## Step 1 — A new Myelin CellTx

In `myelin-exec`, a CellTx is the Molecule-encoded unit of state
change. The simplest way to build one from Rust is to use the
project's own builder:

```rust
use myelin_exec::celltx::{CellTxBuilder, CellInput, CellOutput, OutPoint};
use myelin_exec::types::Script;
use myelin_hashes::Hash;

fn build_trivial_celltx() -> CellTx {
    let input = CellInput {
        previous_output: OutPoint::new([0xab; 32], 0),
        since: 0,
    };

    let lock_script = Script::default(); // an "always-success" lock
    let output = CellOutput {
        capacity: 100_000_000, // 1 CKB in shannons
        lock:     lock_script.clone(),
        type_:    None,
        data:     b"hello myelin".to_vec(),
    };

    let second_output = CellOutput {
        capacity: 100_000_000,
        lock:     lock_script,
        type_:    None,
        data:     vec![],
    };

    CellTxBuilder::new()
        .inputs(vec![input])
        .outputs(vec![output, second_output])
        .cell_deps(vec![])
        .witnesses(vec![vec![]])
        .build()
        .expect("CellTx build should be deterministic")
}
```

A few things to notice:

- `OutPoint::new(txid, index)` — the input references a Cell we
  don't actually have. The verifier doesn't load it; it just
  consumes the OutPoint for shape.
- `capacity` is in shannons (1 CKB = 100,000,000 shannons).
- The `type_: None` on outputs means no type script — the Cells are
  plain owned value Cells.

> [!TIP]
> In real Myelin code, you'll usually start from a Cell produced
> by the runtime (e.g. from a previous block) rather than fabricate
> an OutPoint. The fabricated one above is fine for tutorial
> purposes.

## Step 2 — Submit through the executor

The executor in `myelin-exec` takes a CellTx and runs it through
the deterministic VM:

```rust
use myelin_exec::executor::Executor;
use myelin_state::CellDB;
use myelin_consensus::static_closed_committee::StaticClosedCommittee;

fn run_my_executor(celltx: CellTx) -> MyelinExecutionReport {
    let cell_db = CellDB::new();
    let executor = Executor::new(cell_db, /* vm_profile */ "ckb-strict-basic");

    executor.execute(celltx)
        .expect("trivial CellTx should always execute")
}
```

`MyelinExecutionReport` carries the cycles, exit code, and state
root transition:

```text
accepted            : true
vm_exit_code        : 0
cycles              : 1527
state_root_before   : 0x0000...0000  (empty-set commit)
state_root_after    : 0x9c1a...e2f4  (after CellTx applied)
semantic_profile    : ckb-compatible
```

## Step 3 — Project to a CKB-style transaction

The projection layer answers: *"Could this CellTx be encoded as a
CKB transaction without changing semantics?"*

```rust
use myelin_exec::projection::project_celltx;

fn project_my_celltx(celltx: &CellTx) -> CkbProjectionReport {
    project_celltx(celltx)
        .expect("projection should be deterministic")
}
```

For our trivial CellTx, the report will be:

```text
projection_possible         : true
ckb_style_tx_hash           : 0x... (deterministic from bytes)
unsupported_features        : []
semantic_deviation_flags    : []
```

The empty `unsupported_features` and `semantic_deviation_flags` are
the evidence that the CellTx is fully CKB-compatible.

## Step 4 — Run it through the CLI

If you don't want to write Rust, the CLI exposes the same path:

```bash
cargo run -p myelin-cli -- celltx simple-report
```

The CLI:

1. Builds the same trivial CellTx.
2. Runs it through the executor.
3. Runs the projection layer.
4. Writes the combined report to `reports/simple-report.json`.

```bash
cat reports/simple-report.json | jq '.'
```

```json
{
  "semantic_profile": "ckb-compatible",
  "ckb_projection_possible": true,
  "execution": {
    "accepted": true,
    "vm_exit_code": 0,
    "cycles": 1527,
    "consumed_cells": ["0xabababab..."],
    "created_cells":  ["0x...", "0x..."],
    "state_root_before": "0x0000000000000000000000000000000000000000000000000000000000000000",
    "state_root_after":  "0x9c1a...e2f4"
  },
  "projection": {
    "projection_possible": true,
    "ckb_style_tx_hash": "0x...",
    "cell_inputs":  ["0xabababab..."],
    "cell_outputs": ["0x...", "0x..."],
    "cell_deps":    [],
    "witnesses":    [""],
    "unsupported_features": [],
    "semantic_deviation_flags": []
  }
}
```

> [!NOTE]
> The exact cycle count and state root will differ across runs of
> the VM, because the trivial CellTx doesn't carry any witness
> data. What matters is that both are **deterministic** — every
> invocation produces the same values for the same inputs.

## Step 5 — Make it interesting

To make the tutorial actually exercise something, try changing
the data or adding a type script:

```rust
let typed_output = CellOutput {
    capacity: 100_000_000,
    lock:     lock_script.clone(),
    type_:    Some(my_typed_script),  // <- a real CKB type script
    data:     b"typed cell data".to_vec(),
};
```

A CellTx with a typed output gets:

- A `typed_data_hash` field in the execution report.
- A `conflict_hashes` field listing the typed output's conflict
  domain.
- A projection report that *may* add an entry to
  `unsupported_features` if the type script uses Myelin-only
  syscalls.

Try it. The CLI's `celltx simple-report` doesn't expose this — for
typed outputs you'll want to use the executor API directly.

## Step 6 — Verify the projection

The most useful test you can write against your CellTx is "does the
projection report match my expectations?":

```rust
fn assert_projection(report: &CkbProjectionReport) {
    assert!(report.projection_possible,
        "tutorial assumes a CKB-projectable CellTx");
    assert!(report.unsupported_features.is_empty(),
        "tutorial assumes no Myelin-only syscalls");
    assert!(report.semantic_deviation_flags.is_empty(),
        "tutorial assumes Cell-Model-correct data");
    assert!(report.ckb_style_tx_hash.is_some(),
        "projection_possible must imply a deterministic tx hash");
}
```

If your CellTx passes these assertions, you have a Tier 1 claim:
*"projectable into a CKB-style transaction/context."*

## Where to go next

- [Teeworlds end-to-end](teeworlds-end-to-end.md) — the full
  reference workload.
- [CellDAG scheduler](../architecture/scheduler.md) — what
  schedules multiple CellTxs together.
- [CKB-style projection](../architecture/projection.md) — the
  projection deep dive.
- [Claim ladder](../security/claim-ladder.md) — what Tier 1 means
  in the wider evidence picture.