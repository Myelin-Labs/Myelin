# CellScript adapter

CellScript is an independently versioned upstream compiler. Myelin does not vendor or link its compiler workspace.

`myelin-cellscript-adapter` provides a fail-closed process boundary:

```text
toolchain lock
  -> verified release tag + exact pinned revision + compiler version + Rust toolchain
compiler attestation
  -> executable BLAKE3 + release lineage + source revision
compile request
  -> absolute source/output paths + target profile
compile result
  -> source/artifact/metadata digests
scheduler template
  -> action accesses, still unresolved
state binding
  -> concrete conflict hashes + raw-tx-bound SchedulerPlan
```

Production requests use the `ckb` target profile. The adapter requires the versioned `cellscript-witnessargs-input-type-v2` placement contract: the entry payload is carried in `WitnessArgs.input_type`, the runtime loads group input 0 and falls back to group output 0 for output-only type groups, and the lock field remains owned by the signing adapter. The adapter rechecks the exact target-profile witness ABI, artifact digest, and metadata digest before reading scheduling metadata.

The compiler is allowed to say which transaction source/index an action accesses and whether the action is statically parallelizable. It is not allowed to provide the final logical key. A source-level binding such as `session`, `pool`, or `receipt` is diagnostic text only.

Every access is resolved by `myelin-state` from:

- the exact transaction;
- the exact live-state snapshot or output;
- the full type-script identity;
- a registered `TypedCellDecl`;
- a schema-aware canonical field reader.

Missing or inconsistent data fails admission. Scheduler metadata is not inserted into transaction witnesses, so it cannot change raw transaction identity or pretend to be a CKB witness ABI.

The lock file is `cellscript-adapter/cellscript-toolchain.lock.json`. The current lock schema pins the annotated upstream release `v0.31.0` at peeled commit `5fd4c4690512b3df4cd5a46d64194d7d4d3b9491` — the former private entry-witness-placement patch was absorbed upstream (it first merged into the 0.23 nightly line and the release now rejects legacy raw entry witnesses outright) — and pins the compiler's CKB SDK v5.1.0 path dependency at `1fbf3d4c9b35ef90bdb9e6621a8d26edde6325ce`. Myelin integration sources are under `fixtures/cellscript/`.
