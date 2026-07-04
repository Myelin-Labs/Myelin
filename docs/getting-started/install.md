# Install the toolchain

This page walks you through everything you need to *build* the Myelin
workspace and *run* the CLI. The smoke test (live CKB devnet
submission) needs extra pieces — those are documented at the bottom.

## 1. Rust

Myelin tracks stable Rust. The workspace is built against the same
`rust-version` as the parent CellScript project.

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
git clone https://github.com/Myelin-Network/Myelin.git
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

The binary lands at `target/release/myelin-cli`. From here on, the docs
will just say `cargo run -p myelin-cli -- …` for brevity, but you can
swap that for the release binary whenever you want a faster shell.

## 4. (Optional) Local CKB devnet for live smoke tests

The `scripts/myelin_ckb_devnet_smoke.sh` script submits carrier
transactions to a real CKB devnet. To run it locally:

```bash
# Either use OffCKB (the recommended path)
# see https://docs.nervos.org/docs/node/install-ckb for current install
offckb init --ckb-version latest
offckb start

# ... or use a parent ckb checkout, if you maintain one
cd ../ckb
cargo build --release
target/release/ckb init --testnet
target/release/ckb run --testnet --tmp --listen 127.0.0.1:8114
```

You should see CKB's RPC listener on `127.0.0.1:8114`. The smoke script
will talk to it over JSON-RPC.

## 5. Verify

A one-liner to confirm everything is plumbed correctly:

```bash
cargo run -p myelin-cli -- celltx simple-report
```

This writes a `MyelinExecutionReport` and a `CkbProjectionReport` for a
trivial CellTx. If you see `semantic_profile = "ckb-compatible"` and
`ckb_projection_possible = true`, the toolchain is good. Head to
[First run](first-run.md) for the longer end-to-end path.