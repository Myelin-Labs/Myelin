# Cell Memory Pool

Cell transaction pool with parallel scheduler and RW-Set DAG.

## Overview

This crate implements the Cell transaction pool (mempool):

- **CellPool**: Cell transaction queue with dependency tracking
- **Scoring**: `fee_density·α + unlockability·β` prioritization
- **RBF/CPFP**: Replace-By-Fee and Child-Pays-For-Parent
- **Conflict Detection**: DAG-aware double-spend resolution

## Architecture

```
mempool/
├── cellpool.rs      # Cell transaction pool
├── scorer.rs        # fee_density·α + unlockability·β
├── relay.rs         # Transaction relay/RBF/CPFP
└── conflicts.rs     # Conflict detection and resolution
```

## Key Features

### Scoring Formula

```rust
ancestors_score = 
    (ancestors_fee / ancestors_size) * cycles_factor * age_factor
```

### Conflict Resolution

Priority order (deterministic):
1. `fee_density` ↓ (higher is better)
2. `wtxid` ↑ (lexicographic tiebreaker)

### RBF Rules (Cell-specific)

1. New transaction must spend at least one **same OutPoint** as conflicting tx
2. `effective_fee_rate` (considering cycles) must be higher
3. Absolute fee must exceed total replaced fees + increment

## References

- CKB TxPool: `/home/arthur/RustRoverProjects/ckb/tx-pool/src/`
- Spec: `myelin.md` Section 9

## Status

🚧 **Under Construction** - Part of the Myelin L2 kernel

