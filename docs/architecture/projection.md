# CKB projection stages

Projection reports the highest CKB-alignment stage supported by concrete evidence for one exact transaction.

```text
Rejected
WireEncoded
ContextResolved
ConsensusValidated
ScriptsVerified
NodeAccepted
Committed
Finalized
```

`project_cell_tx_to_ckb` produces only `Rejected` and `WireEncoded`. `myelin-ckb-adapter` produces the higher stages from linked, reverified receipts.

`WireEncoded` means version-zero CKB Molecule transaction bytes, a raw transaction hash, and a witness-inclusive hash were deterministically produced. It does not mean inputs/deps/headers were resolved, CKB consensus rules passed, scripts passed in that exact context, or a node accepted the transaction.

`CkbProjectionReport` includes the wire stage, typed blockers/warnings, counts, Molecule byte length, and both hashes. `CkbEvidenceProjection` adds immutable context, authoritative-node validation, strict local VM, exact-hash observation, local transaction-proof verification, reorg checks, and configured-depth finality. Neither contains a caller-controlled stage override.

Court, DA-anchor, and settlement packages may repeat wire-stage evidence without upgrading it. A package advances only when it is bound to an exact verified adapter receipt. A locally valid court bundle still proves only internal consistency and retains `court_verifiable = false` until an exercised adjudication path exists.

See the repository-level `MYELIN_CKB_PROJECTION_AUDIT.md` for the receipt invariants and exercised boundary.
