# Myelin Session L2 Plan

## 1. Positioning

Myelin should be tightened around one product identity:

> Myelin is a CKB-isomorphic finite Cell session L2.

It is not, for the next milestone, an independent app-chain or sidechain. The
system may keep chain-shaped internals, but its public direction is a bounded
off-chain session ledger whose disputed chunks can be projected into a
CKB-compatible court path.

The immediate goal is to turn the current prototype into a release-grade
session kernel:

```text
session open
  -> off-chain chunk execution
  -> chunk/state commitment
  -> disputed court bundle
  -> CKB-strict replay/projection
  -> settlement or exit
```

## 2. Current Baseline

The following pieces already exist and should be treated as foundations, not
future work:

- `myelin-cli runtime smoke` wires `myelin-mempool`, `myelin-state`,
  `myelin-muhash`, `myelin-math`, and both consensus engines through the CLI.
- `myelin-core-utils` is already split from broader `myelin-utils`, and hot-path
  crates now depend on the smaller surface.
- Teeworlds fixture support already covers inspect, benchmark, VM probe,
  court-bundle emission, and court-bundle verification.
- The production gate already runs runtime smoke, dependency-tree checks, and
  Teeworlds acceptance.
- `CkbStrict` exists and is used by the Teeworlds VM probe.

The missing piece is no longer "connect the runtime spine". The missing piece is
to make the connected spine express a real session protocol.

## 3. Scope Decision

### In Scope

- A finite session ledger where CKB Cells escrow assets and Myelin runs the fast
  off-chain path.
- Deterministic chunk commitments, state roots, tape/data commitments, and
  finality evidence.
- CKB-compatible disputed-chunk projection and verification.
- A production profile that defaults to `CkbStrict` for court-facing execution.
- Explicit opt-in for Myelin-only extensions.

### Out of Scope

- Independent block production for an app-chain.
- P2P gossip, fork choice, validator-set lifecycle, genesis management, RPC
  indexing, slashing, fee markets, or app-chain governance.
- Treating Teeworlds fixture evidence as proof of a production game session.

Independent-chain work can be reconsidered only after the Session L2 path has a
working open/commit/challenge/settle flow.

## 4. P0: Session Protocol Skeleton

Add a first-class session model around the existing runtime primitives.

Required artefacts:

- `SessionId`: domain-separated hash over participants, asset cells, game/app
  id, VM profile, and initial state root.
- `SessionOpen`: declarative session start record with participant set,
  escrowed input Cells, timeout parameters, and selected consensus profile.
- `ChunkCommitment`: ordered chunk index, previous state root, new state root,
  tape/data commitment, scheduler commitment, and ordered CellTx commitments.
- `DisputeBundle`: one challenged chunk plus all data needed to recompute the
  claimed state transition.
- `SettlementIntent`: normal close, disputed close, timeout exit, or abort.

Implementation direction:

- Keep the first version in the Myelin CLI/runtime layer, not as a separate
  daemon.
- Reuse `CellPool` for admission and `CellStateTree` for root transitions.
- Reuse `MyelinBlock` as the finality evidence container, but rename/report it
  in session terms when exposed through CLI JSON.
- Make every commitment domain-separated and stable.

Acceptance criteria:

- A deterministic built-in session fixture opens a session, commits one chunk,
  finalises it with static committee, and emits a JSON report.
- The same fixture finalises with Tendermint and produces identical state
  transition commitments but different finality evidence.
- The report includes `session_id`, `chunk_index`, `state_root_before`,
  `state_root_after`, `ordered_cell_tx_commitments`, `data_commitments`,
  `scheduler_commitment`, `consensus_kind`, and `vm_profile`.

## 5. P1: Court-Facing CKB Strict Profile

The production court path must be narrower than the general Myelin VM path.

Rules:

- Court-facing execution defaults to `VmSemantics::CkbStrict`.
- `VmSemantics::MyelinExtended` remains available only as an explicit profile.
- Reports must always state the VM profile used.
- A report with `MyelinExtended` must not claim CKB court compatibility unless
  the projection layer explicitly proves compatibility.
- Legacy group-source encoding must be treated as a deviation, not an implicit
  production default.

Required cleanup:

- Remove or feature-gate legacy decode fallbacks in public execution-facing
  paths.
- Stop exporting non-essential serialization helper frameworks from the
  `myelin-exec` crate root unless they are part of the session protocol.
- Keep spawn/pipe/read/write/wait/inherited-fd support only if the selected
  court profile needs CKB VM v2 spawn/IPC semantics; otherwise gate them behind
  an explicit feature/profile.

Acceptance criteria:

- `teeworlds vm-probe` and the new session court command both report an
  explicit CKB-strict profile such as `vm_profile = "ckb-strict-basic"` or,
  when spawn/IPC is deliberately enabled, `vm_profile = "ckb-strict-spawn-ipc"`.
- Production gate fails if a court-compatible report is produced without an
  explicit CKB-strict or proven-compatible profile.
- Semantic deviation documentation remains accurate and machine-checkable.

## 6. P2: Session CLI

CLI commands now expose the first deterministic Session L2 path directly.

Implemented commands:

```bash
myelin session open --app-id myelin-custom-game-session-v1 --participant alice --participant bob --escrow-cell '<tx_hash_hex>:0:1000:<lock_hash_hex>' --out reports/session-open.json
myelin session open-fixture --out reports/session-open.json
myelin session commit --session reports/session-open.json --chunk-index 7 --out reports/session-commit.json
myelin session commit-fixture --session reports/session-open.json --out reports/session-commit.json
myelin session court-bundle --commit reports/session-commit.json --chunk-index 0 --out reports/session-court-bundle.json
myelin session verify-court-bundle --bundle reports/session-court-bundle.json --out reports/session-court-verify.json
myelin session da-manifest --bundle reports/session-court-bundle.json --storage-dir reports/session-da-store --out reports/session-da-manifest.json
myelin session verify-da-manifest --manifest reports/session-da-manifest.json --bundle reports/session-court-bundle.json --storage-dir reports/session-da-store --out reports/session-da-verify.json
myelin session da-anchor-package --manifest reports/session-da-manifest.json --bundle reports/session-court-bundle.json --out reports/session-da-anchor-package.json
myelin session verify-da-anchor-package --package reports/session-da-anchor-package.json --manifest reports/session-da-manifest.json --bundle reports/session-court-bundle.json --out reports/session-da-anchor-verify.json
myelin session submit-da-anchor-package --package reports/session-da-anchor-package.json --dry-run --out reports/session-da-anchor-submit.json
myelin session carrier-submission --package reports/session-da-anchor-package.json --input-tx-hash <live_tx_hash> --input-index 0 --input-capacity-shannons 100000000000 --carrier-capacity-shannons 40000000000 --fee-shannons 2000 --lock-code-hash <lock_code_hash> --lock-code-dep-tx-hash <lock_dep_tx_hash> --lock-code-dep-index 0 --verifier-code-hash <carrier_verifier_code_hash> --verifier-code-dep-tx-hash <verifier_dep_tx_hash> --verifier-code-dep-index 1 --verifier-source da-anchor-carrier.cell --witness <entry_witness_hex> --rpc-url http://127.0.0.1:8114 --submit --require-accepted --out reports/session-da-anchor-carrier-submit.json
myelin session verify-submission-context --submission reports/session-da-anchor-submit.json --rpc-url http://127.0.0.1:8114 --out reports/session-da-anchor-context.json
myelin session verify-submission-economics --submission reports/session-da-anchor-submit.json --rpc-url http://127.0.0.1:8114 --min-fee-shannons 1 --min-fee-rate-shannons-per-kb 1000 --max-fee-shannons 1000 --out reports/session-da-anchor-economics.json
myelin session verify-submission-inclusion --submission reports/session-da-anchor-submit.json --rpc-url http://127.0.0.1:8114 --min-status committed --out reports/session-da-anchor-inclusion.json
myelin session verify-submission-stability --inclusion reports/session-da-anchor-inclusion.json --rpc-url http://127.0.0.1:8114 --out reports/session-da-anchor-stability.json
myelin session verify-submission-finality --inclusion reports/session-da-anchor-inclusion.json --rpc-url http://127.0.0.1:8114 --min-confirmations 6 --out reports/session-da-anchor-finality.json
myelin session verify-submission-readiness --context reports/session-da-anchor-context.json --economics reports/session-da-anchor-economics.json --inclusion reports/session-da-anchor-inclusion.json --stability reports/session-da-anchor-stability.json --finality reports/session-da-anchor-finality.json --out reports/session-da-anchor-readiness.json
myelin session settlement-intent --bundle reports/session-court-bundle.json --da-manifest reports/session-da-manifest.json --kind disputed-close --current-time-ms 60000 --challenge-window-ms 60000 --out reports/session-settlement-intent.json
myelin session verify-settlement-intent --intent reports/session-settlement-intent.json --bundle reports/session-court-bundle.json --da-manifest reports/session-da-manifest.json --out reports/session-settlement-verify.json
myelin session settlement-package --intent reports/session-settlement-intent.json --bundle reports/session-court-bundle.json --da-manifest reports/session-da-manifest.json --out reports/session-settlement-package.json
myelin session verify-settlement-package --package reports/session-settlement-package.json --intent reports/session-settlement-intent.json --bundle reports/session-court-bundle.json --da-manifest reports/session-da-manifest.json --out reports/session-settlement-package-verify.json
myelin session submit-settlement-package --package reports/session-settlement-package.json --dry-run --out reports/session-settlement-submit.json
myelin session carrier-submission --package reports/session-settlement-package.json --input-tx-hash <live_tx_hash> --input-index 1 --input-capacity-shannons 60000000000 --carrier-capacity-shannons 40000000000 --fee-shannons 2000 --lock-code-hash <lock_code_hash> --lock-code-dep-tx-hash <lock_dep_tx_hash> --lock-code-dep-index 0 --verifier-code-hash <carrier_verifier_code_hash> --verifier-code-dep-tx-hash <verifier_dep_tx_hash> --verifier-code-dep-index 2 --verifier-source settlement-carrier.cell --witness <entry_witness_hex> --rpc-url http://127.0.0.1:8114 --submit --require-accepted --out reports/session-settlement-carrier-submit.json
myelin session verify-submission-context --submission reports/session-settlement-submit.json --rpc-url http://127.0.0.1:8114 --out reports/session-settlement-context.json
myelin session verify-submission-economics --submission reports/session-settlement-submit.json --rpc-url http://127.0.0.1:8114 --min-fee-shannons 1 --min-fee-rate-shannons-per-kb 1000 --max-fee-shannons 1000 --out reports/session-settlement-economics.json
myelin session verify-submission-inclusion --submission reports/session-settlement-submit.json --rpc-url http://127.0.0.1:8114 --min-status committed --out reports/session-settlement-inclusion.json
myelin session verify-submission-stability --inclusion reports/session-settlement-inclusion.json --rpc-url http://127.0.0.1:8114 --out reports/session-settlement-stability.json
myelin session verify-submission-finality --inclusion reports/session-settlement-inclusion.json --rpc-url http://127.0.0.1:8114 --min-confirmations 6 --out reports/session-settlement-finality.json
myelin session verify-submission-readiness --context reports/session-settlement-context.json --economics reports/session-settlement-economics.json --inclusion reports/session-settlement-inclusion.json --stability reports/session-settlement-stability.json --finality reports/session-settlement-finality.json --out reports/session-settlement-readiness.json
```

Behaviour:

- `open` creates a session from CLI-supplied participants and escrow-like input
  Cells; `open-fixture` keeps the deterministic CI fixture.
- `commit` admits one deterministic chunk for a supplied session and chunk
  index; `commit-fixture` keeps the zero-index fixture path. Both paths admit
  a CellTx through `CellPool`, apply it to `CellStateTree`, finalise a session
  block, and emit chunk commitments.
- `court-bundle` materialises the disputed chunk into a self-contained replay
  bundle.
- `verify-court-bundle` recomputes all commitments, verifies finality evidence,
  and confirms CKB-compatible projection.
- `da-manifest` emits a Merkle `SegmentProof` over the exact Molecule
  transaction bytes needed by court replay. With `--storage-dir`, it writes the
  payload through `SegmentWriter`, seals the segment, and rebuilds the proof
  through `SegmentReader`.
- `verify-da-manifest` verifies that proof, the sealed local segment, and the
  exact court bundle binding, while keeping external L1 DA publication
  explicitly unimplemented.
- `da-anchor-package` turns a verified DA manifest into a deterministic
  CKB-compatible DA anchor CellTx package. The package binds the exact manifest
  JSON bytes, court-bundle hash, court replay transaction hash, challenge
  payload hash, DA segment root, and proof commitment.
- `verify-da-anchor-package` recomputes the manifest hash, embedded Molecule
  transaction hash, CellTx id, witness txid, deterministic DA anchor CellTx,
  and CKB-compatible projection. It keeps
  `l1_da_publication_implemented = false`; this is a package and verifier, not
  yet a submitted L1 DA transaction.
- `submit-da-anchor-package` validates the package, converts the embedded
  CellTx into CKB `send_transaction` JSON, and either dry-runs the request or
  submits it to a configured plain-HTTP CKB RPC endpoint. RPC acceptance is
  recorded in `accepted_by_rpc`, but `l1_da_published` stays false because this
  is still a projected package path rather than the final L1 DA publication
  script. Direct live submission first queries CKB `get_live_cell` for every
  input and cell dep in the projected transaction and refuses to broadcast if
  any input or dep is stale or if live input capacity is below output capacity.
- `carrier-submission` builds a compact CKB carrier transaction for a DA-anchor
  or settlement package. It validates the 160-byte package carrier payload,
  recomputes the payload data hash, requires the payload kind to match the
  package kind, requires every 32-byte compact payload field to match the
  package's declared DA-anchor or settlement fields, derives the `data2` type
  args from `ckb_data_hash(carrier_payload) || carrier_identity_hash`,
  constructs the explicit change output, rejects carrier or change outputs
  below their CKB occupied-capacity minimum, reports those minimums, emits the exact
  `send_transaction` JSON-RPC request, and can either dry-run the request,
  submit it directly to a configured plain-HTTP CKB RPC endpoint, or record a
  separately supplied accepted CKB transaction hash. Accepted carrier reports
  keep the projected CKB raw transaction hash in `ckb_raw_tx_hash`. The command
  only marks `accepted_by_rpc = true` for a live `--submit` RPC response whose
  tx hash equals that locally projected raw transaction hash; a
  `--accepted-tx-hash` value is recorded separately and does not satisfy strict
  live production readiness.
- `verify-submission-context` queries CKB `get_live_cell` for every input and
  cell dep in a DA-anchor or settlement submission report. It marks the
  transaction ready for CKB submission only when all required cells resolve as
  live.
- `verify-submission-economics` queries CKB `get_live_cell` for transaction
  inputs, sums live input capacity and output capacity, computes the implied
  fee, and marks `economically_ready = true` only when capacity balance and the
  configured fee floor, fee-rate policy, and optional maximum-fee policy are
  satisfied.
- `verify-submission-inclusion` queries CKB `get_transaction` for a DA-anchor or
  settlement submission report. It distinguishes mempool acceptance from live
  inclusion: `live_l1_observed = true` only when CKB reports `committed` with a
  block hash.
- `verify-submission-stability` re-queries CKB `get_transaction` for an
  inclusion report and marks `stable_block_identity = true` only when the
  observed committed block hash and block number still match the source
  inclusion.
- `verify-submission-finality` queries CKB `get_tip_header` for an inclusion
  report and computes confirmation depth from the committed block number. It
  marks `finality_confirmed = true` only when the configured confirmation policy
  is met.
- `verify-submission-readiness` is a non-network aggregate verifier. It consumes
  the context, economics, inclusion, stability, and finality reports and marks
  `production_submission_ready = true` only when all reports bind to the same CKB
  transaction, the same committed block identity, the same submission artefact
  lineage, the supplied inclusion artefact lineage, and every readiness marker
  is true. It separately marks `strict_production_submission_ready = true` only
  for live final L1 script evidence whose referenced submission report also
  carries the required final-script pre-submit checks. With
  `--require-live-submission`, it also requires the referenced submission report
  to prove non-dry-run RPC acceptance by carrying a matching `rpc_result` for
  the projected CKB transaction hash, a non-empty RPC URL, and no RPC error. The
  output separates `production_submission_ready`,
  `strict_production_submission_ready`, `readiness_evidence_mode`,
  `live_carrier_submission_ready`, and
  `final_l1_script_submission_ready`, so mock/offline readiness, live carrier
  readiness, and final L1 script readiness remain machine-distinguishable.
- `settlement-intent` turns a verified court bundle plus verified DA manifest
  into a disputed-close settlement object bound to the session id, chunk index,
  state roots, challenge payload hash, court-bundle hash, DA-manifest hash, DA
  segment root, and challenge deadline. It deliberately carries
  `l1_da_published = false` and `l1_court_implemented = false` so the current
  artefact cannot be mistaken for an externally published DA record or on-chain
  settlement script.
- `verify-settlement-intent` recomputes those bindings and rejects premature or
  tampered settlement permission.
- `settlement-package` turns a valid, permitted settlement intent into a
  deterministic CKB-compatible settlement CellTx package. The package binds the
  exact intent JSON bytes, court-bundle hash, DA-manifest hash, challenge
  payload hash, and final state root; it also declares `settlement_authority`,
  a one-use authority Cell requirement whose 192-byte data payload is
  `intent_hash || session_id || participant_set_hash ||
  escrow_input_cells_hash || session_lineage_commitment ||
  session_authority_commitment`, whose `data_hash` is the expected CKB data
  hash, whose session lineage fields are copied from the verified session path,
  whose authority commitment binds those lineage fields together with the intent
  hash and `session_id`, whose consumed input index is `1`, and whose lock must
  match the final DA publication lock.
  Final settlement type args are `session_id_hash || settlement_identity_hash`,
  where `session_id_hash` is the session id field from the consumed authority
  Cell and `settlement_identity_hash` is the CKB data hash of the 160-byte final
  settlement payload. The final settlement script rejects same-type inputs,
  duplicate same-type group outputs, and any second output in the transaction
  using the same deployed final-settlement code hash/hash type. This gives
  transaction-local singleton creation; cross-transaction replay is blocked by
  consuming the one-use authority Cell. Participant-authenticated authority-cell
  creation remains a separate production blocker.
  The package also carries the Molecule transaction bytes and projection report
  needed by the future CKB court path.
- `verify-settlement-package` recomputes the intent hash, embedded Molecule
  transaction hash, CellTx id, witness txid, deterministic settlement CellTx,
  CKB-compatible projection, and settlement-authority requirement. It deliberately keeps
  `l1_court_script_implemented = false`; this is a package and verifier, not
  yet an on-chain court deployment.
- `submit-settlement-package` validates the package, converts the embedded
  CellTx into CKB `send_transaction` JSON, and either dry-runs the request or
  submits it to a configured plain-HTTP CKB RPC endpoint. RPC acceptance is
  recorded in `accepted_by_rpc`, but `l1_court_submitted` stays false because
  this is still a projected package path rather than the final L1 court script.
  Direct live submission first queries CKB `get_live_cell` for every input and
  cell dep in the projected transaction and refuses to broadcast if any input
  or dep is stale or if live input capacity is below output capacity.

Acceptance criteria:

- Both static committee and Tendermint modes pass for the built-in fixture.
- Tampering with the state root fails verification; the same verifier structure
  also recomputes the transaction hash, projection, canonical block hash,
  challenge hash, and finality certificate.
- JSON outputs are deterministic across repeated runs.
- Settlement intent verification passes only after the configured challenge
  window has elapsed, and the report remains explicit that L1 court settlement
  is not implemented.
- Settlement package verification proves that the permitted intent can be
  projected into a deterministic CKB-compatible CellTx package for both
  consensus modes, while keeping the L1 court-script marker false. It also
  proves that the package declares the final-settlement authority Cell
  requirement and that the declared authority data, CKB data hash, input index,
  and lock-binding rule all match the settlement intent.
- Settlement submission dry-run proves the CKB JSON-RPC request can be built and
  hashed for both consensus modes. Live court settlement still requires an
  operator-supplied CKB RPC endpoint, accepted transaction response, and real
  deployed script semantics.
- Submission context verification proves the CKB `get_live_cell` preflight can
  reject missing or spent inputs/cell deps before live submission. The production
  gate covers this with a mock CKB RPC server; real production still needs
  spendable cells and deployed script deps on a public chain.
- Submission economics verification proves a submission can be checked for live
  input capacity, output capacity, absolute fee floor, fee-rate policy, max-fee
  policy, and explicit change-like output accounting before live submission. The
  production gate covers this with a mock CKB RPC server; real production still
  needs wallet-funded cell selection, dynamic fee-rate policy, signing, and
  operator-owned change-output construction. The gate uses exact-funded mock
  inputs plus a maximum-fee cap so silent overpayment is not counted as
  production readiness, and the focused CLI regression distinguishes explicit
  returned change from surplus burned as implicit fee.
- Submission inclusion verification proves the CKB `get_transaction` verifier
  can observe committed transaction status. The production gate covers this with
  a mock CKB RPC server; a real production release still needs public testnet or
  mainnet committed transactions.
- Submission stability verification proves a committed inclusion can be
  re-checked for block-identity drift before treating it as stable. The
  production gate covers the stable case with a mock CKB RPC server; real
  production still needs live reorg/retry rehearsal.
- Submission finality verification proves the CKB `get_tip_header` verifier can
  compute confirmation depth from a committed inclusion report. The production
  gate covers this with a mock CKB RPC server; a real production release still
  needs public-chain confirmation-depth rehearsal and reorg handling.
- Submission readiness aggregation proves the operator-facing decision cannot be
  assembled by cherry-picking unrelated green reports. It now checks expected CKB
  transaction hash agreement, committed block identity agreement, common
  context/economics/inclusion submission lineage, and stability/finality lineage
  back to the supplied inclusion artefact. It also re-opens the referenced
  submission report and requires that report to be readable, schema-matching,
  and bound to the same expected CKB transaction hash. In strict mode
  (`--require-live-submission`), readiness additionally requires the referenced
  submission report to prove non-dry-run RPC acceptance through the deployed
  carrier path with a matching `rpc_result`, non-empty RPC URL, and no RPC
  error. For final-script evidence, strict readiness additionally requires the
  referenced submission report to carry positive live preflight markers for the
  funding input, lock dep, verifier code dep, and verifier code hash; final
  settlement also requires positive final DA evidence CellDep and
  settlement-authority input data/lock checks. Direct DA-anchor or settlement
  package submission reports can still record RPC acceptance, but strict
  production readiness does not treat them as final L1 evidence while the
  corresponding DA/court scripts remain unimplemented.
  `strict_production_submission_ready` makes that distinction explicit. The
  aggregate report labels the evidence mode as
  `coherent-offline-or-mock`, `live-ckb-carrier`, `final-l1-script`, or
  `not-ready`, so downstream tooling does not have to infer that distinction
  from notes. It is still only as strong as the underlying evidence; real
  production requires live CKB reports, not mock RPC evidence.
- DA manifest verification proves the replay payload is available under the
  current single-segment Merkle profile. In the gate it must be backed by a
  sealed local segment; it must not claim external L1 DA publication.
- DA anchor package verification proves the manifest can be projected into a
  deterministic CKB-compatible anchor CellTx for both consensus modes, while
  keeping the L1 DA submission marker false.
- DA anchor submission dry-run proves the CKB JSON-RPC request can be built and
  hashed for both consensus modes. Live L1 publication still requires an
  operator-supplied CKB RPC endpoint and an accepted node response.
- `scripts/myelin_ckb_devnet_smoke.sh` uses the parent `../ckb` checkout to run
  a throw-away devnet, generate Myelin DA-anchor and settlement packages, deploy
  separate compact CellScript DA-anchor and settlement carrier verifiers, commit
  the DA-anchor package's 160-byte compact carrier payload into a live CKB
  carrier transaction guarded by the DA `data2` verifier, bind carrier type args
  to `ckb_data_hash(carrier_payload) || carrier_identity_hash`, where the
  identity hash is the DA manifest hash or settlement intent hash, submit a
  second verifier-guarded live CKB carrier for the settlement package's compact
  payload from the DA carrier's change output, mine both until `committed`, run
  the submission context/economics/inclusion/stability/finality/readiness
  verifiers against live CKB RPC evidence for both carriers, submit
  self-consistent zero-hash-field compact carriers under both deployed
  verifiers, require CKB script verification to reject both, and emit
  `myelin-ckb-devnet-smoke-v1`. The smoke copies the checked-in
  `cellscript/examples/myelin/da-anchor-carrier.cell` and
  `cellscript/examples/myelin/settlement-carrier.cell` sources into the
  throw-away workdir, compiles each source under `typed-cell` first, records the
  typed-cell ELF and metadata sidecar paths in the final report, then compiles
  the CKB-profile ELF that is deployed on devnet. The carrier verifiers reject
  zero hash fields, reject a second same-type group output through
  `ckb::cell_exists`, require
  exact 64-byte type args through a CellScript entry witness, and bind type args
  to `ckb_data_hash(carrier_payload) || carrier_identity_hash`. The carrier
  submission reports, exact `send_transaction` JSON-RPC request bodies, and
  live valid-carrier RPC submissions are handled by
  `myelin session carrier-submission`, not by ad hoc shell JSON assembly. When
  the verifier source file is available, the carrier submission report records
  a SHA-256 hash of that CellScript source next to the deployed verifier code
  hash and code dep. Live carrier submission now requires `--verifier-source`
  to resolve to a readable CellScript source file, so an accepted live carrier
  report cannot silently carry a null verifier source hash. Before broadcasting
  through `send_transaction`, the CLI also preflights the declared input and
  code deps through CKB `get_live_cell`, rejects a missing or stale input,
  rejects an input capacity that differs from the declared funding capacity,
  rejects a live funding-cell lock that differs from the declared funding/change
  lock, and rejects a verifier code dep whose live `cell.data.hash` differs from
  the declared verifier code hash. It also computes the carrier transaction's
  projected CKB raw transaction hash locally and rejects `--require-accepted`
  unless the CKB RPC result equals that hash. For final settlement, the CLI
  additionally requires the package-declared authority Cell as input `1` and the
  final DA publication Cell as an evidence CellDep. Live preflight rejects a
  missing, stale, capacity-mismatched, or data-hash-mismatched authority input,
  and rejects either the authority input or final DA publication CellDep when
  its live lock does not match the declared settlement output lock. The inclusion
  verifier requires carrier
  `outputs_data[0]` to match the declared carrier payload and
  `outputs[0].type` to match the declared verifier code hash, `data2` hash
  type, and expected data-hash-plus-identity args before it marks live inclusion
  as observed. The context verifier also requires the declared verifier code
  dep to resolve live and its returned `cell.data.hash` to match the declared
  verifier code hash before it marks carrier submission context ready. The
  devnet smoke runs aggregate readiness with `--require-live-submission`, so
  each carrier readiness report must prove the original submission used the
  carrier schema, was a non-dry-run RPC-accepted report whose `rpc_result`
  matches the projected CKB transaction hash. The report only sets
  `all_live_checks_passed` when the positive carrier checks and both negative
  tamper checks pass. This narrows the gap between mock RPC evidence and live
  CKB handling. Final DA-anchor and final settlement CellScript verifier
  artefacts now exist, compile under the local typed-cell and CKB profiles, and
  are deployed as code cells in the devnet smoke, but the live submission path
  still uses the compact-carrier verifiers rather than final DA/court submission
  transactions.
- CellScript now includes focused typed-cell package-commitment regressions:
  `v0_18_myelin_package_commitment_has_typed_cell_metadata_and_ckb_vm_rejects_tamper`
  and
  `v0_18_generic_package_commitment_binds_data_hash_to_type_args_in_ckb_vm`.
  They compile `PackageCommitment` under the local `typed-cell` profile and
  execute package-commitment verifiers as CKB type scripts in `ckb-testtool`.
  Matching output data passes and tampered output data fails. The
  `v0_18_myelin_da_and_settlement_carriers_bind_compact_payloads_to_type_args_in_ckb_vm`
  regression compiles typed-cell DA-anchor carrier, settlement carrier, final DA
  publication, and final settlement resources and executes compact 160-byte
  verifier shapes in `ckb-testtool`, requiring type args to carry both the full
  payload data hash and the resource identity field, rejecting duplicate
  same-type group outputs, rejecting same-type group inputs for final-script
  cells, requiring final settlement to read the final DA publication through a
  read-only `CellDep` whose data hash and `data2` type args bind the settlement
  DA manifest hash, requiring a consumed settlement-authority input whose first
  32-byte data field equals the settlement intent hash and whose remaining data
  fields carry the session id, participant digest, escrow digest,
  session-lineage commitment, and authority commitment, requiring the settlement
  output type args to equal `session_id_hash || settlement_identity_hash`,
  requiring both the authority input and settlement output lock hashes to match
  the DA publication lock hash, rejecting competing final-settlement outputs
  under the same deployed final-settlement verifier in one transaction, rejecting
  non-canonical extended type args, and rejecting self-consistent zero-hash-field
  payloads. The live devnet smoke now deploys separate compact
  carrier verifiers and final-script verifier artefacts into the throw-away
  chain, creates the one-use settlement-authority Cell from the settlement
  package's declared `settlement_authority`, records the authority session id,
  participant digest, escrow digest, session-lineage commitment, and authority
  commitment in the smoke report, uses the carrier verifiers for live carrier
  readiness, uses the final-script verifier artefacts for live `final-l1-script`
  readiness, consumes the authority Cell in final settlement, confirms a
  competing final-settlement output probe is rejected by live CKB script
  verification, and confirms self-consistent zero-field carriers are rejected
  under both deployed verifiers. The CLI context regression
  rejects carrier readiness when the verifier code dep data hash does not match
  the declared verifier code hash. The CLI readiness regression rejects mixed
  report lineage even when transaction hashes and block identities are otherwise
  green. The remaining gap is production-grade DA and court semantics beyond the
  current compact payload binding.

## 7. P3: Production Gate Hardening

The production gate should prove that Myelin remains a Session L2, not merely a
collection of passing crates.

Implemented or preserved checks:

- Runtime smoke for both consensus modes.
- Session fixture open/commit/court/verify/DA/DA-anchor-submit-dry-run/
  settlement-intent/package flow for both consensus modes.
- Teeworlds acceptance as workload evidence, not as the only session model.
- Dependency-tree scan proving hot-path crates do not pull broad system/network
  utility surfaces.
- Stale-surface scan for removed chain identities and legacy serializer paths.
- CKB-strict court-profile assertion.

Gate failure conditions:

- Session state roots differ across consensus engines for the same CellTx
  sequence.
- Static committee and Tendermint finality evidence are interchangeable.
- A court bundle verifies without recomputing the canonical block/session hash.
- A settlement intent verifies without being bound to a verified court bundle,
  verified DA manifest, and elapsed challenge window.
- A DA manifest verifies without being bound to the exact court replay payload.
- A CKB-compatible court claim is emitted under an unproven Myelin-only profile.
- `myelin-exec` reintroduces default legacy serializer fallbacks, placeholder
  compression APIs, or non-essential serialization helper frameworks into the
  root public surface.

## 8. Documentation Updates

Update the top-level documentation to use one consistent narrative:

```text
Myelin is a finite Cell session L2 with CKB-compatible court projection.
```

Required edits:

- README: state that independent-chain work is out of scope for this milestone.
- Architecture docs: describe session lifecycle before discussing consensus
  internals.
- Teeworlds docs: keep Teeworlds as pressure workload evidence, not the complete
  product.
- Production gate docs: include the new session commands and CKB-strict profile
  requirement.

## 9. Milestone Exit Criteria

The Session L2 milestone is complete when all of the following are true:

- A deterministic session fixture runs through open, commit, court-bundle, and
  verification.
- A settlement intent can be emitted and verified from the court bundle and DA
  manifest while explicitly marking external DA publication and the on-chain L1
  court as unimplemented.
- A DA manifest can be emitted and verified from the court bundle while
  explicitly marking external DA publication as unimplemented.
- Static committee and Tendermint both finalise the same session transition.
- The state transition is consensus-independent; only finality evidence differs.
- The court bundle is self-contained and rejects tampering.
- The court-facing VM profile is CKB-strict by default.
- Teeworlds remains passing as an external workload.
- The production gate enforces all of the above.

At that point Myelin can be described as a serious Session L2 prototype. It
should still not be marketed as an independent chain.
