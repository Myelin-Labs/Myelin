# Myelin Architecture

Myelin is a CKB-style isomorphic session runtime for typed Cell execution and
single-chunk L1 adjudication. It is a protocol seed for an off-chain finite Cell
ledger. It is not a CKB full-node fork, not a new L1, and not a finished
permissionless L2. CKB is the semantic reference for Cells, scripts, witnesses,
dep groups, and VM-compatible verification; Myelin remains an off-chain finite
Cell ledger with its own scheduler, state root, finality, and benchmark
pipeline.

The first pressure workload is xxuejie's Teeworlds-on-CKB repository, cloned at:

```text
/Users/arthur/RustroverProjects/teeworlds
```

The target execution flow is:

```text
CellScript source
  -> typed-cell metadata + VM artefact
  -> Myelin CellTx
  -> CellDAG scheduler
  -> deterministic script verification
  -> Cell state root
  -> selectable closed-validator finality
  -> Teeworlds pressure-test report
```

## Design Rules

Myelin state is a finite Cell set. A transaction consumes Cells, creates Cells,
reads explicit dependencies, carries witnesses, and produces a deterministic
state-root transition. There is no global account store and no mutable contract
storage hidden behind an address.

Myelin must not import or prune the CKB client. It may use CKB-style concepts,
fixtures, Molecule-compatible data, CKB-VM-compatible binaries, and small
libraries where they are the correct narrow dependency. A CKB projection layer
answers whether a Myelin transition can be represented as a CKB-style
transaction/context; it does not run a CKB node.

Current Myelin security should be described precisely: selectable
closed-validator finality is a fast path for session benchmarking and pressure
testing. Projection reports show whether a transition is CKB-style projectable;
the future L1 court path is what can make disputed chunks adjudicable on CKB.
Until that court path is implemented and exercised, Myelin should not be
described as a permissionless L2.

The strongest current public description is therefore: an experimental
CKB-native isomorphic session runtime, using Teeworlds as the first pressure
workload. It is credible protocol evidence only when the execution report,
projection report, benchmark report, and committee certificate are kept
separate and explicit.

The concise positioning is:

```text
Myelin is a CKB-style isomorphic session runtime: a finite off-chain Cell
ledger with typed conflict scheduling, deterministic VM verification, and a
future CKB-style court path for disputed chunks.
```

The claim ladder must remain explicit:

```text
no projection report      -> designed to stay close to CKB semantics
successful projection     -> projectable into a CKB-style transaction/context
court bundle              -> executable disputed-chunk input shape
future exercised court    -> CKB-aligned adjudication path
```

Static committee finality is the present fast path; it is not permissionless
security by itself.

## Retained Kernel

Keep and harden:

- `cellscript`: compiler and typed-cell metadata producer.
- `myelin-exec`: CellTx, script groups, VM/syscall adapters, CellDAG scheduler.
- `myelin-state`: live/consumed/created Cell state, state root, proof objects.
- `myelin-mempool`: admission queue, fee/cycle scoring, RBF, dependency
  tracking.
- `myelin-consensus`: explicit consensus selection with static closed-committee
  and Tendermint-style weighted precommit finality.
- `myelin-hashes`, `myelin-math`, `myelin-muhash`, `myelin-utils`: deterministic
  hashing, integer, accumulator, and support code.

Extend:

- typed-cell metadata ingestion at admission time.
- scheduler witness and conflict-hash verification.
- static committee certificates and Tendermint precommit certificates over
  deterministic block hashes.
- CKB-style projection reports for CellTxs and execution chunks. This is the
  credibility hinge: every serious demo should say whether the transition is
  projectable into a CKB-style transaction/context, or list explicit deviation
  flags.
- Teeworlds fixture ingestion and benchmark reporting.

Simplify or rewrite:

- any admission rule that cannot be explained by explicit read/write/conflict
  sets.
- any execution path that relies on wall-clock state, random host state, or
  hidden IO.
- legacy compatibility branches once equivalent CKB-profile behaviour is
  explicit and testable.

Delete or disable:

- mining, node sync, PoW, full-node RPC, wallet, and consensus experiments.
- demo-only script paths that conflict with deterministic Cell execution.

## Core Data Model

```rust
pub struct MyelinCellState {
    pub live_cells: Vec<LiveCell>,
    pub consumed_cells: Vec<OutPoint>,
    pub created_cells: Vec<CellOutput>,
    pub cell_deps: Vec<CellDep>,
    pub context_deps: Vec<ContextDep>,
    pub witnesses: Vec<Vec<u8>>,
    pub state_root: [u8; 32],
}

pub struct MyelinBlock {
    pub version: u32,
    pub parent_hash: [u8; 32],
    pub number: u64,
    pub timestamp_ms: u64,
    pub consensus_kind: ConsensusKind,
    pub state_root_before: [u8; 32],
    pub state_root_after: [u8; 32],
    pub ordered_cell_tx_commitments: Vec<[u8; 32]>,
    pub data_commitments: Vec<[u8; 32]>,
    pub scheduler_commitment: [u8; 32],
}
```

Block hash is a canonical hash over the exact Molecule-shaped serialised block
header and commitments. Tests must cover hash stability for the same input and
hash change under any field mutation.

## Typed-Cell Metadata

CellScript's `typed-cell` profile is the compiler contract between application
code and runtime admission. It must emit:

- typed Cell identity.
- typed data hash.
- conflict keys, including composite keys.
- scheduler witness.
- conflict hash.
- proof obligations.
- VM artefact or script reference.

The runtime verifies the metadata before scheduling:

```text
declared typed data hash == hash(actual output data, declared type schema)
declared conflict hash  == hash(read set, write set, conflict domains)
scheduler witness       == canonical witness over CellTx + metadata
script deps             == referenced code Cells or compatible CKB dep group
```

Rejected transactions are classified as:

```text
invalid-conflict-key
invalid-typed-data-hash
invalid-witness
dependency-blocked
write-conflicting
script-dep-mismatch
```

## Scheduler

The scheduler is an admission and audit component, not just an optimisation. It
builds a CellDAG from:

- input OutPoints.
- consumed Cells.
- created Cells.
- read-only referenced Cells.
- typed-cell conflict hashes.
- declared read/write conflict domains.

It emits:

```rust
pub struct MyelinSchedulerReport {
    pub accepted: Vec<[u8; 32]>,
    pub rejected: Vec<[u8; 32]>,
    pub dag_nodes: Vec<SchedulerNode>,
    pub dag_edges: Vec<SchedulerEdge>,
    pub conflict_domains: Vec<ConflictDomain>,
    pub parallel_batches: Vec<Vec<[u8; 32]>>,
    pub rejected_reasons: Vec<RejectedReason>,
    pub report_hash: [u8; 32],
}
```

The deterministic ordering is `fee_density` followed by `wtxid`. There is no
L1 consensus weighting in transaction priority.

## Execution Report

Every CellTx or chunk produces:

```rust
pub struct MyelinExecutionReport {
    pub accepted: bool,
    pub vm_exit_code: i8,
    pub cycles: u64,
    pub consumed_cells: Vec<OutPoint>,
    pub created_cells: Vec<CellOutput>,
    pub read_refs: Vec<OutPoint>,
    pub witness_hashes: Vec<[u8; 32]>,
    pub script_deps: Vec<CellDep>,
    pub conflict_hashes: Vec<[u8; 32]>,
    pub typed_data_hashes: Vec<[u8; 32]>,
    pub scheduler_report_hash: [u8; 32],
    pub state_root_before: [u8; 32],
    pub state_root_after: [u8; 32],
    pub semantic_profile: SemanticProfile,
}
```

Semantic profiles:

- `myelin-native`: uses Myelin-only helper syscalls or metadata.
- `ckb-compatible`: can be projected into a CKB-style transaction/context.
- `ckb-inspired-only`: follows the Cell model but has unsupported projection
  flags.

Early core demos must target `ckb-compatible` first and should emit:

```text
semantic_profile = "ckb-compatible"
ckb_projection_possible = true
```

`myelin-native` remains available for experiments, but it is not the default
evidence path for a CKB-isomorphic runtime. `ckb-inspired-only` is an explicit
deviation label, not a marketing profile. If a demo cannot produce
`ckb_projection_possible = true`, it should be treated as experimental Myelin
runtime evidence, not as CKB-isomorphism evidence.

For early protocol discussions, `ckb-compatible` is the default acceptance
profile. `myelin-native` can exist for engineering experiments, but it should
not carry the main L2/isomorphism claim.

## Selectable Finality

Phase one finality is selectable. A static closed committee is configured from
TOML:

```toml
kind = "static-closed-committee"

[static_committee]
quorum_weight = 2

[[static_committee.validators]]
id = "validator-0"
public_key = "0101010101010101010101010101010101010101010101010101010101010101"
weight = 1

[[static_committee.validators]]
id = "validator-1"
public_key = "0202020202020202020202020202020202020202020202020202020202020202"
weight = 1
```

Tendermint-style weighted precommit finality is configured from TOML:

```toml
kind = "tendermint"

[tendermint]
quorum_power = 2

[[tendermint.validators]]
id = "validator-0"
public_key = "0101010101010101010101010101010101010101010101010101010101010101"
weight = 1

[[tendermint.validators]]
id = "validator-1"
public_key = "0202020202020202020202020202020202020202020202020202020202020202"
weight = 1

[[tendermint.validators]]
id = "validator-2"
public_key = "0303030303030303030303030303030303030303030303030303030303030303"
weight = 1
```

The consensus trait must allow:

```rust
pub trait ConsensusEngine {
    fn kind(&self) -> ConsensusKind;
    fn verify_certificate(&self, block_hash: [u8; 32], cert: &CommitteeCertificate) -> Result<()>;
    fn finalise_block(&self, block: MyelinBlock, cert: CommitteeCertificate) -> Result<FinalisedBlock>;
}
```

`StaticClosedCommittee` and `Tendermint` are selected through
`SelectedConsensus::from_config`. Consensus changes stay behind the same
`ConsensusEngine` trait, with explicit selection in configuration. The trust
model is direct: a quorum of configured validators finalises Myelin blocks.
This is useful for sessions and pressure testing; it is not permissionless
security.

Public wording should use this boundary:

```text
Myelin currently uses selectable closed-validator finality for session
benchmarking and pressure testing; the L1 court/projection path is what makes
it CKB-aligned.
```

## CKB-Style Projection

Projection input is a CellTx or execution chunk. Projection output is:

```rust
pub struct CkbProjectionReport {
    pub projection_possible: bool,
    pub ckb_style_tx_hash: Option<[u8; 32]>,
    pub cell_inputs: Vec<OutPoint>,
    pub cell_outputs: Vec<CellOutput>,
    pub cell_deps: Vec<CellDep>,
    pub witnesses: Vec<Vec<u8>>,
    pub script_groups: Vec<ScriptGroup>,
    pub unsupported_features: Vec<String>,
    pub semantic_deviation_flags: Vec<String>,
}
```

Projection succeeds when all consumed Cells, produced Cells, deps, witnesses,
script groups, and VM syscalls can be represented in a CKB-style context.
Projection fails explicitly when Myelin-only metadata, helper syscalls, session
shortcuts, or DA commitments cannot be encoded without changing semantics.

The projection report is the credibility hinge. Before a transition has a
projection report, Myelin can only say it is designed to stay close to CKB
semantics. Once the report exists, that transition can claim a concrete result:
projectable into a CKB-style transaction/context, or not projectable with
explicit deviation flags.

This is the boundary between "CKB-style" and "CKB-aligned evidence". Public
examples should show the projection result next to the execution result, rather
than burying it inside a benchmark.

The first implementation target is deliberately small:

```text
simple CellTx -> CKB projection report
```

The same projection status is now attached to each bounded Teeworlds execution
chunk by the `teeworlds inspect`, `bench`, and `build-fixture` commands. This
is still projection evidence for the chunk transaction shape, not a completed
L1 court implementation. `teeworlds court-bundle` materialises one disputed
chunk as a self-contained court-input bundle with payload bytes, CKB Molecule
transaction bytes, challenge payload hash, projection report, and
static-committee evidence for the future court path. `teeworlds
verify-court-bundle` recomputes the bundle evidence.

## Session And Court Model

The session path is:

```text
asset custody: canonical CKB-style Cells
session entry: lock or commit Cells into a session
fast path: static-committee Myelin session runtime
DA path: publish chunk commitments
court path: one disputed chunk is CKB-VM-style verifiable
exit path: final state unlocks or materialises Cells
```

CKB is the custody and court layer here, not the real-time game server. The
first adjudication target is single-chunk verification; interactive bisection is
a later fallback, not the core bootstrap assumption.

Core objects:

```rust
pub struct SessionConfig {
    pub session_id: [u8; 32],
    pub participants: Vec<Participant>,
    pub committee: CommitteeConfig,
    pub max_chunk_bytes: u64,
    pub max_cycles: u64,
}

pub struct SessionChunk {
    pub session_id: [u8; 32],
    pub index: u64,
    pub old_state_root: [u8; 32],
    pub payload_hash: [u8; 32],
    pub new_state_root: [u8; 32],
    pub scheduler_report_hash: [u8; 32],
}

pub struct ChallengePayload {
    pub old_state_root: [u8; 32],
    pub chunk_payload: Vec<u8>,
    pub new_state_root: [u8; 32],
    pub script_deps: Vec<CellDep>,
    pub witnesses: Vec<Vec<u8>>,
    pub scheduler_witnesses: Vec<Vec<u8>>,
    pub committee_certificate_or_evidence: Vec<u8>,
}
```

The court path favours single-chunk verification. Interactive bisection is a
fallback design, not the first target.

The executable Session L2 fixture is exposed through:

```bash
cargo run -p myelin-cli -- session open-fixture --out reports/session-open.json
cargo run -p myelin-cli -- session commit-fixture --session reports/session-open.json --out reports/session-commit.json
cargo run -p myelin-cli -- session court-bundle --commit reports/session-commit.json --chunk-index 0 --out reports/session-court-bundle.json
cargo run -p myelin-cli -- session verify-court-bundle --bundle reports/session-court-bundle.json --out reports/session-court-verify.json
cargo run -p myelin-cli -- session da-manifest --bundle reports/session-court-bundle.json --storage-dir reports/session-da-store --out reports/session-da-manifest.json
cargo run -p myelin-cli -- session verify-da-manifest --manifest reports/session-da-manifest.json --bundle reports/session-court-bundle.json --storage-dir reports/session-da-store --out reports/session-da-verify.json
cargo run -p myelin-cli -- session da-anchor-package --manifest reports/session-da-manifest.json --bundle reports/session-court-bundle.json --out reports/session-da-anchor-package.json
cargo run -p myelin-cli -- session verify-da-anchor-package --package reports/session-da-anchor-package.json --manifest reports/session-da-manifest.json --bundle reports/session-court-bundle.json --out reports/session-da-anchor-verify.json
cargo run -p myelin-cli -- session submit-da-anchor-package --package reports/session-da-anchor-package.json --dry-run --out reports/session-da-anchor-submit.json
cargo run -p myelin-cli -- session verify-submission-context --submission reports/session-da-anchor-submit.json --rpc-url http://127.0.0.1:8114 --out reports/session-da-anchor-context.json
cargo run -p myelin-cli -- session verify-submission-economics --submission reports/session-da-anchor-submit.json --rpc-url http://127.0.0.1:8114 --min-fee-shannons 1 --min-fee-rate-shannons-per-kb 1000 --max-fee-shannons 1000 --out reports/session-da-anchor-economics.json
cargo run -p myelin-cli -- session verify-submission-inclusion --submission reports/session-da-anchor-submit.json --rpc-url http://127.0.0.1:8114 --min-status committed --out reports/session-da-anchor-inclusion.json
cargo run -p myelin-cli -- session verify-submission-stability --inclusion reports/session-da-anchor-inclusion.json --rpc-url http://127.0.0.1:8114 --out reports/session-da-anchor-stability.json
cargo run -p myelin-cli -- session verify-submission-finality --inclusion reports/session-da-anchor-inclusion.json --rpc-url http://127.0.0.1:8114 --min-confirmations 6 --out reports/session-da-anchor-finality.json
cargo run -p myelin-cli -- session verify-submission-readiness --context reports/session-da-anchor-context.json --economics reports/session-da-anchor-economics.json --inclusion reports/session-da-anchor-inclusion.json --stability reports/session-da-anchor-stability.json --finality reports/session-da-anchor-finality.json --out reports/session-da-anchor-readiness.json
cargo run -p myelin-cli -- session settlement-intent --bundle reports/session-court-bundle.json --da-manifest reports/session-da-manifest.json --kind disputed-close --current-time-ms 60000 --challenge-window-ms 60000 --out reports/session-settlement-intent.json
cargo run -p myelin-cli -- session verify-settlement-intent --intent reports/session-settlement-intent.json --bundle reports/session-court-bundle.json --da-manifest reports/session-da-manifest.json --out reports/session-settlement-verify.json
cargo run -p myelin-cli -- session settlement-package --intent reports/session-settlement-intent.json --bundle reports/session-court-bundle.json --da-manifest reports/session-da-manifest.json --out reports/session-settlement-package.json
cargo run -p myelin-cli -- session verify-settlement-package --package reports/session-settlement-package.json --intent reports/session-settlement-intent.json --bundle reports/session-court-bundle.json --da-manifest reports/session-da-manifest.json --out reports/session-settlement-package-verify.json
cargo run -p myelin-cli -- session submit-settlement-package --package reports/session-settlement-package.json --dry-run --out reports/session-settlement-submit.json
cargo run -p myelin-cli -- session verify-submission-context --submission reports/session-settlement-submit.json --rpc-url http://127.0.0.1:8114 --out reports/session-settlement-context.json
cargo run -p myelin-cli -- session verify-submission-economics --submission reports/session-settlement-submit.json --rpc-url http://127.0.0.1:8114 --min-fee-shannons 1 --min-fee-rate-shannons-per-kb 1000 --max-fee-shannons 1000 --out reports/session-settlement-economics.json
cargo run -p myelin-cli -- session verify-submission-inclusion --submission reports/session-settlement-submit.json --rpc-url http://127.0.0.1:8114 --min-status committed --out reports/session-settlement-inclusion.json
cargo run -p myelin-cli -- session verify-submission-stability --inclusion reports/session-settlement-inclusion.json --rpc-url http://127.0.0.1:8114 --out reports/session-settlement-stability.json
cargo run -p myelin-cli -- session verify-submission-finality --inclusion reports/session-settlement-inclusion.json --rpc-url http://127.0.0.1:8114 --min-confirmations 6 --out reports/session-settlement-finality.json
cargo run -p myelin-cli -- session verify-submission-readiness --context reports/session-settlement-context.json --economics reports/session-settlement-economics.json --inclusion reports/session-settlement-inclusion.json --stability reports/session-settlement-stability.json --finality reports/session-settlement-finality.json --out reports/session-settlement-readiness.json
```

For external DA evidence, `session da-manifest` can additionally take
`--external-da-receipt reports/external-da-receipt.json`. The receipt must use
`myelin-external-da-receipt-v2`, must bind to the same payload hash and segment
root as the manifest, and must carry a provider recoverable secp256k1 signature
over the receipt fields. For production DA readiness, the signed receipt must
also cover `service_level = "production"`, a retention window of at least 30
days, an HTTPS retrieval endpoint, and a 32-byte audit-log commitment. The
signed receipt and SLA fields are hashed into the DA availability commitment.
The default production gate path omits this receipt and therefore remains
explicitly local-only DA evidence.

Production operations evidence can be bound into either readiness command with
`--operator-custody-policy reports/operator-custody-policy.json` and
`--operator-runbook reports/operator-runbook.json`. The custody document must use
`myelin-operator-custody-policy-v1`; the runbook must use
`myelin-operator-runbook-v1`. Both files are schema checked, typed-control
checked, and hashed into the `operational_policy` commitment. The custody
document must declare hardware-backed keys, dual-control signing, tested
rotation and emergency drills, plus a valid signing threshold; the runbook must
match the readiness report's confirmation depth and fee policy while declaring
stability requery, bounded retries, monitoring cadence, and escalation contact.

This fixture is intentionally small, but it exercises the real spine:
`CellPool` admission, `CellStateTree` mutation, canonical `MyelinBlock`
finality, CKB-compatible transaction projection, and court-bundle
recomputation. Static-closed-committee and Tendermint runs produce the same
session id, CellTx commitments, scheduler commitment, and state roots; their
finality evidence remains consensus-domain separated. The DA manifest step emits and verifies a Merkle
`SegmentProof` for the exact Molecule transaction bytes needed by court replay,
while deliberately keeping `l1_da_published = false`. With `--storage-dir`, it
uses `SegmentWriter` / `SegmentReader` to prove sealed local DA storage. With an
external DA receipt, it also schema-checks, provider-signature-checks, and
commitment-binds provider receipt evidence for the same payload hash and segment
root, which can make DA availability `testnet_beta_ready`. A provider-signed
production SLA receipt can additionally make DA availability `production_ready`;
without that signed SLA, the report continues to avoid claiming production DA or
L1 publication. The DA anchor-package step converts that
verified manifest into a deterministic CKB-compatible anchor CellTx package and
verifies the embedded Molecule transaction, CellTx ids, and projection. It
still keeps `l1_da_publication_implemented = false`; the
submit-da-anchor-package step then builds the CKB `send_transaction` JSON-RPC
request and can submit it to a configured HTTP CKB RPC endpoint. In the normal
production gate this runs as a dry-run, so it proves request construction but
not live public-chain inclusion. `verify-submission-context` queries CKB
`get_live_cell` for the submitted transaction's inputs and cell deps, catching
missing or spent cells before a live submit. `verify-submission-economics`
queries live input capacity, sums output capacity, computes the implied fee, and
requires the configured absolute fee floor, fee-rate policy, and optional
maximum-fee policy before a live submit.
`verify-submission-inclusion` then
queries CKB `get_transaction` for a submission report and marks
`live_l1_observed = true` only for committed transactions with a block hash; the
`verify-submission-stability` step re-queries that committed transaction and
requires the block hash and block number to remain unchanged. The
`verify-submission-finality` step queries CKB `get_tip_header` and requires the
committed inclusion to reach the configured confirmation depth.
`verify-submission-readiness` aggregates those five reports and requires the
same CKB transaction hash, same committed block identity, and all readiness
markers before it emits `production_submission_ready = true`. It separately
emits `final_l1_script_submission_ready` for live final-script evidence and
`end_to_end_production_ready` plus `end_to_end_production_blockers` for the
larger production claim, so devnet final-script acceptance cannot mask missing
DA availability guarantees, canonical threshold-lock enforcement, deployed CKB
court economics, or operator custody/runbook evidence. The production gate
covers context, economics, inclusion, stability, finality, and aggregate
readiness verifiers with mock CKB RPC servers. The settlement-intent step binds the
verified court bundle and verified DA manifest to a disputed-close decision and
elapsed challenge window, while deliberately keeping `l1_da_published = false`
and `l1_court_implemented = false` in the report. It is the off-chain
settlement readiness object. The settlement-package step converts that permitted
intent into a deterministic CKB-compatible settlement CellTx package and
verifies the embedded Molecule transaction, CellTx ids, and CKB projection. It
still keeps `l1_court_script_implemented = false`. The submit-settlement-package
step builds the CKB `send_transaction` JSON-RPC request and can submit it to a
configured HTTP CKB RPC endpoint. In the normal production gate this also runs
as a dry-run, so it proves request construction but not deployed court-script
semantics or live public-chain inclusion.

For live local CKB evidence, `scripts/myelin_ckb_devnet_smoke.sh` starts the
parent `../ckb` devnet, mines an always-success funding cell, deploys separate
DA-anchor and settlement CellScript carrier verifiers, writes a 160-byte Myelin
DA-anchor payload into a CKB carrier output guarded by the DA verifier, and binds
the carrier type args to `ckb_data_hash(carrier_payload) ||
carrier_identity_hash`, where the identity hash is the DA manifest hash or
settlement intent hash. The settlement
carrier is funded from the DA carrier's change output and uses the settlement
verifier with its own 160-byte compact payload. It uses `data2` for both
CellScript verifier type scripts, submits the valid carriers through
`myelin session carrier-submission --submit --require-accepted`, mines both
carrier transactions until `get_transaction` reports `committed`, then feeds
both live carrier submissions through Myelin's context, economics, inclusion,
stability, finality, and readiness verifiers. It then submits a tampered
compact-payload carrier under the settlement verifier and requires CKB script
verification to reject it. The smoke also deploys final DA and final settlement
CellScript verifier artefacts, submits final-script transactions, and requires
final settlement type args to be `session_id_hash || settlement_identity_hash`.
The final settlement verifier consumes a one-use authority Cell, checks the final
DA publication as a read-only `CellDep`, rejects same-type inputs, rejects
duplicate same-type group outputs, and rejects any second output in the same
transaction using the same deployed final-settlement code hash/hash type. That
is the current CKB-compatible anti-replay model: transaction-local singleton
creation plus cross-transaction replay protection through the consumed authority
Cell. The settlement package now also emits and verifies local secp256k1
threshold signatures and deterministic threshold-lock args for the
participant-controlled authority-cell creation policy. Final-script submission
and the devnet smoke path require the consumed authority Cell to use those
declared lock args and expose an
`authority_threshold_lock_deployment_checked` readiness marker when the live
lock code dep plus final DA and authority cells all match the declared
threshold-lock args. The package-level authority evidence still keeps
`ckb_enforceable = false`; canonical deployed CKB threshold-lock cryptographic
enforcement, production key custody, and deployment policy remain separate
operational work.
Settlement intents carry a recomputable `court_economics` policy commitment
over participant/escrow binding, locally signature-verified DA committee
availability evidence, challenge timing, minimum dispute bond, slash/reward
basis points, refund/remainder balance, deadline-only settlement, and required
DA evidence. This makes the disputed-close economics locally checkable while
still leaving external DA availability guarantees and deployed CKB
court-economics enforcement explicitly out of scope. Submission readiness
carries an `operational_policy` commitment over confirmation depth, stability,
fee policy, retry identity, live key-submission evidence, monitoring checks, and
optional typed operator custody/runbook documents with machine-visible
requirement lists; it can be testnet-beta ready while keeping
`production_ready = false` and listing missing production blockers until those
operator documents are present and live public-chain evidence is accepted.
The top-level readiness report also keeps `end_to_end_production_ready = false`
until the submission artefact itself proves production DA availability,
canonical threshold-lock enforcement, deployed court-dispute economics, and the
operational policy is production-ready.
The resulting `myelin-ckb-devnet-smoke-v1` report proves devnet CKB acceptance,
deployed compact-payload type-script execution, final-script strict readiness,
live rejection of mismatched carrier data, and live rejection of a competing
final-settlement output probe. For carrier submissions, the inclusion verifier
only marks live inclusion as observed when CKB `outputs_data[0]` matches the
declared carrier payload and `outputs[0].type.args` matches the expected
data-hash-plus-identity layout.

The local CellScript tests
`v0_18_myelin_package_commitment_has_typed_cell_metadata_and_ckb_vm_rejects_tamper`
and
`v0_18_generic_package_commitment_binds_data_hash_to_type_args_in_ckb_vm` keep
the package-commitment boundary covered. The
`v0_18_myelin_da_and_settlement_carriers_bind_compact_payloads_to_type_args_in_ckb_vm`
regression adds typed-cell `DaAnchorCarrier`, `SettlementCarrier`,
`DaAnchorFinal`, and `SettlementFinal` resources and executes their 160-byte
compact-payload verifier shapes as CKB type scripts through `ckb-testtool`.
Matching output data plus identity-bound type args pass; tampered output data,
mismatched identity args, same-type final inputs, duplicate final outputs,
competing final-settlement outputs, wrong final DA evidence, wrong authority
session identity, and self-consistent zero-field payloads fail.

## Teeworlds Pressure Workload

The cloned Teeworlds repository provides a concrete CKB verification shape:

- `ckb/main.cpp` boots the replayer binary.
- `CCkbTapeStream` reads the game tape from witness index `1`.
- the Rust tooling builds or fetches a transaction with four witnesses:
  signature witness, tape, packed map, and game config.
- `ReplayGame` consumes tape events, reconstructs game execution, and emits a
  deterministic state CRC.
- the documented sample verification reports `160,779,457` cycles.

Myelin should support two benchmark modes. Mode A is the first priority because
it keeps the demo inside the `ckb-compatible` semantic profile.

Mode A: CKB-style fixture mode.

```text
teeworlds-cli utils build-test-tx
  --replayer ../ckb/build/replayer_stripped
  --tape dump.bin.2
  --map stripped.map
  --config test_game1.cfg
  --output fixture.json
```

Myelin ingests `fixture.json`, extracts the replayer code dep and witnesses,
projects it into a `CellTx`, verifies it with CKB-strict VM semantics, records
cycles, and wraps the result in a Myelin block.

Mode B: native session mode.

The tape is split into bounded chunks:

```text
ChunkPayload {
  tape_range,
  map_hash,
  config_hash,
  previous_game_state_root,
  expected_game_state_root_or_crc,
}
```

Each chunk is a CellTx with explicit input session Cells, output session Cells,
DA commitments, scheduler witness, and typed conflict key:

```text
conflict_key("teeworlds/session/{session_id}/chunk/{index}")
```

The VM verifies one chunk, emits `MyelinExecutionReport`, updates the session
Cell, and the static committee finalises the block containing the chunk. This is
session-runtime evidence unless the chunk also emits a successful CKB projection
report.

## Benchmark Report

The benchmark command should write:

```json
{
  "workload": "teeworlds",
  "source_repo": "/Users/arthur/RustroverProjects/teeworlds",
  "mode": "ckb-style-fixture",
  "game_duration_seconds": null,
  "player_count": null,
  "tape_size_bytes": null,
  "chunk_size_bytes": null,
  "memory_bytes": null,
  "vm_cycles": null,
  "execution_latency_ms": null,
  "scheduler_overhead_ms": null,
  "block_size_bytes": null,
  "committee_finalisation_latency_ms": null,
  "state_root_before": null,
  "state_root_after": null,
  "semantic_profile": "ckb-compatible",
  "ckb_projection_possible": true,
  "notes": "Populate by running the fixture through the Myelin VM runner."
}
```

No expected success should be hard-coded. The benchmark records measured
results and projection status.

## CLI Shape

The first useful commands are:

```bash
cargo run -p myelin-cli -- celltx simple-report
cargo run -p myelin-cli -- committee finalise-demo --config config/static-committee.toml
cargo run -p myelin-cli -- teeworlds inspect \
  --mock-tx path/to/teeworlds-mock-tx.json \
  --chunk-bytes 262144 \
  --out reports/teeworlds-fixture.json
cargo run -p myelin-cli -- teeworlds bench \
  --mock-tx path/to/teeworlds-mock-tx.json \
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
  --teeworlds-root /Users/arthur/RustroverProjects/teeworlds \
  --out reports/teeworlds-doctor.json
cargo run -p myelin-cli -- teeworlds build-fixture \
  --teeworlds-root /Users/arthur/RustroverProjects/teeworlds \
  --replayer path/to/replayer_stripped \
  --tape path/to/tape.bin \
  --map path/to/stripped.map \
  --config path/to/test_game.cfg \
  --mock-tx-output path/to/teeworlds-mock-tx.json \
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

## Implementation Order

1. Stabilise crate names and public imports under `myelin-*`.
2. Add static committee config, certificate, and block finalisation.
3. Add canonical block hash tests.
4. Add typed-cell admission verification for conflict hash, typed data hash, and
   scheduler witness.
5. Produce `simple CellTx -> execution report`.
6. Produce `simple CellTx -> CKB projection report`.
7. Add Teeworlds fixture ingestion from xxuejie's generated mock transaction.
8. Produce `Teeworlds fixture -> measured benchmark JSON` with per-chunk
   CKB-style projection status.
9. Put the verified chunk into a Myelin block and finalise it with a committee
   certificate.

## Current Status

Completed in this preparation pass:

- Teeworlds repository cloned to the parent project directory.
- Teeworlds Rust tooling adapted locally by pinning `fixed` to `1.30.0`; `cargo
  check` passes in `/Users/arthur/RustroverProjects/teeworlds/rust-tools`.
- Myelin workspace crate names and imports use the `myelin-*` prefix.
- Old L1 mining and consensus vocabulary is absent from active Myelin code.
- `myelin-consensus` provides `ConsensusEngine`, `SelectedConsensus`,
  canonical `MyelinBlock` hashing, working `StaticClosedCommittee` finality,
  and working `Tendermint` precommit finality.
- `myelin-exec::projection` provides `simple CellTx -> CKB projection report`
  using the CKB Molecule transaction layout. `myelin-cli teeworlds inspect`,
  `bench`, and `build-fixture` now attach the same projection status to every
  bounded Teeworlds chunk CellTx.
- `myelin-exec::execution_report` provides deterministic non-contextual
  `simple CellTx -> execution report`.
- `myelin-exec` now uses Molecule as the only public VM object ABI.
  Non-Molecule VM object ABI versions are rejected. `LOAD_TRANSACTION` uses
  Molecule transaction bytes under both Myelin-extended and CKB-strict
  semantics.
- Native `myelin-exec` has no direct or transitive legacy serializer
  dependency, and no legacy serializer API usage in its execution, CellTx,
  typed metadata, scheduler-witness, VM ABI, or serialization code.
  `myelin-hashes`, `myelin-math`, and `myelin-utils` no longer carry that
  legacy serializer for native builds;
  the hash crate's wasm bindings are gated to `wasm32` so workflow wasm support
  does not enter the native execution graph.
- `myelin-exec::serialization::VersionedEnvelope` now emits a
  Molecule-compatible table bytes. The core CellTx
  family (`OutPoint`, `Script`, `CellInput`, `CellOutput`, `CellDep`,
  `DepType`, and `CellTx`) uses CKB Molecule payloads inside that envelope.
  `VersionedSerializable` has no derive-based default codec, so new
  implementors must declare an explicit payload codec.
- `myelin-exec` now serialises `SecureEnvelope` as a Molecule-compatible table
  and routes `serialize_with_integrity` / `deserialize_with_integrity` through
  the versioned Molecule-compatible envelope utilities. `SecureEnvelope`
  deserialisation no longer accepts the old legacy byte layout.
- Typed-cell metadata (`TypedCellDecl`) now has an explicit Molecule-compatible
  metadata codec. Core CKB-style transaction structs and typed-cell metadata now
  use explicit codecs. The old legacy CellScript scheduler-witness decode path
  has been removed; public scheduler-witness admission is Molecule-only.
- `myelin-state` uses explicit Molecule-compatible bytes. CellDB live-cell records,
  spend-journal records, DA segment metadata, DA append indexes, and
  `SegmentProof` public evidence now use explicit Molecule-compatible
  table/vector encodings.
- `myelin-cli` provides executable commands for CellTx reports, static
  committee finality, Teeworlds mock transaction inspection, and measured
  Teeworlds fixture benchmark JSON.
- `myelin-cli teeworlds build-fixture` invokes xxuejie's
  `teeworlds-cli utils build-test-tx`, then benchmarks the generated mock
  transaction inside Myelin.
- `myelin-cli teeworlds doctor` reports whether the cloned Teeworlds repo is
  ready for Rust fixture generation, CKB replayer build, and Myelin VM probing.
- `myelin-cli teeworlds court-bundle` emits the disputed chunk payload bytes,
  chunk payload hash, CKB Molecule transaction bytes, CKB Molecule transaction
  hash, projection report, deterministic challenge payload hash, and
  static-committee evidence needed by the future single-chunk court path.
- `myelin-cli teeworlds verify-court-bundle` verifies that self-contained
  bundle by recomputing the payload hash, Molecule transaction hash, projection
  fields, challenge hash, and static-committee certificate.
- `myelin-cli teeworlds vm-probe` constructs the Teeworlds witness layout and
  runs the replayer binary as a type-script group through Myelin's CKB-VM
  verifier with CKB-strict syscall semantics. The probe models the replayer's
  CKB witness contract with input witness slots `1 = tape`, `2 = map`, and
  `3 = config`.
- The local Teeworlds clone now builds
  `/Users/arthur/RustroverProjects/teeworlds/ckb/build/replayer_stripped` as a
  stripped RISC-V ELF.
- `myelin-cli teeworlds build-fixture` has been run against the real stripped
  replayer, a real generated `dm1.map`, and a deterministic scripted tape
  generated by `teeworlds-cli utils build-scripted-tape`; it invokes xxuejie's
  fixture builder, emits a CKB mock transaction, chunks the tape, emits
  per-chunk CKB-style projection reports, benchmarks ingestion, and finalises a
  static-committee benchmark block.
- `myelin-cli teeworlds court-bundle` has been run against that generated mock
  transaction for chunk `0`; it emits `vm_profile = "ckb-strict-basic"`,
  `ckb_spawn_ipc_required = false`, `court_verifiable = true`,
  `semantic_profile = "ckb-compatible"`, no projection blockers, a CKB Molecule
  transaction hash, embedded chunk payload bytes, embedded Molecule transaction
  bytes, deterministic challenge payload hash, and two static-committee
  signatures. `l1_court_implemented = false` remains explicit.
- `myelin-cli teeworlds verify-court-bundle` has been run against that bundle;
  it reports `valid = true` with 16 passing checks, including VM profile,
  spawn/IPC requirement, payload hash,
  Molecule transaction hash, projection hashes, challenge payload hash,
  signature hashes, signer ids, committee certificate, and quorum weight.
- `myelin-cli teeworlds vm-probe` successfully executes the real stripped
  RISC-V replayer through Myelin's CKB-strict VM path on that generated scripted
  tape. The current evidence exercises client connect/enter, direct input,
  predicted input, tick replay, CKB witness wiring, map/config loading, replay
  loop, and VM syscall path; a live gameplay tape remains the next pressure
  workload.
- `scripts/myelin_teeworlds_acceptance.sh` is the repeatable local acceptance
  gate for this evidence. It regenerates the scripted tape, invokes xxuejie's
  fixture builder, runs Myelin build-fixture, VM probe, court-bundle, and
  court-bundle verification, then asserts the JSON outputs are
  `ckb-compatible`, projection-possible, CKB-strict, court-verifiable, and
  static-committee finalised.
- `scripts/myelin_production_gate.sh` is the top-level local release gate. It
  runs formatting, workspace checks, focused protocol tests, runtime smoke,
  Session L2 open/commit/court/DA/settlement/package checks for both consensus
  modes, dependency and stale-surface scans, and the Teeworlds acceptance gate.
  `scripts/myelin_protocol_gate.sh` remains only as a compatibility wrapper
  that delegates to the production gate.
- A local live-client Teeworlds session was attempted against the cloned fork,
  but the current launch paths did not produce a connected client or
  end-of-match sequencer dump. This keeps the present evidence boundary honest:
  Myelin runs xxuejie's CKB replayer shape today, while a dedicated gameplay
  tape harness remains the next integration step.
- Targeted checks pass for:
  - `cargo fmt --all --check`
  - `cargo check --workspace`
  - `cargo test -p myelin-hashes -p myelin-math -p myelin-exec -p myelin-consensus -p myelin-state -p myelin-mempool -p myelin-cli`
  - dependency-tree inversion for the removed legacy serializer package reports
    that no such package is present.
  - legacy vocabulary scan for inherited L1 terms, old branding, and legacy
    serializer markers returns no active matches outside ignored artefacts.
  - `cargo run --bin teeworlds-cli -- utils build-scripted-tape --ticks 300 --clients 1 --input-every 5 --seed 1 --output /tmp/myelin-teeworlds-scripted-tape.bin`
  - `cargo run -p myelin-cli -- teeworlds build-fixture --teeworlds-root /Users/arthur/RustroverProjects/teeworlds --replayer /Users/arthur/RustroverProjects/teeworlds/ckb/build/replayer_stripped --tape /tmp/myelin-teeworlds-scripted-tape.bin --map /Users/arthur/RustroverProjects/teeworlds/build/data/maps/dm1.map --config /Users/arthur/RustroverProjects/teeworlds/build/myelin_replay_40265.cfg --mock-tx-output /tmp/myelin-teeworlds-scripted-mock-tx.json --runs 3 --out /tmp/myelin-teeworlds-scripted-build-fixture.json`
  - `cargo run -p myelin-cli -- teeworlds court-bundle --mock-tx /tmp/myelin-teeworlds-scripted-mock-tx.json --chunk-bytes 262144 --chunk-index 0 --out /tmp/myelin-teeworlds-scripted-court-bundle.json`
  - `cargo run -p myelin-cli -- teeworlds verify-court-bundle --bundle /tmp/myelin-teeworlds-scripted-court-bundle.json --out /tmp/myelin-teeworlds-scripted-court-bundle-verify.json`
  - `cargo run -p myelin-cli -- teeworlds vm-probe --replayer /Users/arthur/RustroverProjects/teeworlds/ckb/build/replayer_stripped --tape /tmp/myelin-teeworlds-scripted-tape.bin --map /Users/arthur/RustroverProjects/teeworlds/build/data/maps/dm1.map --config /Users/arthur/RustroverProjects/teeworlds/build/myelin_replay_40265.cfg --out /tmp/myelin-teeworlds-scripted-vm-probe.json`
  - `scripts/myelin_teeworlds_acceptance.sh`
  - `scripts/myelin_production_gate.sh`

Next stronger evidence:

- Run the same Myelin VM path against a live gameplay tape generated by an
  actual Teeworlds session with network clients. The scripted tape proves the
  repository's player/input replay shape, not the full GUI/network session
  harness.
