# Teeworlds Fixture Path

This document records the current bridge between Myelin and xxuejie's
Teeworlds-on-CKB repository. The fixture is a pressure workload for Myelin's
CKB-compatible path, not a claim that the phase-one runtime is already a
permissionless L2.

The intended public framing is narrow: Teeworlds is the first pressure workload
for a CKB-style session runtime. It is not proof that Myelin has completed
permissionless L2 security; it is executable evidence for replay shape,
projection reporting, single-chunk court input, and static-committee
finalisation.

The repository is cloned at:

```text
$HOME/RustroverProjects/teeworlds
```

The recommended acceptance gate is:

```bash
cd $MYELIN_ROOT
scripts/myelin_teeworlds_acceptance.sh
```

For the full Myelin release audit, including the stale-semantics scan,
dependency check, focused Rust tests, runtime smoke, Session L2 checks, and
both consensus modes, run:

```bash
cd $MYELIN_ROOT
scripts/myelin_production_gate.sh
```

By default it uses:

```text
TEEWORLDS_ROOT=$HOME/RustroverProjects/teeworlds
OUTPUT_DIR=/tmp/myelin-teeworlds-acceptance
TICKS=300
CLIENTS=1
INPUT_EVERY=5
SEED=1
RUNS=3
CHUNK_BYTES=262144
MAX_CYCLES=70000000
```

The gate rebuilds the deterministic scripted tape, invokes xxuejie's
`build-test-tx` through `myelin-cli teeworlds build-fixture`, runs the real
RISC-V replayer through `teeworlds vm-probe` in CKB-strict mode, builds a
single disputed-chunk court bundle, verifies the bundle, and validates the JSON
evidence. It fails unless the fixture and court chunk are `ckb-compatible`, CKB
projection is possible, the static closed committee finalises the benchmark
block, the VM probe succeeds, and every court-bundle verifier check passes.

The reusable fixture entry point is the existing Rust command:

```bash
cd $HOME/RustroverProjects/teeworlds/rust-tools
cargo run --bin teeworlds-cli -- utils build-scripted-tape \
  --ticks 300 \
  --clients 1 \
  --input-every 5 \
  --seed 1 \
  --output /tmp/myelin-teeworlds-scripted-tape.bin

cargo run --bin teeworlds-cli -- utils build-test-tx \
  --replayer ../ckb/build/replayer_stripped \
  --tape path/to/tape.bin \
  --map path/to/stripped.map \
  --config path/to/test_game1.cfg \
  --output path/to/teeworlds-mock-tx.json
```

That command packages the CKB-VM replayer binary, game tape, stripped map, and
game config into a CKB mock transaction. Myelin ingests this mock transaction
with:

```bash
cd $MYELIN_ROOT
cargo run -p myelin-cli -- teeworlds inspect \
  --mock-tx path/to/teeworlds-mock-tx.json \
  --chunk-bytes 262144 \
  --out reports/teeworlds-fixture.json

cargo run -p myelin-cli -- teeworlds bench \
  --mock-tx path/to/teeworlds-mock-tx.json \
  --chunk-bytes 262144 \
  --runs 3 \
  --out reports/teeworlds-bench.json

cargo run -p myelin-cli -- teeworlds court-bundle \
  --mock-tx path/to/teeworlds-mock-tx.json \
  --chunk-bytes 262144 \
  --chunk-index 0 \
  --out reports/teeworlds-court-bundle.json

cargo run -p myelin-cli -- teeworlds verify-court-bundle \
  --bundle reports/teeworlds-court-bundle.json \
  --out reports/teeworlds-court-bundle-verify.json

cargo run -p myelin-cli -- teeworlds doctor \
  --teeworlds-root $HOME/RustroverProjects/teeworlds \
  --out reports/teeworlds-doctor.json

cargo run -p myelin-cli -- teeworlds build-fixture \
  --teeworlds-root $HOME/RustroverProjects/teeworlds \
  --replayer path/to/replayer_stripped \
  --tape path/to/tape.bin \
  --map path/to/stripped.map \
  --config path/to/test_game.cfg \
  --mock-tx-output path/to/teeworlds-mock-tx.json \
  --chunk-bytes 262144 \
  --runs 3 \
  --out reports/teeworlds-build-fixture.json

cargo run -p myelin-cli -- teeworlds vm-probe \
  --replayer path/to/replayer_stripped \
  --tape path/to/tape.bin \
  --map path/to/stripped.map \
  --config path/to/test_game.cfg \
  --max-cycles 70000000 \
  --out reports/teeworlds-vm-probe.json
```

The current command inspects witness layout, treats witness 1 as the tape,
witness 2 as the stripped map, and witness 3 as the game config, splits the tape
into bounded chunks, emits chunk commitments, emits CKB-style projection status
for every chunk CellTx, measures fixture ingestion, and finalises a fixture block
via the phase-one static closed committee. The `build-fixture` command also
invokes xxuejie's Rust fixture builder first, so it is the direct integration
command for the cloned repository. `vm-probe` builds a Myelin `CellTx` with the
replayer as a type script, skips lock groups, and runs the type group through
Myelin's CKB-VM verifier using CKB-strict semantics. `court-bundle` emits a
self-contained court-input bundle for one disputed chunk: chunk payload bytes,
CKB Molecule transaction bytes, challenge payload hash, CKB projection report,
and static-committee evidence. `verify-court-bundle` recomputes the embedded
payload hash, Molecule transaction hash, projection fields, challenge hash, and
committee certificate. This is the executable court-input bundle; it is not yet
the CKB on-chain adjudication script.

The local environment now builds the real CKB replayer at
`$HOME/RustroverProjects/teeworlds/ckb/build/replayer_stripped`. The
build requires Homebrew CMake, LLVM tools, and `ld.lld`; on this macOS machine
the working CKB build command is:

```bash
cd $HOME/RustroverProjects/teeworlds/ckb
PATH="/opt/homebrew/opt/llvm/bin:$PATH" \
  make CLANG=/opt/homebrew/opt/llvm/bin/clang LD=/opt/homebrew/bin/ld.lld
```

Use `teeworlds doctor` before attempting the real replay. On the current
machine, it reports that the Rust fixture builder, generated Teeworlds sources,
CKB replayer build, and Myelin VM probe prerequisites are ready.

`vm-probe` has executed the real stripped RISC-V replayer through Myelin's
CKB-VM path. The current strongest successful probe uses a deterministic packed
scripted tape generated by `teeworlds-cli utils build-scripted-tape`, with a
scripted client connect/enter sequence, repeated direct and predicted inputs, a
real generated `dm1.map`, and a Teeworlds config. It preserves the CKB input
witness layout expected by the replayer:

```text
witness[1] = tape
witness[2] = map
witness[3] = config
```

Earlier four-byte and forced `change_map` tapes deliberately trapped at the
replayer assertion path because they were not valid replay streams. A local
live-client session was also attempted with the cloned fork's server and GUI
client, but the available launch paths did not produce a connected client or an
end-of-match sequencer dump in this environment. The scripted tape is therefore
the current deterministic pressure workload; the next stronger workload is a
live Teeworlds session harness that emits a tape from actual connected clients.

The intended flow is:

```text
Teeworlds mock transaction
  -> bounded replay chunks
  -> Myelin CellTx per chunk
  -> CKB projection status per chunk
  -> single-chunk court bundle + verifier when a chunk is disputed
  -> static-committee finalised benchmark block
  -> measured benchmark JSON
```

The preferred result for early demos is:

```text
semantic_profile = "ckb-compatible"
ckb_projection_possible = true
```

If a Teeworlds chunk requires Myelin-native shortcuts, the report must mark that
explicitly and treat the result as session-runtime evidence rather than
CKB-isomorphism evidence.

For that reason, Mode A is the default demo path. It preserves the CKB witness
contract and should keep:

```text
semantic_profile = "ckb-compatible"
ckb_projection_possible = true
```

Mode B may be useful later for native session ergonomics, but it must remain
clearly labelled until its chunks also produce successful projection reports.

Current `teeworlds inspect`, `teeworlds bench`, and `teeworlds build-fixture`
reports include:

```text
fixture.ckb_projection_possible
fixture.chunks[].ckb_projection.semantic_profile
fixture.chunks[].ckb_projection.ckb_projection_possible
fixture.chunks[].ckb_projection.ckb_raw_tx_hash
fixture.chunks[].ckb_projection.ckb_wtx_hash
```

This is projection evidence for the bounded chunk transaction shape. It is still
not a finished L1 court implementation. `teeworlds court-bundle` now emits the
single-chunk transaction/context material, payload bytes, Molecule transaction
bytes, and deterministic challenge hashes that the court path must later
consume inside an on-chain or formally specified adjudication flow.

Local adaptation applied to the cloned test repository:

```bash
cd $HOME/RustroverProjects/teeworlds/rust-tools
cargo update -p fixed --precise 1.30.0
cargo check
```

Reason: `fixed 1.31.0` currently requires Rust 1.93, while the available local
toolchain is `rustc 1.92.0-nightly`.

Additional macOS build adaptations applied in the cloned Teeworlds repository:

- `ckb/musl/ckb/build.sh` uses `sysctl -n hw.ncpu` when GNU `nproc` is absent.
- `ckb/libcxx/build.sh` creates and resolves its install directory in a
  Darwin-safe order.
- `ckb/Makefile` uses `-idirafter` for `ckb-c-stdlib/molecule` so its
  `version` file does not shadow the C++20 `<version>` standard header.
