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

> [!TIP]
> If you're on macOS and `cargo build` panics about `rlimit`, see the
> project-level notes in `AGENTS.md` — `ulimit -n 16384` before invoking
> cargo fixes it.

## 2. Clone and build Myelin

```bash
git clone https://github.com/Myelin-Labs/Myelin.git
cd Myelin

# Sanity check: formatting, clippy, tests
cargo fmt --all --check
cargo check --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
```

If all four pass, you have a working Myelin workspace.

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
CKB_ROOT=/home/arthur/a19q3/ckb \
RUN_TEEWORLDS=0 scripts/myelin_production_gate.sh
```

Set `RUN_CKB_DEVNET=0` only for a deliberately reduced local validation
when the parent checkout is unavailable.

## 5. Verify

A one-liner to confirm everything is plumbed correctly:

```bash
cargo run -p myelin-cli -- celltx simple-report
```

This writes a `MyelinExecutionReport` and a `CkbProjectionReport` for a
trivial CellTx. If you see `projection_stage = "wire-encoded"` and
`wire_encoded = true`, the toolchain is good. Head to
[First run](first-run.md) for the longer end-to-end path.
