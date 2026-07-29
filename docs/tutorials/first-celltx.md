# Tutorial: first CellTx and projection report

Run the built-in report:

```bash
cargo run -p myelin-cli -- celltx simple-report
```

At the library boundary, a minimal transaction uses separate output data:

```rust
use myelin_exec::{project_cell_tx_to_ckb, CellInput, CellOutput, CellTx, OutPoint, ProjectionStage, Script};

let tx = CellTx::new(
    vec![CellInput::new(OutPoint::new([1; 32], 0), 0)],
    vec![],
    vec![CellOutput {
        capacity: 100_000_000,
        lock: Script::new([2; 32], 1, vec![]),
        type_: None,
    }],
    vec![b"hello".to_vec()],
    vec![vec![]],
)?;

assert_eq!(tx.version, 0);
let projection = project_cell_tx_to_ckb(&tx);
assert_eq!(projection.stage, ProjectionStage::WireEncoded);
assert!(projection.ckb_raw_tx_hash.is_some());
assert!(projection.ckb_wtx_hash.is_some());
assert!(!projection.scripts_verified());
# Ok::<(), Box<dyn std::error::Error>>(())
```

This demonstrates transaction construction and CKB Molecule wire evidence only. The fabricated input is not present in a live snapshot, no code dependency was supplied, and the projection API does not perform contextual consensus or script validation.

For a complete local transition, use the session fixture commands in [First run](../getting-started/first-run.md). They resolve Cells, verify strict CKB-VM scripts, apply state atomically, and finalise a Myelin block—but their CKB projection stage still honestly remains `wire-encoded` until context-bound projection receipts are implemented.
