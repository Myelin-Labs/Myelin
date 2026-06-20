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
session fixture -> open/commit/court/verify with both consensus modes
Teeworlds fixture -> measured benchmark JSON with per-chunk CKB projection status
```

## Immediate Evidence Targets

The preferred full local release gate is:

```bash
scripts/myelin_production_gate.sh
```

It checks the active Myelin tree for removed source-chain and legacy serializer
vocabulary, proves the native dependency graph has no removed serializer
package, runs the focused Rust workspace checks, exercises runtime smoke,
Session L2 open/commit/court/DA/settlement/package flows for both consensus
engines, and then runs the Teeworlds acceptance gate. The older
`scripts/myelin_protocol_gate.sh` name is kept only as a compatibility wrapper
that delegates to the production gate.

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
cargo run -p myelin-cli -- session open \
  --app-id myelin-custom-game-session-v1 \
  --participant alice \
  --participant bob \
  --escrow-cell '<tx_hash_hex>:0:1000:<lock_hash_hex>' \
  --consensus static-closed-committee \
  --out reports/session-open.json
cargo run -p myelin-cli -- session open-fixture \
  --consensus static-closed-committee \
  --out reports/session-open.json
cargo run -p myelin-cli -- session commit-fixture \
  --session reports/session-open.json \
  --out reports/session-commit.json
cargo run -p myelin-cli -- session commit \
  --session reports/session-open.json \
  --chunk-index 7 \
  --out reports/session-commit.json
cargo run -p myelin-cli -- session court-bundle \
  --commit reports/session-commit.json \
  --chunk-index 0 \
  --out reports/session-court-bundle.json
cargo run -p myelin-cli -- session verify-court-bundle \
  --bundle reports/session-court-bundle.json \
  --out reports/session-court-verify.json
cargo run -p myelin-cli -- session da-manifest \
  --bundle reports/session-court-bundle.json \
  --storage-dir reports/session-da-store \
  --out reports/session-da-manifest.json
cargo run -p myelin-cli -- session verify-da-manifest \
  --manifest reports/session-da-manifest.json \
  --bundle reports/session-court-bundle.json \
  --storage-dir reports/session-da-store \
  --out reports/session-da-verify.json
cargo run -p myelin-cli -- session da-anchor-package \
  --manifest reports/session-da-manifest.json \
  --bundle reports/session-court-bundle.json \
  --out reports/session-da-anchor-package.json
cargo run -p myelin-cli -- session verify-da-anchor-package \
  --package reports/session-da-anchor-package.json \
  --manifest reports/session-da-manifest.json \
  --bundle reports/session-court-bundle.json \
  --out reports/session-da-anchor-verify.json
cargo run -p myelin-cli -- session submit-da-anchor-package \
  --package reports/session-da-anchor-package.json \
  --dry-run \
  --out reports/session-da-anchor-submit.json
cargo run -p myelin-cli -- session verify-submission-context \
  --submission reports/session-da-anchor-submit.json \
  --rpc-url http://127.0.0.1:8114 \
  --out reports/session-da-anchor-context.json
cargo run -p myelin-cli -- session verify-submission-economics \
  --submission reports/session-da-anchor-submit.json \
  --rpc-url http://127.0.0.1:8114 \
  --min-fee-shannons 1 \
  --min-fee-rate-shannons-per-kb 1000 \
  --max-fee-shannons 1000 \
  --out reports/session-da-anchor-economics.json
cargo run -p myelin-cli -- session verify-submission-inclusion \
  --submission reports/session-da-anchor-submit.json \
  --rpc-url http://127.0.0.1:8114 \
  --min-status committed \
  --out reports/session-da-anchor-inclusion.json
cargo run -p myelin-cli -- session verify-submission-stability \
  --inclusion reports/session-da-anchor-inclusion.json \
  --rpc-url http://127.0.0.1:8114 \
  --out reports/session-da-anchor-stability.json
cargo run -p myelin-cli -- session verify-submission-finality \
  --inclusion reports/session-da-anchor-inclusion.json \
  --rpc-url http://127.0.0.1:8114 \
  --min-confirmations 6 \
  --out reports/session-da-anchor-finality.json
cargo run -p myelin-cli -- session verify-submission-readiness \
  --context reports/session-da-anchor-context.json \
  --economics reports/session-da-anchor-economics.json \
  --inclusion reports/session-da-anchor-inclusion.json \
  --stability reports/session-da-anchor-stability.json \
  --finality reports/session-da-anchor-finality.json \
  --out reports/session-da-anchor-readiness.json
cargo run -p myelin-cli -- session settlement-intent \
  --bundle reports/session-court-bundle.json \
  --da-manifest reports/session-da-manifest.json \
  --kind disputed-close \
  --current-time-ms 60000 \
  --challenge-window-ms 60000 \
  --out reports/session-settlement-intent.json
cargo run -p myelin-cli -- session verify-settlement-intent \
  --intent reports/session-settlement-intent.json \
  --bundle reports/session-court-bundle.json \
  --da-manifest reports/session-da-manifest.json \
  --out reports/session-settlement-verify.json
cargo run -p myelin-cli -- session settlement-package \
  --intent reports/session-settlement-intent.json \
  --bundle reports/session-court-bundle.json \
  --da-manifest reports/session-da-manifest.json \
  --out reports/session-settlement-package.json
cargo run -p myelin-cli -- session verify-settlement-package \
  --package reports/session-settlement-package.json \
  --intent reports/session-settlement-intent.json \
  --bundle reports/session-court-bundle.json \
  --da-manifest reports/session-da-manifest.json \
  --out reports/session-settlement-package-verify.json
cargo run -p myelin-cli -- session submit-settlement-package \
  --package reports/session-settlement-package.json \
  --dry-run \
  --out reports/session-settlement-submit.json
cargo run -p myelin-cli -- session verify-submission-context \
  --submission reports/session-settlement-submit.json \
  --rpc-url http://127.0.0.1:8114 \
  --out reports/session-settlement-context.json
cargo run -p myelin-cli -- session verify-submission-economics \
  --submission reports/session-settlement-submit.json \
  --rpc-url http://127.0.0.1:8114 \
  --min-fee-shannons 1 \
  --min-fee-rate-shannons-per-kb 1000 \
  --max-fee-shannons 1000 \
  --out reports/session-settlement-economics.json
cargo run -p myelin-cli -- session verify-submission-inclusion \
  --submission reports/session-settlement-submit.json \
  --rpc-url http://127.0.0.1:8114 \
  --min-status committed \
  --out reports/session-settlement-inclusion.json
cargo run -p myelin-cli -- session verify-submission-stability \
  --inclusion reports/session-settlement-inclusion.json \
  --rpc-url http://127.0.0.1:8114 \
  --out reports/session-settlement-stability.json
cargo run -p myelin-cli -- session verify-submission-finality \
  --inclusion reports/session-settlement-inclusion.json \
  --rpc-url http://127.0.0.1:8114 \
  --min-confirmations 6 \
  --out reports/session-settlement-finality.json
cargo run -p myelin-cli -- session verify-submission-readiness \
  --context reports/session-settlement-context.json \
  --economics reports/session-settlement-economics.json \
  --inclusion reports/session-settlement-inclusion.json \
  --stability reports/session-settlement-stability.json \
  --finality reports/session-settlement-finality.json \
  --out reports/session-settlement-readiness.json
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

For production operations evidence, `session verify-submission-readiness` also
accepts `--operator-custody-policy reports/operator-custody-policy.json` and
`--operator-runbook reports/operator-runbook.json`. Those files are schema
checked, typed-control checked, and hashed into `operational_policy`; without
them the report keeps `operational_policy.production_ready = false`. The
custody policy must declare hardware-backed keys, dual-control signing,
tested rotation and emergency-drill controls, plus a non-zero signing threshold
within the operator set. The runbook must bind to the readiness report's
confirmation depth and fee policy, require stability requery, and declare
bounded retry and monitoring cadence.

For external DA evidence, `session da-manifest` also accepts
`--external-da-receipt reports/external-da-receipt.json`. The receipt must use
`myelin-external-da-receipt-v2`, bind to the manifest payload hash and segment
root, and carry a provider recoverable secp256k1 signature over the receipt
fields. With sealed local DA storage and a matching provider-signed receipt, the
DA availability evidence can be `testnet_beta_ready`. It only sets
`production_ready = true` when the same provider signature also covers explicit
production SLA fields: `service_level = "production"`, at least 30 days of
`retention_seconds`, an HTTPS `retrieval_endpoint`, and a 32-byte
`audit_log_commitment`. The default local gate still omits this production
receipt and does not set `l1_da_published`.

The Teeworlds command consumes the CKB mock transaction produced by xxuejie's
`teeworlds-cli utils build-test-tx`, splits the tape witness into bounded
chunks, emits CKB-style projection status for every chunk CellTx, commits those
chunks, measures fixture ingestion, and finalises a benchmark block with the
phase-one static closed committee.
The `session` commands are the first executable Session L2 spine. They open an
escrow-like finite Cell session from either a CLI-supplied descriptor or a
deterministic fixture, commit a chunk through
`CellPool`, `CellStateTree`, and the selected consensus engine, materialise a
self-contained court bundle, and verify the bundle by recomputing its Molecule
transaction hash, CKB-compatible projection, canonical block hash, challenge
hash, and finality evidence. Both static-closed-committee and Tendermint
profiles are supported; the state transition is consensus-independent while
the finality evidence remains domain-separated. `session da-manifest` emits a
Merkle `SegmentProof` for the exact Molecule transaction bytes needed by court
replay, and `session verify-da-manifest` binds that proof back to the court
bundle. With `--storage-dir`, the payload is durably written to a sealed local
DA segment and the proof is rebuilt from `SegmentReader`; with
`--external-da-receipt`, a provider-neutral receipt is schema-checked,
provider-signature checked, and hash-bound into the DA availability commitment.
`session da-anchor-package` converts that
verified manifest into a deterministic CKB-compatible DA anchor CellTx package,
and `session verify-da-anchor-package` recomputes the manifest hash, embedded
Molecule transaction, CellTx ids, and projection. `session submit-da-anchor-package`
builds the CKB `send_transaction` JSON-RPC request and can submit it to a
configured HTTP CKB RPC endpoint; the production gate uses `--dry-run`, so it
proves request construction while leaving `l1_da_published = false`; even a
direct RPC-accepted projected package report records node acceptance separately
and does not claim final L1 DA publication. `session verify-submission-context` queries CKB
`get_live_cell` for every input and cell dep, so fake or already-spent cells are
caught before live submission. `session verify-submission-economics` queries the
live input cells, sums input and output capacity, and marks
`economically_ready = true` only when outputs are funded and the configured fee
floor, fee-rate policy, and optional max-fee policy are met. `session verify-submission-inclusion` queries CKB
`get_transaction` for the submission hash and reports `live_l1_observed = true`
only when the transaction is committed with a block hash.
`session verify-submission-stability` re-queries `get_transaction` and marks
`stable_block_identity = true` only when CKB still reports the same committed
block hash and block number. `session verify-submission-finality` then queries
CKB `get_tip_header`, combines the tip height with the inclusion report's
committed block number, and marks `finality_confirmed = true` only when the
configured confirmation depth is met.
`session verify-submission-readiness` aggregates context, economics, inclusion,
stability, and finality reports into a `production_submission_ready` coherence
decision and refuses readiness if the reports do not bind to the same CKB
transaction or committed block identity. The report also exposes
`strict_production_submission_ready`, `readiness_evidence_mode`,
`end_to_end_production_ready`, `end_to_end_production_blockers`,
`live_carrier_submission_ready`, and `final_l1_script_submission_ready`, so
offline/mock verifier evidence, live carrier evidence, final compact L1 script
evidence, and full production readiness cannot be confused. Final L1 script
readiness is true only when the referenced submission report also proves the
final-script pre-submit checks: live funding/code cells, matching verifier code
hash, and, for final settlement, the final DA evidence CellDep, the
package-declared authority input with matching data hash and settlement lock,
matching threshold-lock args, and explicit settlement uniqueness evidence.
Strict end-to-end production readiness additionally requires no named blockers:
real DA availability guarantees, canonical deployed threshold-lock enforcement,
deployed CKB court-dispute economics, and operational custody/runbook evidence
must all be present. The report also carries
`authority_threshold_lock_deployment_checked` /
`authority_threshold_lock_deployment_mode` for final settlement submissions,
proving the live lock code-dep plus final DA and authority lock preflight used
the declared threshold-lock args. It still leaves package-level
`authority_authentication.ckb_enforceable = false` by default. Production
authority evidence is now an explicit opt-in on `session settlement-package` via
`--threshold-lock-deployment-evidence`; the evidence must bind a checked CKB
deployment, code dep, audited source/report hashes, signer set, threshold, and
the generated threshold-lock args hash into the authority attestation before
`ckb_enforceable` / `production_ready` can become true.
The readiness report carries
`operational_policy`, a public-chain operations commitment covering reorg
confirmation depth, stability requery, fee floor/rate/max-fee policy, retry
identity, key-submission evidence, monitoring evidence, and optional hashed
operator custody/runbook policy files with machine-visible requirement lists.
It can be `testnet_beta_ready` with live public-chain evidence while still
leaving `production_ready = false` and listing
`operator-custody-policy-missing` / `operator-runbook-missing` until those typed
artefacts are supplied.
`session settlement-intent` then turns a verified court bundle plus verified DA
manifest into an explicit disputed-close settlement object with challenge-window
binding and `l1_da_published = false` / `l1_court_implemented = false` markers.
It includes `court_economics`, a deterministic disputed-close policy with an
explicit minimum bond, loser-slash basis points, challenger reward, honest-party
refund, zero unresolved remainder, DA-evidence requirement, and
settlement-after-deadline invariant. `session verify-settlement-intent`
recomputes the same three-way binding and economics commitment. The policy is
locally checkable by default and can opt into production court evidence via
`--court-economics-deployment-evidence`; that file must bind the deployed CKB
court verifier, audited source/report hashes, economics commitment, challenge
payload, DA availability commitment, minimum bond, loser-slash basis points, and
deadline/DA requirements before `court_economics.production_ready` can become
true.
`session settlement-package` then emits a deterministic CKB-compatible
settlement CellTx package that binds the exact intent JSON bytes, court bundle,
DA manifest, challenge payload, and final state root. It also declares
`settlement_authority`: a one-use authority Cell requirement whose 192-byte
data payload is `intent_hash || session_id || participant_set_hash ||
escrow_input_cells_hash || session_lineage_commitment ||
session_authority_commitment`, whose `data_hash` is the expected CKB data hash,
whose lineage fields are copied from the verified session path, whose authority
commitment binds those lineage fields together with the intent hash and
`session_id`, whose `authority_authentication` carries locally verified
secp256k1 threshold-signature evidence over the authority data hash and session
lineage plus deterministic CKB lock args for the participant threshold set,
whose consumed input index is `1`, and whose lock code and threshold-lock args
must match the final DA publication lock. This is not yet deployed CKB
threshold-lock cryptographic enforcement.
Final settlement type args are exactly `session_id_hash ||
settlement_identity_hash`, where `session_id_hash` is copied from the consumed
authority Cell and `settlement_identity_hash` is the CKB data hash of the
160-byte final settlement payload. The final settlement CellScript rejects
same-type inputs, duplicate same-type group outputs, and any second output in
the transaction using the same deployed final-settlement code hash/hash type.
That gives transaction-local singleton creation; cross-transaction replay is
blocked by consuming the one-use authority Cell. The package now emits and
verifies local threshold signatures plus deterministic threshold-lock args for
authority-cell creation; final-script submission rejects a mismatched declared
authority lock identity before broadcast and exposes a live threshold-lock
deployment preflight marker when the lock code dep plus final DA and authority
cells all match the declared threshold-lock args, while canonical deployed CKB
threshold-lock cryptographic enforcement, production key custody, and deployment
policy remain outside this milestone.
`session verify-settlement-package` recomputes the embedded Molecule
transaction, CKB projection, and settlement-authority requirement.
`session submit-settlement-package` builds the CKB
`send_transaction` JSON-RPC request and can submit it to a configured HTTP CKB
RPC endpoint; `session verify-submission-context` checks live inputs/deps and
`session verify-submission-economics` checks capacity balance and fee policy
before `session verify-submission-inclusion` checks whether CKB reports the
transaction as committed. `session verify-submission-finality` checks that the
committed transaction is deep enough under the configured confirmation policy,
and `session verify-submission-stability` catches moved or disappeared committed
transactions before finality is trusted. `session verify-submission-readiness`
then emits the single operator-facing readiness decision. The gate dry-runs
submission and uses mock CKB context, economics, inclusion, stability, and
finality servers for verifier coverage, so the package is still not a deployed
court script by itself. For a live local CKB check, run:

```bash
scripts/myelin_ckb_devnet_smoke.sh
```

That optional smoke starts the parent `../ckb` devnet, mines an
always-success funding cell, deploys separate DA-anchor and settlement
CellScript carrier verifiers plus separate final-script verifier artefacts as
`data2` type scripts, writes 160-byte DA-anchor and settlement payloads into
verifier-guarded CKB output data, and binds the type args to
`ckb_data_hash(payload) || identity_hash`, where the identity hash is the DA
manifest hash or settlement intent hash. The settlement carrier is funded from
the DA carrier's change output; the final DA submission is funded from the
settlement carrier's change output; and the final settlement submission is
funded from the final DA change output. Valid submissions go through
`myelin session carrier-submission --submit --require-accepted`, using the
default carrier role for carrier evidence and `--verifier-role final-l1-script`
for final-script evidence. Final settlement submission must provide the
package-declared authority Cell as input `1` and the final DA publication Cell
as an evidence `CellDep`; live preflight checks that both cells are live, that
the authority input has the declared capacity and package-declared CKB data
hash, and that both the authority input and final DA publication CellDep use
the declared settlement output lock before broadcasting. The smoke mines each
transaction until CKB reports it `committed`, runs the same
context/economics/inclusion/stability/finality and aggregate readiness
verifiers against live CKB RPC evidence for both roles, then submits
tampered compact-payload carriers under both deployed verifiers and requires CKB
script verification to reject them. For carrier submissions, live inclusion requires CKB
`outputs_data[0]` to match the declared carrier payload and
`outputs[0].type.args` to match the expected data-hash-plus-identity layout. The
same smoke also compiles and deploys `da-anchor-final.cell` and
`settlement-final.cell` as final-script verifier artefact code cells, recording
their typed-cell metadata, CKB ELFs, code hashes, and code deps.
`myelin-ckb-devnet-smoke-v1` report exposes `all_live_checks_passed` only when
both valid carriers report `readiness_evidence_mode = "live-ckb-carrier"`, both
final-script submissions report `readiness_evidence_mode = "final-l1-script"`,
both final-script submissions keep `end_to_end_production_ready = false` with
named production blockers,
the final settlement report carries positive final DA/authority pre-submit
checks plus settlement uniqueness evidence, the competing final settlement
replay probe is rejected, and both tampered carriers are rejected. It is
deliberately not part of the default production gate because it is slower and
requires a local parent CKB checkout.

CellScript now has focused regressions and live-devnet deployment evidence for
the compact carrier and final-script steps:
`v0_18_myelin_package_commitment_has_typed_cell_metadata_and_ckb_vm_rejects_tamper`
compiles a `PackageCommitment` resource under the local `typed-cell` profile,
then runs the same package-commitment CellScript logic as a CKB type script in
`ckb-testtool`. The matching 32-byte package commitment passes and a tampered
commitment fails. `v0_18_generic_package_commitment_binds_data_hash_to_type_args_in_ckb_vm`
proves the reusable verifier shape: output data holds the 32-byte package
commitment, while type args hold the CKB data hash of that commitment.
`v0_18_myelin_da_and_settlement_carriers_bind_compact_payloads_to_type_args_in_ckb_vm`
adds typed-cell `DaAnchorCarrier`, `SettlementCarrier`, `DaAnchorFinal`, and
`SettlementFinal` payload checks for the 160-byte compact shape, including type
args that bind both the full payload data hash and the resource identity field.
The final-script variants are creation-only: the CKB VM regression rejects any
same-type group input, so a final proof cell cannot be updated in place under
the same type script. Final settlement also requires the final DA publication
cell as a read-only `CellDep`, checks that its data hash and `data2` type args
bind the same DA manifest hash carried by the settlement payload, requires a
consumed settlement-authority input whose first 32-byte data field equals the
settlement intent hash and whose remaining fields carry the session id,
participant digest, escrow digest, session-lineage commitment, and authority
commitment. It requires the settlement output type args to be `session_id_hash ||
settlement_identity_hash`, requires both the authority input and settlement
output lock hashes to match the DA publication lock hash, rejects competing
final-settlement outputs under the same deployed final-settlement verifier in
the same transaction, and leaves the DA publication cell unspent. The live
parent-CKB devnet smoke deploys the DA-anchor and settlement
carrier verifiers plus the final-script verifier artefacts as separate `data2`
CKB type scripts, creates the one-use settlement-authority Cell from the
settlement package's declared `settlement_authority`, records the authority
session id, participant digest, escrow digest, session-lineage commitment, and
authority commitment in the smoke report, and consumes it in the final
settlement transaction using the package-declared threshold-lock args. It also
submits a competing final-settlement output probe before the valid final
settlement and requires live CKB script verification to reject it. This proves
deployed compact-payload script semantics for the local devnet carrier and
final-script paths, plus locally verified DA committee signatures,
authority-authentication signatures, and threshold-lock args binding. It still
does not claim production DA availability unless a signed production SLA receipt
is supplied, and it still does not claim deployed threshold-lock cryptographic
authority enforcement, production key management, or deployed CKB court-dispute
economics.
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
