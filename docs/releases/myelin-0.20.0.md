# Myelin 0.20.0

- Release date: 2026-08-25
- Tag: `v0.20.0`
- Previous release: `v0.10.0`
- Functional baseline: `bd593701143c39f929aa5333d614f4ec5d50c8b1`

Myelin 0.20.0 is the first release of the modular continuous-session runtime.
It keeps deterministic finite-Cell execution independent from block-production
timing and closed-validator finality, then binds the chosen module and its exact
configuration into session genesis, storage, network envelopes, WAL records,
blocks, proofs, and recovery.

This release is for controlled sessions, integration work, and benchmarking.
It is not a CKB full node, a new L1, a public-testnet court, or a finished
permissionless L2.

## What changed

- Added the continuous session kernel, RocksDB store, module-neutral network,
  supervised runtime composition root, configurable producer, optional escrow,
  and wallet authentication crates.
- Added a closed catalog for static committee, proof of authority, and
  Tendermint. All three finalize the same canonical state transition through
  different typed proof and signature domains.
- Added paged recovery audit, rolling checkpoints, durable queues, atomic
  outbox commits, and bounded reserving production batches.
- Added provider-neutral DA certificates and strengthened DA, settlement, and
  court-package evidence binding.
- Added archived, independently re-verifiable public CKB testnet evidence for
  standard-lock and multisig-v2 transactions.
- Updated the pinned CellScript process boundary and exercised its exact CKB
  witness ABI and attestation chain.

## Commit audit

The reviewed range is `v0.10.0..bd59370`. Every commit is included in the
functional baseline. The release commit adds version metadata, this record,
the changelog entry and release index link. It also replaces one manual
remainder check with the equivalent current Rust integer API required by the
warnings-denied Clippy gate, and removes stale pre-release version suffixes
from three draft OpenStrike identity domains before any compatibility promise
exists.

| Commit | Review | Release effect |
| --- | --- | --- |
| `d6e6cb7` | Accepted | Reframed the README without changing protocol behavior. |
| `5d941e9` | Accepted | Added complete closed-validator Tendermint rounds, provider-neutral DA evidence, strict Molecule handling, and matching gates. |
| `e074e65` | Accepted | Added typed PoA seals and three-engine dispatch; claims remain explicitly closed-validator. |
| `99d3e09` | Accepted | Added CKB multisig rehearsal commands and archived non-secret public-testnet evidence. |
| `7deb8e9` | Accepted | Bound the CellScript adapter to the exact upstream witness ABI and extended parent-CKB smoke coverage. |
| `de8d99b` | Accepted | Introduced the session, RocksDB, network, runtime, escrow, and wallet-auth module boundaries plus typed proof codecs. |
| `35896f5` | Accepted | Replaced unbounded recovery and queue growth with rolling checkpoints, pagination, quotas, and durable acknowledgements. |
| `c206b1b` | Accepted | Added bounded reserving production and explicit `Instant`, `Interval`, `Open`, `Never`, and manual policies. |
| `c4effeb` | Accepted | Added design records and release illustrations; no runtime claim is derived from those assets. |
| `d672f3f` | Accepted | Synchronized the README and wiki with the implemented continuous-session boundaries. |
| `bd59370` | Accepted | Corrected the wiki build instructions and closes the 0.20 functional range. |

## Review conclusions

- The module catalog is deliberately compiled in and closed. This release does
  not claim dynamic loading or an unrestricted third-party plugin surface.
- Finality cannot validate an otherwise invalid transition: state execution and
  exact pre/post roots remain consensus-independent.
- A stored session cannot be reopened under a substituted engine, authority
  set, message codec, proof codec, or WAL schema.
- Producer policies decide when a bounded candidate closes, never whether its
  transactions are valid or final.
- Public-testnet records prove only the exact CKB transactions and receipt
  stages they contain. They do not prove a deployed public-testnet Myelin court.

## Validation record

The release tree passed the following checks on 2026-08-25:

```bash
cargo fmt --all --check
cargo check --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
ALLOW_SKIP_TEEWORLDS=1 scripts/myelin_production_gate.sh
```

The production gate passed its parent CKB devnet smoke, including live
deployment, submission, commitment checks, and rejection of replayed or
tampered settlement evidence. Teeworlds is an external, non-vendored workload;
its checkout and replayer were unavailable, so that workload was explicitly
skipped. The skip is not a passing Teeworlds integration result.

## Upgrade note

Myelin had not made a public compatibility commitment for the pre-0.20 session
formats. This release replaces those internal formats directly. Existing local
session databases and JSON reports should be regenerated rather than migrated
through an alias or legacy decoder.

## Remaining work

- Deploy and exercise the exact final verifier path on public CKB testnet.
- Complete disputed-chunk adjudication and court economics.
- Integrate durable external DA with independent retrieval and failure drills.
- Establish production validator/operator custody, rotation, monitoring, and
  incident-response procedures.
- Add broader differential, property, adversarial, contention, and long-running
  recovery tests.
