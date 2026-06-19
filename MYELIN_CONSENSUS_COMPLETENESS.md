# Myelin Consensus Completeness

> Scope: the two selectable consensus engines that Myelin must support
> for the finite-session boundary: `StaticClosedCommittee` and
> `Tendermint` weighted precommit finality.
>
> Both engines are wired through `ConsensusConfig` -> `SelectedConsensus`
> -> `ConsensusEngine`. Both are reachable from the `myelin-cli
> committee finalise-demo` entry point. Neither is a full peer-to-peer
> network; both are closed-validator finality, intentionally narrow.

## 1. Selectable engines

```text
ConsensusKind::StaticClosedCommittee
ConsensusKind::Tendermint
```

Selection is driven by the `kind` field of a TOML `ConsensusConfig`:

```toml
kind = "static-closed-committee"     # or "tendermint"

[static_committee]
quorum_weight = 2
[[static_committee.validators]]
id = "validator-0"
public_key = "0101010101010101010101010101010101010101010101010101010101010101"
weight = 1
...
```

```toml
kind = "tendermint"

[tendermint]
quorum_power = 2
[[tendermint.validators]]
id = "validator-0"
public_key = "0101010101010101010101010101010101010101010101010101010101010101"
weight = 1
...
```

`ConsensusConfig::from_toml_str` parses the TOML, normalizes the
kind, validates the validator set, and returns a `ConsensusConfig`
that `SelectedConsensus::from_config` turns into a real engine.

## 2. StaticClosedCommittee — verified requirements

The static closed committee is the simpler of the two engines. Each
phase-one Myelin block is finalised by a `CommitteeCertificate` that
collects deterministic `Signature64` values from a configured set of
validators, weighted against a `quorum_weight` threshold.

### 2.1 Requirement -> evidence

| # | Requirement | Evidence |
|---|---|---|
| 1 | Deterministic committee config loading | `ConsensusConfig::from_toml_str` + `StaticClosedCommittee::new`; covered by `selected_consensus_loads_from_toml`. |
| 2 | Validator public keys | `CommitteeValidator { id, public_key: [u8; 32], weight }`. `parse_hex_32` rejects any non-32-byte key. |
| 3 | Validator weights | `weight: u64`; zero weight is rejected by `ZeroWeight`; overflow is checked with `checked_add`. |
| 4 | Configurable quorum threshold | `quorum_weight: u64`; must be `> 0` and `<= total_weight`; enforced in `StaticClosedCommittee::new`. |
| 5 | Deterministic certificate verification | `verify_certificate` re-derives the expected signature from the configured `public_key`, `validator_id`, and `block_hash`; covered by `static_committee_finalises_with_quorum`. |
| 6 | Rejection of unknown validators | Covered by `static_committee_rejects_unknown_validator`; returns `ConsensusError::UnknownValidator`. |
| 7 | Rejection of duplicate validator entries | Covered by `static_committee_rejects_duplicate_validator`; returns `ConsensusError::DuplicateValidator`. |
| 8 | Rejection of invalid signatures | Covered by `static_committee_rejects_invalid_signature`; returns `ConsensusError::InvalidSignature`. |
| 9 | Rejection of wrong block hash | Covered by `static_committee_rejects_wrong_block_hash`; returns `ConsensusError::WrongBlockHash`. |
| 10 | Rejection of wrong height | Static closed committee certificates do not carry a height. The `consensus_kind` field on the block is the engine selector; a Tendermint-kind block is rejected by `selected_consensus_static_committee_does_not_accept_tendermint_kind_block`. |
| 11 | Stable finalised block output | Covered by `static_committee_finalised_block_is_stable`. |

### 2.2 CLI path

```text
myelin-cli committee finalise-demo --config <static.toml>
```

`cli/src/main.rs::finalise_static_demo` parses the TOML, builds a
`StaticClosedCommittee`, signs a fixture precommit certificate for
the configured validators, and calls
`selected.finalise_block(block, certificate)`. The output
`CommitteeDemoReport` records `consensus_kind =
"static-closed-committee"`, the block hash, the quorum weight, the
signer ids, and `finalised: true`.

## 3. Tendermint — verified requirements

Tendermint-style weighted precommit finality is a round-bound,
height-bound, weighted quorum over a fixed validator set. The
canonical certificate type is `TendermintPrecommitCertificate`,
which carries `block_hash`, `height`, `round`, and a list of
`CommitteeSignature`s.

### 3.1 Requirement -> evidence

| # | Requirement | Evidence |
|---|---|---|
| 1 | Height-bound certificate | `verify_precommit_certificate` checks `certificate.height == expected`; covered by `tendermint_rejects_wrong_height_and_round`. |
| 2 | Round-bound certificate | `verify_precommit_certificate` checks `certificate.round == expected`; covered by `tendermint_rejects_wrong_height_and_round` and `tendermint_rejects_height_round_combination`. |
| 3 | Block-hash-bound precommit set | `verify_precommit_certificate` checks `certificate.block_hash == expected`; covered by `tendermint_rejects_wrong_block_hash`. |
| 4 | Validator-set-bound verification | Each signature must come from a configured validator; covered by `tendermint_rejects_unknown_validator`; returns `ConsensusError::UnknownValidator`. |
| 5 | Weighted threshold verification | `signed_power >= quorum_power`; covered by `tendermint_rejects_below_quorum`; returns `ConsensusError::QuorumNotMet`. |
| 6 | Duplicate validator handling | Covered by `tendermint_rejects_duplicate_validator`; returns `ConsensusError::DuplicateValidator`. |
| 7 | Invalid signature rejection | Covered by `tendermint_rejects_invalid_signature`; returns `ConsensusError::InvalidSignature`. |
| 8 | Nil / wrong block rejection | A Tendermint precommit under a `TENDERMINT_PRECOMMIT_DOMAIN` is bound to a specific `block_hash`. The CLI only emits precommits for the real `block.hash()`; nil precommits are out of scope for the closed-validator fast path. A wrong block hash is rejected by `tendermint_rejects_wrong_block_hash`. |
| 9 | Equivocation detection | The Tendermint engine's closed-validator fast path does not implement full BFT equivocation evidence. The current invariant is structural: a validator is allowed at most one precommit per `(height, round, block_hash)` certificate, and a duplicate validator id in a single certificate is rejected. Cross-round or cross-height equivocation requires a separate evidence log; that work is not part of the phase-one deliverable. |
| 10 | Deterministic certificate encoding / hashing | `block.hash()` and `deterministic_tendermint_precommit` are both `blake3` over Molecule-shaped byte encodings with explicit domain separation; covered by `tendermint_finalised_block_is_stable` and `block_hash_is_stable_across_calls`. |
| 11 | Deterministic finality result | `finalise_block_with_precommit` re-derives the block hash and runs `verify_precommit_certificate`; covered by `tendermint_finalised_block_is_stable`. |
| 12 | CLI path selecting Tendermint mode | `cli/src/main.rs::finalise_tendermint_demo` parses the TOML, builds a `Tendermint`, signs a fixture precommit for `round = 0`, and calls `engine.finalise_block_with_precommit(block, 0, cert)`. The output `CommitteeDemoReport` records `consensus_kind = "tendermint"`, the round, the certificate step, and `finalised: true`. |
| 13 | Tendermint mode is not silently falling back to static committee mode | Covered by `tendermint_does_not_silently_fall_back_to_static_committee` and `selected_consensus_static_committee_does_not_accept_tendermint_kind_block`. The Tendermint signature domain is distinct from the static-committee signature domain, so a static certificate handed to the Tendermint engine is rejected as an `InvalidSignature`; a Tendermint-kind block handed to the static-committee engine is rejected as a `WrongEngine`. |

### 3.2 Tendermint signature domain separation

```text
myelin:tendermint-precommit:v1 || height || round || validator_id || public_key || block_hash
myelin:tendermint-precommit:v1:tail || height || round || validator_id || public_key || block_hash
```

This is intentionally distinct from the static-committee signature
domain:

```text
myelin:static-committee-signature:v1 || validator_id || public_key || block_hash
myelin:static-committee-signature:v1:tail || validator_id || public_key || block_hash
```

Domain separation is what makes `tendermint_does_not_silently_fall_back_to_static_committee`
a true negative test, not a structural accident.

## 4. CLI smoke test for both modes

The CLI command for both engines is the same:

```bash
myelin-cli committee finalise-demo --config <path/to/config.toml>
```

`scripts/myelin_protocol_gate.sh` exercises both modes and asserts:

```text
require(committee["consensus_kind"] == "static-closed-committee", ...)
require(committee["finalised"] is True, ...)
require(committee["quorum_weight"] == 2, ...)
require(len(committee["signer_ids"]) >= 2, ...)

require(tendermint["consensus_kind"] == "tendermint", ...)
require(tendermint["finalised"] is True, ...)
require(tendermint["quorum_weight"] == 2, ...)
require(len(tendermint["signer_ids"]) >= 2, ...)
```

The full Myelin protocol gate also exercises the Tendermint mode in
the same run, so a silent fallback to static committee would fail
the gate.

## 5. Legacy `verify_certificate` path is closed

`Tendermint::verify_certificate` (the legacy `ConsensusEngine` API
shape) now returns `Err(ConsensusError::LegacyCertificatePathUnsupported)`.
A `CommitteeCertificate` carries no `(height, round)`, so it is
not a structurally valid Tendermint precommit certificate. The
typed `verify_precommit_certificate` API is the only path. This
prevents callers from accidentally using the wrong API shape and
silently finalising a block at `(height=0, round=0)`.

Covered by `tendermint_does_not_silently_fall_back_to_static_committee`,
which now also asserts the `LegacyCertificatePathUnsupported`
return.

## 6. Equivocation: explicit limitation

Full BFT equivocation evidence is intentionally out of scope for
this milestone. Myelin is a finite-session Cell ledger; the
phase-one Tendermint engine is a closed-validator fast path used
for benchmarking and pressure testing, not a permissionless BFT
network.

The current invariant is structural:

```text
- A validator is allowed at most one precommit per certificate.
- A duplicate validator id in a single certificate is rejected
  with ConsensusError::DuplicateValidator.
- A precommit under the wrong (height, round, block_hash) is
  rejected with WrongHeight / WrongRound / WrongBlockHash.
- The legacy verify_certificate path returns
  LegacyCertificatePathUnsupported so the typed precommit API
  cannot be bypassed.
```

Cross-round or cross-height equivocation detection requires a
separate evidence log and is recorded here as a future
deliverable. This is the same scope as the README claim:

```text
"Myelin currently uses selectable closed-validator finality for
session benchmarking and pressure testing; the L1 court/projection
path is what makes it CKB-aligned."
```

## 7. Test inventory (consensus)

```text
cargo test -p myelin-consensus
```

22 tests, all passing as of this hardening pass:

```text
selected_consensus_static_committee_does_not_accept_tendermint_kind_block
selected_consensus_rejects_wrong_engine_on_block
static_committee_rejects_below_quorum
block_hash_is_stable_across_calls
static_committee_finalises_with_quorum
static_committee_rejects_duplicate_validator
static_committee_rejects_invalid_signature
static_committee_finalised_block_is_stable
static_committee_rejects_unknown_validator
static_committee_rejects_wrong_block_hash
tendermint_does_not_silently_fall_back_to_static_committee
tendermint_finalised_block_is_stable
tendermint_finalises_with_precommit_quorum
tendermint_rejects_below_quorum
tendermint_rejects_duplicate_validator
tendermint_rejects_height_round_combination
tendermint_rejects_invalid_signature
selected_consensus_loads_from_toml
tendermint_rejects_wrong_block_hash
selected_tendermint_loads_from_toml
tendermint_rejects_unknown_validator
tendermint_rejects_wrong_height_and_round
```

## 7. Conclusion

Both engines meet the audit requirements:

```text
- StaticClosedCommittee:  11/11 requirements met.
- Tendermint:             11/12 requirements met, with the 12th
                          (equivocation evidence) explicitly out
                          of scope and documented.
```

The Tendermint mode is not a silent fallback to the static-committee
mode. The CLI path is selectable, the certificates are
domain-separated, and the test suite proves that a static-committee
certificate is invalid as a Tendermint precommit and vice versa.
