# Cell Execution Layer

CKB-inspired Cell model implementation for the Myelin L2 kernel.

## Overview

This crate implements the execution layer for Cell transactions, including:

- **CellTx Types**: Cell transaction structure (Lock/Type scripts, capacity, data)
- **Scheduler**: Parallel transaction execution with RW-Set DAG
- **VM Integration**: CKB-VM (RISC-V) for script verification
- **Standard Scripts**: VM lock fixtures, timelock helpers, and script sources

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
│   ├── ckbvm.rs     # CKB-VM RISC-V integration
│   ├── interface.rs # Lock/type script interface
│   └── syscalls.rs  # System calls: load_cell/load_tx/...
└── scripts/         # Standard script library
    ├── mod.rs
    ├── secp256k1_blake3_lock.c
    ├── timelock.rs
    └── fixtures/
```

## References

- CKB Cell Model: `/home/arthur/RustRoverProjects/ckb/util/types/src/core/cell.rs`
- CKB Script Verifier: `/home/arthur/RustRoverProjects/ckb/script/src/verify.rs`
- Spec: `/home/arthur/RustRoverProjects/Spora/spora.md` Section 4-6

## Status

🚧 **Under Construction** - Part of the Myelin L2 kernel

See `spora.md` for full implementation plan.
