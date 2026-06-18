# Cell State Management

DA storage layer with Segment files and NMT/KZG commitments.

## Overview

This crate manages Cell state and data availability:

- **Cell Indexing**: OutPoint → Segment pointer mapping
- **Segment Storage**: 1GB append-only files with mmap
- **DA Proofs**: NMT/KZG commitments and sampling verification
- **Reorg Support**: SpendJournal for K-deep rollback

## Architecture

```
state/
├── kv/              # KV abstraction layer (RocksDB backend)
│   ├── mod.rs       # KV trait (get/put/batch/snapshot)
│   └── rocksdb_impl.rs
├── index/           # Cell indexing
│   ├── cell_db.rs   # CellID → SegmentPtr
│   └── script_index.rs # lock/type → CellIDs
├── store/           # Data availability storage
│   ├── segment.rs   # Segment file management (1GB segments)
│   ├── proof.rs     # NMT/KZG commitments and sampling
│   └── writer.rs    # Sequential writer
└── reorg/           # DAG reorg support
    └── spend_journal.rs # K-deep rollback log
```

## Design Principles

⚠️ **Big data never enters DB**: Cell data always goes to Segment files (mmap), RocksDB only stores indexes.

- **Hot Index**: RocksDB (CellIndexEntry with segment pointers)
- **Cold Data**: Append-only segment files (1GB each)
- **DAG-Aware**: State inherits from selected parent, SpendJournal for reorg

## Column Families (RocksDB)

| CF | Key | Value | Purpose |
|----|-----|-------|---------|
| `cells` | OutPoint(36B) | CellIndexEntry | Cell index → Segment pointer |
| `cells_by_lock` | LockHash(32B) | Vec\<OutPoint\> | Lock inverted index |
| `segments` | SegmentID(4B) | SegmentMeta | Segment metadata (nmt_root) |
| `spend_journal` | BlockHash(32B) | Vec\<CellChange\> | K-deep rollback log |

## References

- CKB Store: `/home/arthur/RustRoverProjects/ckb/store/src/`
- CKB Freezer: `/home/arthur/RustRoverProjects/ckb/freezer/src/freezer.rs`
- Spec: `spora.md` Section 7

## Status

🚧 **Under Construction** - Part of the Spora fork


