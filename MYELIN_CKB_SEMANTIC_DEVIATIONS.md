# Myelin CKB Semantic Deviations

> A focused list of every place where Myelin deliberately diverges
> from upstream CKB semantics, together with the reason for the
> divergence and the place in code that surfaces the deviation.

This document pairs with `MYELIN_CKB_PROJECTION_AUDIT.md`. The
projection audit describes *how* the projection layer reports
deviations. This document describes *what* the deviations are.

## 1. Deviation register

| ID | Deviation | Where it lives in code | Why it exists |
|---|---|---|---|
| D-01 | Myelin uses a custom `CellTx` version (`CELL_TX_VERSION = 0xC001`) instead of CKB's `0`. | `exec/src/lib.rs::CELL_TX_VERSION`, `exec/src/projection.rs::project_cell_tx_to_ckb` | Myelin is a finite off-chain Cell ledger, not a CKB full node. The version byte is reserved so a future Myelin wire layout can be distinguished from CKB at the byte level. Projection reports this as `ProjectionWarning::NonCkbTransactionVersion`. |
| D-02 | Myelin has a `NetworkId` enum that is not present in CKB. | `exec/src/lib.rs::NetworkId` | Reserved for future session-network tagging. Not currently serialised into the CKB projection. |
| D-03 | Myelin scheduler witnesses (CellScript typed-cell scheduler metadata) are not part of the CKB Molecule transaction layout. | `exec/src/celltx/types.rs::CellScriptSchedulerWitness`, `exec/src/celltx/types.rs::push_cellscript_scheduler_witness` | The scheduler witness is a Myelin-only artefact carried as a regular witness slot. The projection layer does not encode it into the CKB Molecule table; it is preserved as a typed witness for Myelin's own scheduler. |
| D-04 | Myelin's `script` hash is domain-separated under `myelin:script-hash:v1` and is not the CKB script hash. | `exec/src/celltx/types.rs::Script::hash_v1` | The Myelin script hash is the local, versioned canonical form. CKB script hash is also exposed via `ckb_script_hash_molecule` for projection. |
| D-05 | Myelin has an extended `VmSemantics::MyelinExtended` profile that allows `HeaderDep`-mapped `LOAD_CELL`, Myelin-only helper syscalls in the 3001..3004 range, the Myelin session header ABI, and a legacy group source encoding. | `exec/src/vm/mod.rs::VmSemantics` | The default profile is `MyelinExtended`. CKB-strict mode (`CkbStrict`) is selectable so the CKB-VM path is reproducible against upstream CKB semantics. The CLI's `teeworlds vm-probe` runs with `CkbStrict`. |
| D-06 | Myelin `Header.proposals_hash` and `Header.nonce` are *kept* in the CKB RawHeader Molecule struct even though Myelin does not perform PoW. | `exec/src/serialization/molecule_compat.rs::CkbRawHeader`, `CkbHeader` | The CKB Molecule wire layout requires the bytes to exist at the expected positions. The doc comments were rewritten from "Proof-of-work" / "PoW nonce" to "Compact CKB header target field" / "CKB header nonce field" in the previous preparation pass, so the code is honest about being wire-faithful rather than mining. |
| D-07 | Myelin `CellInput.since` follows the CKB `since` bit layout (bit 63 = relative, bit 62 = block number). | `exec/src/celltx/types.rs::CellInput` | The bit layout is the same as CKB. The only Myelin-specific field is the encoding, which is plain little-endian `u64` to match CKB. |
| D-08 | Myelin `CellOutput.capacity` follows the CKB occupied-capacity formula (`8 + 32 + 1 + lock.args.len() + 32 + 1 + type.args.len() + data_len`). | `exec/src/celltx/types.rs::CellOutput::occupied_capacity` | The CKB formula is required for a CKB-shaped projection to remain valid. Myelin's `verify_capacity` enforces it. |
| D-09 | Myelin `CellDep` supports `DepType::Code` and `DepType::DepGroup` like CKB. The DepGroup cell-data layout defaults to the CKB Molecule `OutPointVec` for projection; the historical `count || outpoints` Myelin layout is still available via `DepGroupDataAbi::Myelin`. | `exec/src/celltx/types.rs::parse_dep_group_data`, `parse_dep_group_data_for_abi` | Both encodings are supported so existing Myelin tests/builders don't break, while the CKB projection uses the canonical Molecule layout. |
| D-10 | Myelin carries a witness-based typed-data-hash binding through `compute_typed_data_hash` for typed outputs. | `exec/src/celltx/types.rs::compute_typed_data_hash` | This is the typed-cell analogue of CKB's type-script commitment and is the input to the CKB-style projection's typed data. |
| D-11 | Myelin uses a `fee_density * unlockability` mempool scoring policy rather than a CKB-style fee/cycles policy. | `mempool/src/scorer.rs` | The Myelin policy is the local one; it is not part of the CKB projection surface because CKB does not embed mempool policy. |
| D-12 | Myelin has an extra `header_deps: Vec<[u8; 32]>` field on `CellTx` (CKB has this too but as `Vec<OutPoint>`). | `exec/src/celltx/types.rs::CellTx::new_with_header_deps` | This is a thin divergence: Myelin stores the 32-byte header hash directly to keep the projection layer simple. The CKB projection layer encodes `header_deps` as `OutPointVec`-style. |
| D-13 | Myelin's `MyelinBlock` carries a `consensus_kind` field so the block knows which engine finalised it. | `consensus/src/lib.rs::MyelinBlock`, `cli/src/main.rs::demo_block` | A CKB header does not carry this; the engine is implicit. Myelin carries it explicitly because both static-committee and Tendermint modes are first-class. |
| D-14 | Myelin's `MyelinBlock` carries a `scheduler_commitment: [u8; 32]` field that is the Myelin scheduler-report commitment. | `consensus/src/lib.rs::MyelinBlock` | CKB does not have a scheduler commitment; Myelin does because the CellDAG scheduler is first-class. The field is hashed into the block hash, so it is part of the protocol boundary. |
| D-15 | Myelin's `MyelinBlock` carries `data_commitments: Vec<[u8; 32]>` for data-availability chunk commitments. | `consensus/src/lib.rs::MyelinBlock` | The Teeworlds acceptance path uses these to commit to the tape-chunk data. CKB uses transaction commitments for the same role, but at a different boundary. |
| D-16 | Myelin consensus is closed-validator only. Static closed committee and Tendermint closed-validator precommit finality are both wired through `SelectedConsensus`. | `consensus/src/lib.rs::SelectedConsensus` | The README claim is explicit: this is a finite-session fast path, not permissionless BFT. The cross-engine signature-domain separation is what makes the Tendermint engine non-silent vs. the static engine. |
| D-17 | Myelin's `execute_teeworlds_mock_tx` produces a court bundle that records `l1_court_implemented: false`. | `cli/src/main.rs::teeworlds_court_bundle` | The court bundle is the executable input shape for a future CKB court path, not a claim that the CKB on-chain court script is finished. |

## 2. Deviations that are NOT surfaced today

The following Myelin-only behaviours are not yet represented in the
projection report. They are listed here so future sweeps know what
to add:

```text
- VmSemantics::MyelinExtended (D-05) is recorded in the
  `teeworlds vm-probe` report via the `ckb_strict` flag, but not in
  the CKB projection report. A future sweep could surface a
  `SemanticDeviation::NonCkbStrictSyscallProfile` when
  `CkbStrict` is not selected.
- Myelin scheduler witnesses (D-03) are not encoded in the CKB
  Molecule table. A future sweep could surface a
  `SemanticDeviation::SchedulerWitnessPresent` warning when an
  admitted scheduler witness is detected.
```

## 3. Conclusion

Myelin is a CKB-shaped runtime, not a CKB clone. Every divergence
from CKB is either:

```text
- a wire-faithful position Myelin must keep (D-06, D-07, D-08);
- an explicit field on the CKB side that Myelin preserves
  (header_deps, DepGroup, capacity);
- a typed-cell extension that is surfaced as a deviation warning
  or left out of the CKB projection (D-03, D-05);
- a finite-session runtime decision (D-13, D-14, D-15, D-16,
  D-17).
```

The projection report names the deviations that matter for the CKB
side and leaves the rest in Myelin's own protocol boundary.
