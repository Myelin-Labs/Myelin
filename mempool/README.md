# Myelin Mempool

Deterministic admission and ordering support for Myelin Cell transactions.

## Overview

This crate implements the local Cell transaction pool used by the Myelin
session runtime:

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

## Boundary

This is not an L1 transaction relay or CKB tx-pool clone. It is the local
admission and prioritisation layer for finite session CellTxs. Use
`../README.md` and `../docs/MYELIN_ARCHITECTURE.md` for the current protocol
positioning.
