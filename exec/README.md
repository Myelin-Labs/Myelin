# Myelin Execution Layer

CKB-style Cell execution for the Myelin session runtime.

## Overview

This crate implements the execution layer for finite Cell transactions:

- **CellTx types**: Cell transaction structure with lock/type scripts,
  capacity, data, deps, witnesses, and CKB-style projection support.
- **Scheduler**: typed access metadata and CellDAG construction for conflict
  ordering.
- **VM integration**: CKB-VM RISC-V script verification with explicit CKB-strict
  and Myelin-extended semantics.
- **Standard scripts**: VM lock fixtures, timelock helpers, and script sources.
- **Serialization**: Molecule-compatible public VM ABI and explicit internal
  envelope codecs.

## Architecture

```
exec/
├── celltx/          # Cell transaction types and encoding
│   ├── types.rs     # CellTx, CellInput, CellOutput, Script
│   ├── codec.rs     # Molecule serialization
│   └── sighash.rs   # blake3 signature hashing
├── scheduler/       # Parallel execution scheduler
│   ├── dag.rs       # RW-Set → CellDAG construction
│   ├── conflict.rs  # Conflict resolution (fee_density/wtxid)
│   └── executor.rs  # Topological parallel execution
├── vm/              # VM adapter layer
│   ├── machine.rs   # CKB-VM machine wrapper
│   ├── verifier.rs  # Lock/type script verifier
│   └── syscalls/    # System calls: load_cell/load_tx/...
└── scripts/         # Standard script library
    ├── mod.rs
    ├── secp256k1_blake3_lock.c
    ├── timelock.rs
    └── fixtures/
```

## Boundary

`myelin-exec` is not a CKB full node and does not import the CKB client. Its
job is to execute and report finite Cell transitions, expose CKB-style
projection evidence, and provide the script-verification substrate used by the
Session L2 and Teeworlds pressure workload.

Use `../docs/MYELIN_ARCHITECTURE.md`,
`../MYELIN_CKB_PROJECTION_AUDIT.md`, and
`../MYELIN_CKB_SEMANTIC_DEVIATIONS.md` for the current protocol boundary.
