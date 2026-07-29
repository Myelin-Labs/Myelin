# CellScript adapter

CellScript is an independently versioned upstream compiler. Myelin does not vendor or link its compiler workspace.

`myelin-cellscript-adapter` provides a fail-closed process boundary:

```text
toolchain lock
  -> exact upstream revision + compiler version + Rust toolchain
compiler attestation
  -> executable BLAKE3 + version + source revision
compile request
  -> absolute source/output paths + target profile
compile result
  -> source/artifact/metadata digests
scheduler template
  -> action accesses, still unresolved
state binding
  -> concrete conflict hashes + raw-tx-bound SchedulerPlan
```

Production requests use the `ckb` target profile. The adapter rechecks artifact and metadata digests before reading scheduling metadata.

The compiler is allowed to say which transaction source/index an action accesses and whether the action is statically parallelizable. It is not allowed to provide the final logical key. A source-level binding such as `session`, `pool`, or `receipt` is diagnostic text only.

Every access is resolved by `myelin-state` from:

- the exact transaction;
- the exact live-state snapshot or output;
- the full type-script identity;
- a registered `TypedCellDecl`;
- a schema-aware canonical field reader.

Missing or inconsistent data fails admission. Scheduler metadata is not inserted into transaction witnesses, so it cannot change raw transaction identity or pretend to be a CKB witness ABI.

The lock file is `cellscript-adapter/cellscript-toolchain.lock.json`. It currently pins upstream release `v0.22.0`, peeled source commit `830b5971237401a74dd7848b200f48b4d2ed79f4`, and the compiler's CKB SDK v5.1.0 path dependency at commit `1fbf3d4c9b35ef90bdb9e6621a8d26edde6325ce`. Myelin integration sources are under `fixtures/cellscript/`.
