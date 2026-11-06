# Install the toolchain

This page walks you through everything you need to *build* the Myelin
workspace and *run* the CLI. The smoke test (live CKB devnet
submission) needs extra pieces — those are documented at the bottom.

## 1. Rust

Myelin's development toolchain is pinned by `rust-toolchain.toml`; its
declared workspace MSRV remains in `Cargo.toml`. Upstream CellScript is
an independent process boundary and uses the separate Rust toolchain
recorded in `cellscript-adapter/cellscript-toolchain.lock.json`.

```bash
# Install rustup if you don't already have it
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Add the RISC-V target that CKB scripts use
rustup target add riscv64imac-unknown-none-elf

# Verify
cargo --version
rustc --version
```

For OS packages and isolation prerequisites, see
[validation dependencies](../operations/production-gate.md#machines-and-dependencies).

## 2. Clone and build Myelin

```bash
git clone https://github.com/Myelin-Labs/Myelin.git
cd Myelin

# Formatting, all-target lint/type checking, workspace and process fixtures
scripts/myelin_production_gate.sh --profile core --out target/validation
```

Inspect `target/validation/report.json` for the executed scope and retained logs.
External integrations are explicitly skipped by the core profile.

## 3. Build the CLI

```bash
cargo build -p myelin-cli --release
```

The binary lands at `target/release/myelin`. From here on, the docs
will just say `cargo run -p myelin-cli -- …` for brevity, but you can
swap that for the release binary whenever you want a faster shell.

## 4. Parent CKB devnet for the full gate

The production gate starts an isolated integration chain from the parent
CKB checkout, reproduces the exact CellScript compiler, deploys four
verifiers, and exercises valid and invalid transactions:

```bash
CKB_ROOT=/absolute/ckb scripts/myelin_production_gate.sh --profile ckb --out target/ckb-validation
```

The selected CKB profile requires both compiler reproduction and devnet acceptance;
missing inputs fail that profile. Use `--profile core` for independent core work.

## 5. Verify

A one-liner to confirm everything is plumbed correctly:

```bash
cargo run -p myelin-cli -- celltx simple-report
```

This writes a `MyelinExecutionReport` and a `CkbProjectionReport` for a
trivial CellTx. If you see `projection_stage = "wire-encoded"` and
`wire_encoded = true`, the toolchain is good. Head to
[First run](first-run.md) for the longer end-to-end path.
