# VM profiles and projection stages

Myelin no longer compresses execution semantics and CKB evidence into a single “compatible” label. They are independent axes.

VM profile:

- `CkbStrict` / `ckb-strict-basic`: minimal CKB syscall/source behavior used by session and court paths;
- `MyelinExtended`: Myelin-only helper behavior, unsuitable for a CKB script-verification claim.

Projection stage:

- the pure transaction projector emits `rejected` or `wire-encoded`;
- the CKB evidence adapter can emit `context-resolved`, `consensus-validated`, `scripts-verified`, `node-accepted`, `committed`, and `finalized` from linked receipts.

A transaction may run successfully in strict CKB-VM and still have only a `wire-encoded` report when it did not run through the adapter's immutable context receipt chain. VM profile and evidence stage remain independent axes.
