# Myelin State

Finite Cell state and local data-availability storage for Myelin sessions.

## Overview

This crate manages Cell state and data-availability artefacts:

- **Cell Indexing**: RocksDB-backed `CellDB` mapping OutPoint → `CellMeta`,
  plus a `ScriptIndex` (lock_hash / type_hash → OutPoints).
- **State Root**: an incremental MuHash accumulator (`CellStateTree`) —
  O(1) per insert/remove, no full re-hash over the cell set.
- **Segment Storage**: append-only segment files (1 GB each) written via
  ordinary `std::fs::File` I/O.
- **DA Proofs**: a conventional Merkle tree over ordered chunk payloads,
  with an explicit upgrade path to NMT/KZG for namespaced sampling later.

## Architecture

```
state/
├── cell_tree.rs       # CellStateTree: incremental MuHash state root + CellEntry
├── index/             # RocksDB-backed indexes
│   ├── cell_db.rs     # CellDB: OutPoint -> CellMeta, spend set, spend journal CF
│   └── script_index.rs # ScriptIndex: lock/type hash -> OutPoints + SegmentInfo
├── molecule.rs        # Molecule encoding helpers for state artefacts
└── store/             # Data-availability storage
    ├── segment.rs     # SegmentWriter / SegmentReader / SegmentMeta (1 GB segments)
    └── proof.rs       # MerkleTreeBuilder, SegmentProof, compute_segment_root
```

## Design Principles

- **Hot index, cold data**: RocksDB stores only indexes and metadata;
  large Cell payloads go to append-only segment files.
- **Incremental root**: `CellStateTree` keeps the MuHash accumulator up
  to date on every insert/remove, so `root()` is O(1).
- **Deterministic by design**: no RNG, no wall-clock dependence in the
  state path — finality evidence must be reproducible.

## Column Families (RocksDB)

| CF | Key | Value | Purpose |
|----|-----|-------|---------|
| `cells` | OutPoint (36 B) | `CellMeta` | Live cell index |
| `spent` | OutPoint (36 B) | spend marker | Spent-output tracking for conflict detection |
| `spend_journal` | Block hash (32 B) | cell-change record | Historical metadata for rollback queries |

## Boundary

This crate is not a CKB store/freezer fork. It stores and proves Myelin's
finite session Cell state, including the local DA evidence used by the
Session L2 readiness path. Use `../README.md` and the root architecture
notes for the current protocol positioning.
