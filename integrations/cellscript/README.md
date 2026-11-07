# Optional CellScript integration

This independent Cargo workspace owns the compiler adapter, pinned upstream
toolchain, four `.cell` carrier contracts and their acceptance drill. Root Myelin
does not build it, invoke it or select it in a validation profile. Dependencies
point from this integration into Myelin's generic execution/artifact APIs.

```bash
cargo test --locked --manifest-path integrations/cellscript/Cargo.toml
integrations/cellscript/scripts/reproduce-compiler.sh
```

The reproduction script builds the exact compiler/SDK revisions in
`adapter/cellscript-toolchain.lock.json` using the compiler's independent Rust
toolchain. The adapter itself uses this workspace's `rust-toolchain.toml`.
Each compile writes an ELF, upstream metadata, an integration compile receipt and
`<name>.artifact.json` in the [generic artifact format](../../docs/architecture/contract-artifacts.md).
The source hash and compiler identity are provenance; program semantics are
enforced by the selected VM, script and application configuration.

For the separate carrier drill, set `CKB_ROOT`, `CKB_BIN`, `CELLSCRIPT_BIN` and
`CELLSCRIPT_ATTESTATION`, then run:

```bash
integrations/cellscript/scripts/ckb-carriers.sh
```

See [compiler verification](compiler-boundary.md) and
[carrier acceptance](carrier-acceptance.md). Compiler reproduction is an explicit
integration action and is not a Myelin release prerequisite. The manual
`CellScript integration` workflow reproduces it independently from root CI.

The compiler target remains `ckb`. No `myelin` target, runtime dependency, old API
alias, source-required submission path or compatibility reader is retained.
