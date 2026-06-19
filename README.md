# Myelin

Myelin is a CKB-style isomorphic session runtime for typed Cell execution and
single-chunk L1 adjudication.

It is a protocol seed for an off-chain finite Cell ledger, not a CKB full-node
fork and not a new L1.

The precise early positioning is experimental: Myelin is a CKB-native
isomorphic session runtime, not a finished trustless or permissionless L2.
Static committee and Tendermint-style precommit finality are selectable session
fast paths; projection and the future L1 court path are the CKB alignment
boundary.

In one sentence: Myelin is an off-chain Cell session ledger. It moves
high-frequency state transitions off-chain, keeps them finite and typed, and
keeps disputed single-chunk verification aligned with CKB-VM semantics.

This repository intentionally removes the inherited L1/node surface: no PoW
consensus, no mining, no P2P block propagation, no full-node daemon, and no
L1 mempool/block-template stack. What remains is the kernel needed to build an
off-chain finite Cell ledger:

- `cellscript/` - the local CellScript fork with the `typed-cell` target profile.
- `exec/` - Cell transactions, lock/type script verification, VM/syscall glue,
  typed-cell scheduler witnesses, and CellDAG scheduling.
- `state/` - live Cell state roots and data-availability proof primitives.
- `mempool/` - Cell transaction pool and deterministic conflict scoring.
- `consensus/` - selectable finality engines: static closed committee and
  Tendermint-style weighted precommit finality over canonical session block
  hashes.
- `crypto/`, `math/`, `utils/` - local support crates retained by the execution
  and state kernel.

## Protocol Shape

Myelin is intended to evolve toward:

```text
CellScript source
  -> typed-cell metadata + VM artifact
  -> CellTx delta
  -> CellDAG conflict scheduler
  -> deterministic VM verification
  -> committed session Cell state root
```

The target protocol is not an independent L1. It is a fast finite Cell session
ledger whose state transitions should be reported as projectable into CKB-style
transaction contexts where possible, so a future L1 court can verify disputed
transitions and exits.

Current Myelin security is intentionally narrower than a finished permissionless
L2:
phase-one blocks use selectable closed-validator finality for session
benchmarking and pressure testing. The CKB-style projection and court path is
what keeps the runtime aligned with CKB semantics; it is not a claim of
permissionless security yet.

The correct early public claim is:

```text
Myelin currently uses selectable closed-validator finality for session
benchmarking and pressure testing; the L1 court/projection path is what makes
it CKB-aligned.
```

Core demos must prefer the `ckb-compatible` semantic profile:

```text
semantic_profile = "ckb-compatible"
ckb_projection_possible = true
```

`myelin-native` is allowed for experiments, but it should not be the default
path for early protocol evidence. The credibility target is not merely
"inspired by CKB"; it is a transition that can be reported as projectable into a
CKB-style transaction/context, or rejected with explicit deviation flags.

The claim ladder is:

```text
no projection report      -> designed to stay close to CKB semantics
successful projection     -> projectable into a CKB-style transaction/context
future exercised court    -> disputed chunk adjudicable by the CKB-aligned path
```

Static committee finality alone must not be marketed as permissionless L2
security.

## Current Bootstrap Scope

This repository cut keeps the typed-cell execution substrate and removes the
chain infrastructure that is not part of the Myelin L2 protocol. Current
executable evidence is deliberately narrow and should be read through that
bootstrap scope.

The immediate evidence targets are deliberately narrow:

```text
simple CellTx -> execution report
simple CellTx -> CKB projection report
Teeworlds fixture -> measured benchmark JSON with per-chunk CKB projection status
```

## Immediate Evidence Targets

The preferred full local protocol gate is:

```bash
scripts/myelin_protocol_gate.sh
```

It checks the active Myelin tree for removed source-chain and legacy serializer
vocabulary, proves the native dependency graph has no removed serializer
package, runs the focused Rust workspace checks, emits a simple CellTx execution
report, finalises demo blocks through the selected static closed-committee and
Tendermint consensus engines, and then runs the Teeworlds acceptance gate.

The narrower Teeworlds integration gate is:

```bash
scripts/myelin_teeworlds_acceptance.sh
```

It regenerates a deterministic scripted Teeworlds tape from xxuejie's cloned
repository, builds a CKB mock transaction, runs Myelin fixture ingestion, runs
the real RISC-V replayer through the CKB-strict VM probe, emits a disputed-chunk
court bundle, verifies that bundle, and rejects the run unless the output stays
`ckb-compatible`, projection-possible, and static-committee finalised.

The protocol evidence now has executable entry points for those targets:

```bash
cargo run -p myelin-cli -- celltx simple-report
cargo run -p myelin-cli -- committee finalise-demo --config path/to/static-committee.toml
cargo run -p myelin-cli -- teeworlds inspect --mock-tx path/to/teeworlds-mock-tx.json
cargo run -p myelin-cli -- teeworlds bench --mock-tx path/to/teeworlds-mock-tx.json --runs 3
cargo run -p myelin-cli -- teeworlds court-bundle \
  --mock-tx path/to/teeworlds-mock-tx.json \
  --chunk-index 0 \
  --out reports/teeworlds-court-bundle.json
cargo run -p myelin-cli -- teeworlds verify-court-bundle \
  --bundle reports/teeworlds-court-bundle.json
cargo run -p myelin-cli -- teeworlds doctor \
  --teeworlds-root /Users/arthur/RustroverProjects/teeworlds
cargo run -p myelin-cli -- teeworlds build-fixture \
  --teeworlds-root /Users/arthur/RustroverProjects/teeworlds \
  --replayer path/to/replayer_stripped \
  --tape path/to/tape.bin \
  --map path/to/stripped.map \
  --config path/to/test_game.cfg \
  --mock-tx-output path/to/teeworlds-mock-tx.json \
  --runs 3
cargo run -p myelin-cli -- teeworlds vm-probe \
  --replayer path/to/replayer_stripped \
  --tape path/to/tape.bin \
  --map path/to/stripped.map \
  --config path/to/test_game.cfg
```

The Teeworlds command consumes the CKB mock transaction produced by xxuejie's
`teeworlds-cli utils build-test-tx`, splits the tape witness into bounded
chunks, emits CKB-style projection status for every chunk CellTx, commits those
chunks, measures fixture ingestion, and finalises a benchmark block with the
phase-one static closed committee.
`teeworlds court-bundle` materialises one disputed chunk as a self-contained
court-input bundle: chunk payload bytes, CKB Molecule transaction bytes,
CKB-style projection evidence, deterministic challenge hashes, and
static-committee certificate evidence. `teeworlds verify-court-bundle`
recomputes those hashes, projection fields, and committee signatures. This is
the executable input shape for the future court path, not a claim that the CKB
on-chain court script is finished.
`teeworlds doctor` checks whether the cloned repository, generated Teeworlds
sources, LLVM tools, and `ckb/build/replayer_stripped` are ready for a real
CKB-VM replay.
`teeworlds vm-probe` builds the same witness layout and executes the replayer as
a type-script group through Myelin's CKB-VM verifier in CKB-strict mode. The
probe preserves the replayer's CKB witness contract: witness `1` is the tape,
witness `2` is the map, and witness `3` is the config.

## Requirements

- Rust 1.85 or newer.
- `pkg-config`, OpenSSL, Clang, and libclang for the retained native crates.

## Licence

Myelin keeps the inherited MIT licence.
