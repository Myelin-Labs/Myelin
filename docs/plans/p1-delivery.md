# P1 delivery and acceptance record

Scope: every open P1 issue in Myelin-Labs/Myelin at the start of this work.
The complete issue descriptions and acceptance criteria are preserved in
[the issue snapshot](p1-issue-snapshot.json); a passing subset does not close
an issue or the overall goal.

Baseline: `f0013339a1459577ab44d02bbcf7fb28e0796f48`, Rust/Cargo 1.92.0.
Branch: `codex/p1-issues`.

| Issue | Deliverable | Status |
| --- | --- | --- |
| #7 | Versioned cross-session protocol ADR, failure model, golden vectors | RFC and reference vectors implemented locally; published in PR #23 |
| #8 | Isolated handoff gates, competing consumers, restart and atomicity checks | Implemented and verified locally; published in PR #23 |
| #9 | Genesis-bound aggregate resource budgets and host isolation | Protocol budgets and Linux worker verified locally; bounded archive/recovery and isolated bundle replay are integrated, with real filesystem-capacity recovery evidence; published in PR #23 |
| #10 | Replayable application authorization of issued protocol effects | Execution/replay boundary, trust classes, reusable scarce/copyable policies and adversarial checks implemented locally; trusted-code archive replay covers transfer and cancellation/refund; archived transfer replay also passes isolation/empty-container restore; complete historical build provenance is retained with #13; published in PR #23 |
| #11 | Shared-store lock/consume/cancel/refund reference state machine | Signed payloads, lock/intake, competing targets, cancellation/refund and consumed-lock retirement implemented locally; exact continuation now inherits outstanding obligations with bounded canonical ownership, fresh cancellation authorization, CLI/restart/crash and archive replay checks; published in PR #23 |
| #12 | Durable command journal and unknown-outcome reconciliation | Durable original-command storage, reservations, reconstruction and exact receipt/business-result recovery implemented locally; signed later-epoch absence recovery and remote submission/reconnect adapter are implemented; standalone inventory/turn/fixed-tick examples now pass local crash/replay acceptance; both downstream integrations and independent local release artifacts are verified under #19; published in PR #23 |
| #13 | Portable offline evidence bundles and replayer manifests | Bounded inventory/export, independent trust, protocol dependencies, finality audit, pinned sandbox replay and offline OCI restore pass; complete typed build kits export/restore and reproduce the original native binaries offline, whose original verifier replays the same history/result; all three command/fixed-tick example archives now pass original-program replay after source deletion; local requirement mapping recorded below; published in PR #23 |
| #14 | Successor compatibility, migration and command inheritance | Typed continuation/compatibility/migration, atomic converted genesis, authority separation and outstanding-obligation inheritance implemented locally; isolated historical bytecode migration and signed-invalid-output rejection pass after source deletion; full target/predecessor conflict checks, successful/unsupported CLI examples and crash checks pass; published in PR #23 |
| #15 | Authoritative cross-host writer fencing and migration drills | Single-authority canonical gateway, epoch/watermark enforcement and certified effect ledger implemented locally; mTLS service/client and journal adapter pass real local network/process fault drills; explicit shared canonical ledger and signed remote claim terminal reconciliation pass lost-response/racing-terminal/reopen drills; original acceptance clauses are mapped below; published in PR #23 |
| #18 | Archive retention, bounded recovery and dependency preservation | Bounded audit/replay, checkpoint lookup, hot-to-cold retention, exact pin rebinding, online backup/restore and terminal payload compaction implemented locally; Linux superseded archive payload collection checks owned databases/checkpoints, protected roots, active readers and complete replacement dependency coverage, with restart after process death; final publication now reserves its complete validation sweep before reading, exposing the next finite grant and preventing repeated-prefix livelock; long-history, many-session and 2/8/32-authority-rotation measurements and reference RPO/RTO objectives recorded; published in PR #23 |
| #19 | Shared game adapters, conformance and downstream adoption | Shared command/fixed-tick profile, query and result adapter with three native examples implemented locally; explicit tick clock, integer/RNG policy and replayable control events use the same journal/recovery contracts; Chrome/WASM example-rule conformance passes exact native roots/results and checkpoint restoration; all three examples have complete build/runtime archives and isolated original-program replay; Chess and Neon Miner now use shared adapters/journal/fenced recovery in separate worktrees; real HTTP retry/restart/isolation, explicit Chess seat admission, complete Miner campaign and original-executable replay pass; independently pinned source packages and actual read-only release container builds/restart/offline replay pass locally; published in PR #23 |
| #20 | Reproducible workload, fault, recovery and isolation measurements | Existing entrypoint covers inventory/turn/tick rules, strict CellTx and authorized match-to-inventory rewards at three scales, four triggers and shared/separate processes; 72 normal host cases and history/fault drills, 24 strict CellTx cases, 27 reward cases and two capacity cases pass; controlled TCP evidence providers pass slow/offline/lost-reply/restart drills; actual ext4 capacity exhaustion passes both layouts, independent services progress during the fault, and all original receipts survive remount/restart/archive replay; nine reward/provider and eight cross-class regressions pass on the quota source; existing authority/network drills satisfy cross-worker fencing without another gate; real Chess SQLite write outage preserves canonical receipts and independent progress; 30 storage-breakdown cases pass; original acceptance mapping is complete below; published in PR #23 |
| #21 | Portable CI acceptance lanes and exact release evidence | One validation entry point selects required profiles/lanes and records exact source, commands, artifacts and pass/fail/skip outcomes; Linux/macOS core workflow implemented locally; a frozen core snapshot passes in a fresh offline container, current local core passes and real browser evidence is recorded separately; local lane-scope/evidence mapping recorded below; published in PR #23; hosted execution and current check status are linked below |

## Publication and hosted validation

The implementation and all 63 original acceptance mappings are published in
[Myelin PR #23](https://github.com/Myelin-Labs/Myelin/pull/23). Downstream adoption
is published independently in [Chess PR #1](https://github.com/UTXOPlay/myelin-chess/pull/1)
and [Neon Miner PR #1](https://github.com/UTXOPlay/myelin-neon-miner/pull/1).
Miner preserves the pre-existing UI changes in a separate signed baseline commit.
Publication here means reviewable source and evidence; it does not imply a merged
main branch, a production deployment, or a larger release-profile pass.

The first hosted attempt could not start its retired macOS 14 runner. After
selecting macOS 15, [run 42257169685](https://github.com/Myelin-Labs/Myelin/actions/runs/42257169685)
passed formatting and strict Clippy on both platforms but failed workspace tests.
Ubuntu 22.04's bubblewrap lacked --disable-userns; its replacement is the upstream
0.13.1 release, built after SHA-256 verification without weakening the sandbox.
The macOS image selected OpenSSL 1.1.1w. Its default configuration and the fixture
command both supplied basicConstraints, producing an invalid duplicate extension.
The same CertificateParseError was reproduced locally at the same test line.
A fixture-owned minimal OpenSSL configuration removes the ambient extension.
All nine existing authority-network tests then passed with OpenSSL 1.1.1w and
4.0.3; no tests, gates or version branches were added.

Those repairs are signed commit 8fb2c7bf66578c913a9fe82377a9a8cb02cbd6c5.
The [PR checks](https://github.com/Myelin-Labs/Myelin/pull/23/checks) carry the
current hosted result; each uploaded report binds its own exact checkout and
artifacts. These scoped repair results do not replace the complete hosted core
outcome. Earlier failures remain failures, and the frozen local campaigns below
retain their original source identities.

## Scope fidelity

The original issue snapshot remains the acceptance boundary. #13 asks for offline
replay on a clean machine; #15 asks for two independently active RocksDB copies;
#18 asks for a resumable budget cursor. It does not explicitly require a second
physical host or serialization of an in-flight audit cursor. Those additional
deployment experiments must not become invented blocking gates. Existing clean
container and isolated replay evidence is mapped below; the other issue audits
use their actual clauses as mapped below. No issue is closed by this clarification.
Remote claim terminal reconciliation is now exercised under the declared
single-authority/shared-ledger trust model. Reference deployment recovery objectives
and independent downstream source/image releases are now recorded below. The
specified #20 workloads/faults remain real requirements. Reference RPO/RTO targets
are operational objectives, not measurements of production availability.

## Requirement mapping — protocol and application behavior

Together with the #13/#15/#21 tables below, these rows map all 63 original
acceptance clauses to their local implementation and retained evidence.
The requirement text is preserved in the issue snapshot. This mapping does not
turn proposed remote protocols into implemented features or mark a hosted release
as executed. The user's later current-only cleanup supersedes old compatibility
and test-count-preservation requirements.

| Issue / original acceptance | Implementation and evidence |
| --- | --- |
| #7 — ADR, types, state machines, errors and assumptions | [ADR-008](../architecture/cross-session-protocol.md) defines each section and labels the generic envelope as proposed; the linked reference implementations define their actual current encodings. |
| #7 — one/separate stores, authorities and sealed-source messages | ADR “Concrete topologies” covers all five cases, including rejected cross-ledger scarce intake; transfer continuation tests retain original handoff identities and inherited obligations. |
| #7 — identify each guarantee provider | ADR distinguishes storage atomicity, application authorization, remote coordination, DA and actual L1 enforcement; no layer silently promotes another layer's evidence. |
| #7 — public golden vectors | [Cross-session corpus](../../fixtures/protocol/cross-session-v1.json), standalone encoder/example and session/tests/protocol_vectors.rs run without a private platform database; current core includes the vector tests. |
| #7 — Registry/relay is not consensus | ADR topologies and [writer fencing](../architecture/writer-fencing.md) exclude discovery/relay from authoritative ownership and terminal-state decisions. |
| #8 — each dedicated gate detects weakening | handoff_regressions.rs uses named ordinary/expiring/gated IDs, isolates violated conditions and checks precise reasons; target, timestamp, evidence pipeline/stage and prepare-to-commit changes have separate cases. |
| #8 — successful evidence-gated consumption | handoff_evidence_requires_the_exact_pipeline_and_minimum_stage progresses absent → insufficient → sufficient and commits the final case. |
| #8 — competing consumers/restart have one effect | handoff_policy_targets_compete_atomically_after_both_prepare and real process-exit/reopen cases check the exact head, consumption, command receipt, target state and outbox together. |
| #8 — retain historical test record | The original 756-test statement remains in the issue snapshot. Later user-directed deletion/consolidation supersedes preserving every old test; current core reports its actual 794 Rust tests, not an additive coverage claim. |
| #9 — aggregate boundary cases | resource_regressions.rs covers the original zero-CellTx megabyte probe, many small handoffs, authorization totals, outboxes, snapshots and complete records including finality proof. |
| #9 — rejected work changes no durable effects | The same tests check unchanged head/receipts/outbox/markers, including direct store writes, consumed plus issued payload totals and offline decode. |
| #9 — host wall/RSS separate from VM cycles | session-worker/tests/process.rs exercises real child resource failures; worker reports retain wall/CPU/RSS independently. The host/strict-Cell workload reports retain zero/nonzero VM cycles as different execution classes. |
| #9 — document old byte limit and new budgets | [Commit resources](../architecture/commit-resources.md) states that max_block_bytes counts CellTx bytes and lists genesis-bound aggregate limits plus operational/whole-disk boundaries. |
| #10 — recipient/amount/object/version/expiry/authority mutation | effect_regressions.rs and session-transfer/tests/transfers.rs check actual owner signatures and mutated intent fields; successor authorization binds the complete target configuration. |
| #10 — reject missing disposition, arbitrary authorization, duplicates | owner_authorization_and_source_disposition_fail_closed_without_durable_effects, default-policy rejection and signed-but-unauthorized source replay cover these cases without treating a nonempty byte string as proof. |
| #10 — valid transfer replays exact effects | valid_lock_and_target_credit_replay_after_restart_with_identical_effect_context, transfer integration tests and the archived original replayer compare roots and effect/range commitments. |
| #10 — custom policy and exported trust | ProtocolEffectsContext, explicit Disabled/AuthorityMetadata/Application policies and manifest/config binding expose the trust model; Application intake rejects metadata promotion. |
| #10 — no deployed bridge claim | [Source effects](../architecture/protocol-effects.md) and [reference transfer](../architecture/reference-transfer.md) state local shared-store scope and separate host replay from L1/court enforcement. |
| #11 — one competing target wins | Transfer policy-target and storage handoff races contend on one authoritative key after both candidates prepare. |
| #11 — cancellation/consumption/refund conservation | lifecycle.rs races cancel/consume, rejects refund-before-cancel and replay after cancellation; retirement.rs removes consumed backing without refunding it. |
| #11 — source/target/ACK/cancel/refund crashes | crashes.rs and lifecycle.rs terminate actual child processes at the named boundaries, reopen twice, recover original outcomes and verify balances/locks/nullifiers. |
| #11 — obligations survive continuation or sealing | succession.rs checks inherited locks, fresh cancellation authorization, competing target consumption, complete rightsholder approval and the 32-link ownership bound; archive dependency tests replay both terminal paths through authority rotations. |
| #11 — independent runnable example, correct atomicity scope | The existing transfer CLI runs exact and policy targets, restart/replay, continuation and lifecycle operations; its 44 invocations pass in core. It explicitly requires a shared store and does not claim a transaction across independent stores. |
| #12 — crashes through HTTP response | durable.rs::crashes_around_write_reserve_prepare_commit_ack_and_http_response, fenced crash tests and the three examples' 21 real exits exercise original-command recovery; downstream HTTP tests add actual lost replies/restarts. |
| #12 — stable ID/result, payload conflict, scoped signatures | original_signature_identity_and_result_survive_lost_reply_and_reopen, identity-conflict and forged/retargeted command cases retain the exact original business result and reject changed scope. |
| #12 — valid prefix and Unknown reservations | stale_prefix_reconstruction_preserves_intents_and_unknown_work, mixed conflict/missing cases and signed later-epoch reconciliation preserve unproven submissions; same-epoch Missing never releases them. |
| #12 — inventory and turn examples | session-journal/examples/inventory.rs and turn_based.rs share the adapter, crash checks and original-program archive replay; fixed ticks uses the same recovery contract. |
| #12 — do not reinterpret tape history | CommandProfile binds a distinct input schema, program and authentication/clock policy. New command profiles do not reinterpret old tapes; current-only storage readers reject older formats and archived originals retain their own interpreters. |
| #14 — authority separation | rotated_authority_cannot_reinterpret_or_finalize_the_old_session and per-session bundle trust/finality keep old and new verifiers separate. |
| #14 — wrong predecessor/existing target/sealed writes/duplicates | Store successor conflict tests and succession_regressions.rs validate full predecessor/target bindings and atomic unique target creation; source sealing blocks later writes. |
| #14 — namespaces cannot recreate commands/rewards | Signed migration fixtures retain economic nullifiers while resetting only explicit fields; reference transfer copies nonces/receipts and requires newly scoped authorization for inherited actions. |
| #14 — incompatible offline migration and crashes | Isolated archived migration bytecode runs after source deletion; a correctly signed forged output fails actual replay. Real process exits preserve atomic source sealing and successor creation. |
| #14 — successful and unsupported CLI upgrades | The existing transfer CLI accepts authorized exact continuation and rejects the unsupported rule upgrade; no new upgrade gate is needed. |

## Requirement mapping — retention, games and measurements

| Issue / original acceptance | Implementation and evidence |
| --- | --- |
| #18 — measure history/session/epoch costs | The workload guide retains 4,096-block history, three normal scales and 2/8/32-authority dependency observations, including storage, RSS, export and recovery. The later 30-case storage breakdown records actual WAL/block/receipt/dependency-index sizes. |
| #18 — missing data fails explicitly | Bundle inventory/dependency/retention tests reject missing artifacts, cold bytes, causal edges and changed roots; resumable work never reports verified before its complete final sweep. |
| #18 — clean offline historical replay | The frozen clean-container and complete-build reports restore archived original programs without the producing database/service; current sandbox tests separately verify the latest runtime contracts. Their source identities are not conflated. |
| #18 — RPO/RTO method and targets | [Reference recovery objectives](../operations/workload-measurements.md#reference-deployment-recovery-objectives) define exact acknowledged-result comparison, measured phases, a 60 s qualifying target and explicit surviving-volume conditions. The target is not represented as measured end-to-end availability. |
| #18 — index reconstruction cannot alter canonical facts | Backup/restore and dependency-catalog reconstruction compare original heads, balances/nullifiers and range receipts. Missing canonical indexes fail audit rather than inventing history; actual Chess projection repair recovers the original canonical room and independent game receipts. There is no unchecked store reindex/upgrade API. |
| #19 — three example classes share integration checks | applications.rs parameterizes inventory/turns/ticks through the same batching, restart and process-exit contracts; each archived original program replays its corresponding history. |
| #19 — identical roots/results across browser/batching/restart | Current browser evidence has ten vectors/64 exact native-WASM comparisons. identical_commands_ignore_batch_height_and_restart_boundaries checks application roots/results while allowing block hashes to reflect different batch boundaries. |
| #19 — duplicates/staleness/authorization/reconnect/timeouts | Application tests reject forged seats/system kinds, stale/invalid ticks and wrong profiles; example controls and downstream HTTP flows cover explicit timeout/takeover, reconnect and original-result retry. |
| #19 — two independently runnable/releasable adopters | Chess and Neon Miner use shared adapters/journal/fenced recovery. The pinned source/image release report retains actual builds, HTTP duplicates, whole-container restart and original-executable offline replay after volume deletion. Publication of these local artifacts is separate. |
| #19 — capabilities reject unsupported operations | snapshot_profile_and_canonical_encoding_are_required_and_capabilities_are_enforced checks the manifest and rejects unsupported handoff/successor/settlement paths; host profiles do not claim CKB-VM execution. |
| #20 — three reproducible scales and raw reports | One documented workload entrypoint fixes seeds, input plans, compiler/source/binary identities, raw samples and case outcomes; all five kinds cover three scales and both process layouts. |
| #20 — measured trigger/resource recommendations | The guide compares Instant/Interval/Open/manual, documents workload-specific starting settings and observed backlogs, and defines provisional reward-credit and evidence-burst targets without claiming sustained production capacity. |
| #20 — faults check canonical results and conservation | Original-command lost/unknown outcomes, process exit, real TCP provider faults, actual ext4 ENOSPC and Chess SQLite writer outage compare heads/receipts/results and relevant reward terminal states; they do not stop at process health. |
| #20 — long-history recovery and fencing | The retained 4,096-block drill deletes the original DB before archive replay. Existing authority/network tests keep independent stale replicas active, rotate epochs, lose replies and reconcile exact commands/claim terminals. They satisfy the fencing drill without a duplicate benchmark lane. |
| #20 — Teeworlds explicit outcome | Workload reports mark Teeworlds skipped. The independent selected Teeworlds lane requires its real external checkout/assets and fails if missing; no skip is counted as passed. |

The current core log was checked against all 123 test entrypoints in the 17
protocol/storage/transfer/journal/archive files used by this mapping. Child-process
test output may place the parent's final “ok” after nested process output; the
audit preserves those complete result spans rather than assuming one-line logs.
This audit only reads the retained successful report; it adds no test or gate.

## Requirement mapping — writer fencing and migration

This maps #15 to the explicit single-authority reference. It does not claim
quorum HA or terminal consensus between independently owned claim ledgers.

| #15 acceptance | Implementation and retained evidence |
| --- | --- |
| Two active independent RocksDB copies cannot create accepted old-worker progress or rights | `independent_active_replicas_cannot_fork_canonical_progress_or_external_rights` rejects a forged replica proof and duplicate effects; the remote claim flow keeps two stale replicas open while the canonical source/target advance |
| Partitions, drift, lease expiry, takeover crashes and stale backups have defined outcomes | The writer-fencing failure table and migration runbook define epoch-independent clocks/leases and unavailable-authority behavior; real TLS disconnect/lost-response tests, grant/commit races and eleven authority process-exit boundaries preserve exact outcomes |
| Existing local CAS coverage remains | `durable_head_cas_rejects_two_writers_at_one_height` and the competing-consumer/lifecycle suite pass in the same core run |
| Unsafe takeover stays blocked/pending without replacing history | Missing/wrong epoch stores, rolled-back canonical heads and mismatched grants reject; journal Unknown stays reserved until a matching receipt or later-epoch command-absence certificate; Missing/Issued claim observations never authorize refunds |
| Migration runbook and drill evidence without Registry consensus | `docs/operations/writer-migration.md` covers drain, original-command recovery, grant confirmation, issuer claim reconciliation, backup and effect delivery; the core report below retains the real-store/process/network drill results; Registry is outside the authority protocol |

The shared ledger has one live authority per session and separate ownership DBs
and signing policies. Its single commit lock orders consume/cancel against claim
observations. The issuer certificate binds ordered IDs, the observed issuer epoch,
typed lifecycle records, involved session configurations/heads and exact block
references; the client enforces its required minimum epoch. A lost target response recovers both its original command result and
the source claim terminal. Reopen preserves Consumed/Retired/Cancelled/Reclaimed
outcomes and total assets; both transfer applications replay to their exact roots.
This uses the existing network suite and transfer executor, without a new lane.
Local implementation/evidence is complete for these declared #15 boundaries;
the implementation is published in PR #23. Application deployment and #20 latency measurements
are separately scoped, not implicit claims of these drills.

The same retained fencing evidence satisfies #20's explicit "cross-worker fencing
drill" acceptance clause. The complete core report below passes eight fencing and
nine network tests, including two independently active replicas, lost TLS commit
replies, partition/ownership rotation, stale-owner rejection and shared-ledger
claim reconciliation. These authority, transfer, session/store and journal paths are included in the
current full core report and its exact source inventory. This is reuse of the existing correctness drill, not
a new latency or physical multi-host claim. A duplicate fencing measurement gate
is therefore not an unfinished requirement.

## Requirement mapping — offline archives and CI

This maps #13 and #21 to their original acceptance clauses. It does not add
a physical-second-host, hosted-CI or all-optional-profiles requirement.

| #13 acceptance | Implementation and retained evidence |
| --- | --- |
| Original historical verifier reproduces the same root/result | Original-program sandbox replay in `session-bundle/tests/support/sandbox.rs`; the complete game build acceptance below compares the archived originals with two independently rebuilt offline inventories and exact range receipts after source deletion |
| Code, rules, authority, network, input, source dependency and snapshot tampering is detected | `changed_rules_authority_input_checkpoint_and_replayer_have_separate_failures`, inventory anchor mutation cases, and `missing_causal_edges_sources_and_forged_terminal_indexes_reject`; snapshot validity is checked by application replay, separately from structural/finality audit |
| Missing artifacts, unrecognized anchors and missing dependencies are distinguishable | Typed `ErrorCode` reports and actual inventory/export/replay CLI cases; no serialized verification flag is accepted as evidence |
| Composition retains each security domain and causal edge | Per-session trust anchors and finality reports, required handoff/source/terminal edges, and bidirectional successor catalog checks; no combined consensus claim |
| Offline replay on a clean machine with machine-readable reports | Fresh offline, unprivileged container reproduction and archived-game replay described below; original source databases and producing container are absent |

The current manifest also binds schema/profile/consensus/program/runtime and
resource limits. The typed file-package golden corpus and cross-session/successor
vectors cover canonical commitments. Explicit admitted actions, timeout/control
events and entropy policy remain replayable; the application export contract
requires excluding credentials and unnecessary private data. Complete build kits and
parameter-only measurement capsules are deliberately reported with different
provenance. No new program is substituted for a pinned historical interpreter.

| #21 acceptance | Implementation and retained evidence |
| --- | --- |
| Clean core runner without private paths | The retained `clean-core-v3` result below starts with empty Cargo/target directories, no host bind mounts and no network; current local core results are separately bound to their own source inventories |
| External checks are explicit skips or selected-release failures | One `validation.py` selector records all seven lane states; actual missing-browser and failed-subprocess probes fail selected scope while recording unexecuted lanes as skips |
| Reports bind exact commits, logs and artifacts | HEAD plus dirty-tree file inventory, before/after source identity, compiler/toolchain logs, nested artifact identities and independent post-run digest checks |
| Formatting, compilation, tests and integrations have explicit lanes | Core runs fmt, one all-target Clippy compilation, one fixture-enabled workspace test pass, Python boundaries and the existing transfer CLI; compiler, devnet, games, browser and archive integrations remain explicit lanes |
| Async/module/size descriptions match code | Canonical architecture and concurrency documentation distinguish synchronous execution/WAL, Rayon VM work, bounded Tokio transport admission and process isolation; stale fixed file-size claims are removed |

The original clean-container and complete-build reports identify older frozen
source snapshots; later code changes are covered by their separately recorded
current core/targeted runs. Configured Linux/macOS hosted CI has not been executed
here. A core or browser pass is not a `ckb`, `games` or `full` release-profile
pass. This mapping records the locally exercised #13/#21 contracts; publication
remains pending.

## Current-only cleanup

The current requested scope removes backward compatibility, historical schema import/upgrade paths,
and repeated gates. These changes supersede compatibility clauses in the original issue snapshot.

Supported storage is MREC 13, block 6, RocksDB schema 15 and journal 2. Bundle format is v2.
Authority RPC Packet is v2 only; v1 is rejected. The protobuf service name
is independently versioned. `SessionAuthority::open` takes an explicit shared
`CanonicalLedger`; the former path-based constructor has no compatibility wrapper.
Native workers accept only `MWK2`. One `Block` message carries the effect context
and explicit lifecycle actions, replacing separate context/lifecycle wire branches
and repeated parent/child dispatch. `Frame` remains the current basic execution
API and `Restore` checks snapshots; neither is an old-record fallback. The existing
packet-boundary test rejects the old envelope, with no additional test or lane.
Older stores/records are rejected; there is no automatic or marker-only upgrade.
Application-authorized state conversion into a new successor remains supported.
Wallet address normalization accepts only current CKB2021 Bech32m payloads.
Testnet rehearsal uses explicit per-role environment variables; old funding aliases
are removed. Storage backends must implement pagination, receipt and checkpoint
queries explicitly; production interfaces no longer supply full-history scan fallbacks.
The unbounded `load_chain` API is removed entirely. VM spawn/IPC constructors now
require a real scheduler runtime; seven placeholder implementations and fourteen
placeholder/trivial dispatch tests are removed. Three old carrier-evidence CLI
argument aliases and a duplicate old-schema rejection test are removed as well.
The DA CLI accepts provider-neutral quorum certificates. The old single-provider
receipt command, argument, data model, template and four dedicated tests are
removed. Certificate import recomputes signatures, blob bindings and epoch
validity instead of trusting serialized verification flags. CLI evidence schemas
no longer fill missing required fields with backward-compatible defaults.
The unused permissive serialization validator and its sixteen tests are removed;
typed decoding still checks the exact current envelope format/schema and payload.
Artifact export plans require an explicit dependencies list, including an empty
list; the old omitted-field import shape is no longer accepted.
The unused raw VM ABI module, multi-version negotiator, constant-only serializer
wrapper and their duplicate tests are removed. VM object ABI remains Molecule
`0x8001`, exposed as `VM_ABI_VERSION`; `VmSerializable` only provides its real
encoding/decoding methods. Execution receipts retain cycles and exit code;
unused gas/log placeholders are removed. Versioned payload codecs are required
at compile time; no-codec trait implementations and unused serialization macros
and the benchmark that depended on a no-codec implementation are removed.
The real Molecule verifier test also covers absent cell data and
malformed bytes previously repeated in a separate integration suite.
The single-variant `VmAbiFormat`, verifier/syscall ABI selectors and invalid
schema-zero envelope `Default` are removed. Syscalls call current Molecule
encoders directly. Artifact trailer validation still rejects non-current versions
and nonzero flags/reserved bytes; appending a trailer also rejects an invalid
existing declaration. Ten more scalar/duplicate tests were removed or consolidated
into actual byte roundtrip and artifact-boundary checks; no parallel test gate
was added.
Cellbase payloads now come only from the first output data. The witness-payload
fallback and its special txid/wtxid hashing branch are removed. Both transaction
IDs use one body encoder; only wtxid includes witnesses. The four net removed
hashing tests asserted constants, array lengths or repeated/mirrored computations;
existing coverage now checks real witness/body mutation behavior for ordinary
transactions and cellbase shapes. Six more constructor/constant-only VM and cache
tests are removed; execution-cycle, memory-limit and cache-behavior tests remain.
Old witness-payload identities are not preserved.
Build/toolchain roots now require the current typed file package. The opaque-root
compatibility branch, conditional prefix collector and duplicate example-level
package decoder are removed. Existing inventory/package tests cover rejection of
recognized opaque roots without adding a parallel test suite. Synthetic test
packages remain explicitly incomplete build inputs.
Eight more constructor/duplicate tests are removed or consolidated. Secure-envelope
roundtrips now cover empty, ordinary and larger payloads in one test; a mutation
matrix checks payload, hash and declared length at both verification and decoding
boundaries. Existing eviction and clear tests retain cache-membership coverage.

The duplicate length-wrapped batch format and its encode/decode helpers are now
removed. Sequences use the existing streaming format only. Redundant metadata
peek/boolean-validation helpers, the encode-again size estimator and unused result
alias are removed without compatibility shims. Typed single-value helpers remain;
metadata comes from `VersionedEnvelope`, whose `parse()` checks the exact current
type/schema. The example uses those actual interfaces and drops its mirrored
performance comparison. Nineteen unit tests were removed or consolidated into
full sequence equality, malformed/truncated input rejection and exact byte/counter
checks. Two no-op doctests are replaced by one real typed roundtrip. Two of the
removed unit tests only read the verifier's configured semantics field; actual
strict/default VM execution coverage remains.

`scripts/myelin_production_gate.sh` runs one workspace Clippy pass and one workspace test pass,
including VM IPC and command-example process fixtures, then transfer CLI and Python boundary checks.
External CellScript, CKB devnet, Teeworlds and offline build/container checks are explicit opt-ins.
Repeated crate runs, duplicated CLI fixture checks, text identity scans and past phase manifests
have been removed. The CellScript shell lane also delegates duplicate source/tag
and compiler checks to the adapter, and Teeworlds report generation uses the same
evidence directory as acceptance. See [production validation](../operations/production-gate.md).

The launcher now delegates to one Python implementation. Explicit `core`, `ckb`,
`games` and `full` profiles replace the old `RUN_*` switches, which are rejected.
Repeated `--lane` selections can run an integration without rerunning core.
Missing selected tools fail; unselected lanes are explicit skips. Reports bind
the actual source inventory, logs and generated artifacts, including nested
reports. There is no separate `cargo check` after all-target Clippy. The
transfer CLI uses the example already produced by the workspace test pass;
its redundant separate build with a different feature set is removed. The
Linux/macOS core workflow uses the same entry point and pins action commits and
the workspace Rust toolchain; Python 3.11 is installed explicitly. Obsolete
nine-stage diagrams and implicit devnet/Teeworlds descriptions are removed.

The P1 table above tracks implemented behavior and remaining scope, not a claim that every P1 is closed.

## Latest validation — 2026-11-07

The current MWK2 implementation and workload recorder passed the complete core
profile at /home/arthur/.cache/myelin-p1-current-worker-core-v1/report.json:
report SHA-256 e8542aade9d0d6a310e6a3611f9bf0927bf80ef8400310b9b7a0fffbaae110c2,
source tree 901c23c3bbcb22b84fb23d2ff8fe272023f9349d793887d9b946a5cf42692fa6.
All 794 Rust tests across 72 groups, eleven Python boundary checks, formatting,
strict all-target/all-feature Clippy and the existing transfer CLI passed.
The before/after source identities match; all 132 retained artifacts were
independently checked. There is no old worker operation/decoder or added lane.

The browser lane was then run separately on that exact source. Its report at
/home/arthur/.cache/myelin-p1-current-browser-v1/report.json has SHA-256
15639671b9a8012050ffd10483ea9e7b4559209c1670a7a9b8de7b7862ff943e.
Google Chrome 157.0.8103.38 and wasm-bindgen 0.2.118 passed all ten vectors and
64 exact native/WASM comparisons; ten altered inputs and ten altered snapshots
were rejected. All 18 artifacts and the unchanged source identity were checked.
This supersedes the older browser snapshot for current example-rule conformance
without rerunning core.

The earlier storage-growth campaign at
/home/arthur/.cache/myelin-p1-storage-growth-v1/report.json passed 30 cases:
five kinds, three scales, both layouts and Instant production. Its SHA-256 is
e9afe7d70e83117cfdb136550f5a87de42adc183d400ef3a640115d26a6effd2;
source tree e86ed3281248ac61a92fb500409a52c0873b14c3cf2ed057b4c6b224fadb3fba.
It records 37,354 inputs, 46,634 host command receipts, 10,026 blocks and
9,985,280 strict VM cycles. All 22,523 artifacts were checked, and block/receipt
counts, manifest dependency references and physical file-class totals were
independently reconciled. The workload guide retains the exact storage tables,
measurement boundaries and reproduction command. This campaign added no
production API, codec, test suite or gate.

The subsequent acceptance audit maps all original clauses and corrects stale
documentation about completed archive, continuation and downstream integration
work. Those documentation edits are not retroactively included in the source
hashes above. The implementation, tests, tools and fixtures remain the tested
bytes; a content inventory comparison records that distinction. Older complete
build/container and performance reports remain bound to their own snapshots.
None of these selected runs is a current full/CKB/game release-profile pass.

## Previous validation — 2026-11-06

The preceding complete Rust core report is `/home/arthur/.cache/myelin-p1-reward-publication-core-v1/report.json`, SHA-256
`ccd71303251985b5f9feb2e5bbb9dda74573dfab7275051d5288656e5d50bc7a`, source
`97305c581a96ac642137f460bd66db8c37ea9eba79bd6edad422d483eda9acd9`.
All 794 Rust tests across 72 groups, strict all-target/all-feature Clippy,
eleven Python checks and the existing transfer CLI acceptance (44 calls across
exact-target and policy-target scenarios) passed.
All 132 artifacts and the unchanged source inventory were independently rehashed.
The examples retain their 21 real process-exit checks in the existing suite.
This includes the actual shared-ledger remote-claim lifecycle flow using the
existing TLS server fixture and scarce-transfer executor. The existing archive
publication test now also verifies that insufficient step/byte grants pause
before reading, an exact sufficient grant completes, and source mutation while
paused still prevents publication. No additional test or gate was needed for
that fix.

The same source/executable passed 27 reward cases and eight cross-class workload
regressions. Configurable arrival/batch/window parameters subsequently passed
the existing eleven Python checks and two large reward cases at 100 rewards/s/pair.
Those cases retain identical Rust source and executable; later documentation
updates do not change the measured source inventories. Reports, hashes, fault
scope and provisional capacity settings are recorded in
[workload measurements](../operations/workload-measurements.md).
The earlier reward run remains failed; its publication stall is not hidden by the
successful replacement campaign.

Subsequently, the existing measurement executable added controlled TCP evidence
providers, per-message stage timing and live descendant RSS accounting. Its source
`95849e92c4f6b79806043d9d66a61fef7e48cd53b7cb53d6ff83a85e1b5b2e0e` passed nine
reward/provider cases, eight host/CellTx regressions, the affected package's
28 Rust tests, strict Clippy and eleven Python checks. No production-library code,
backward reader or gate was added. The workload guide retains exact reports/hashes,
separates canonical credit from asynchronous evidence publication, and defines a
measured burst-drain qualification target. The complete core report above belongs
to its stated earlier snapshot; these later scoped checks are not relabeled as a
new complete core run.

The subsequent filesystem-capacity campaign passed four cases on source
`1765b649524dc7b815328bb573af8106c3e4057d6a206317227edce2088334d9`:
two normal cases and real RocksDB `ENOSPC` in both shared/separate layouts.
All 1,088 rewards / 3,264 commands recovered exactly after remount and original
image removal. Three independent services completed 384 rewards during the
isolated store fault; shared neighbors rejected writes without partial canonical
changes. The same source passed nine reward/provider cases, eight cross-class
cases, 28 package tests, strict Clippy and eleven Python checks. The workload guide
records exact reports, hashes, resource/latency observations and the two preserved
failed attempts. The redundant zero-free-block check was removed: the actual
RocksDB write error establishes the fault even if closing the DB releases a block.
Fault selection reuses the existing entrypoint; no backward reader, test suite or
validation lane was added.

The existing downstream Chess online flow then replaced a repeated lost-create
response with a real SQLite writer lock. Native room creation commits before the
index returns `SQLITE_BUSY`/HTTP 503; an independent four-move game progresses and
replays while the lock is held. After release and process restart, exact original
room and independent receipts/heads survive, and canonical admission repairs the
projection. The entire online flow passed using the exact independently released
native binary; only its test script differs from that release's source. Raw
exports, requests, timing and identity evidence are recorded in the workload guide.
No game runtime change, backward fallback or new gate was needed.

The preceding claim-reconciliation core report is
`/home/arthur/.cache/myelin-p1-claim-reconciliation-core-v2/report.json`, SHA-256
`de52b78a5bcc04add5efca7dff4cad22db745e28069918e5148f5775a3e32314`, source
`b91baad8d51237f5163f6a4b36048145d363fdd1024ee065d98d98582f21d0eb`.

The journal now skips WAL writes for authenticated operations that leave its
entire state unchanged. Locked updates reuse signature verification only for
complete recovery evidence identical to already verified in-memory evidence;
structural bindings remain checked, and opening disk state always verifies again.
No persisted verification flag or old-format decoder was added. The existing
fenced test mutates a stored signature and recomputes the envelope checksum:
reopening still rejects it. The existing uncertain-write test now retains 12,000
originals and confirms recovery cannot overwrite a durable outcome or exceed a
smaller byte budget. Defaults remain 4,096 entries / 16 MiB; explicit applications
may request up to 16,384 entries / 32 MiB, with independent count and byte bounds.

### Two downstream game integrations

Both isolated worktrees use `CommandExecutor`, `CommandJournal`,
`RegisteredFinalityVerifier`, `SessionAuthority` and `recover_local_commands`.
Rules remain in each game. Exact original IDs bind the full action, construction
head, profile and gateway-attested account; this is not a personal wallet
signature. The browser persists the original request, resolves Unknown before
new work, and returns the original receipt with a current view after retry.

New-game commands durably close their source with a deterministic target ID.
Target databases are synced and renamed before the active pointer changes;
restart finishes the same creation. A namespace lock excludes another local
owner. Late retries never rewind active history. Missing metadata and old
command/storage/import shapes reject. Each native integration exercises six
actual creation-process exits, with no separate acceptance lane.

Chess worktree: `/home/arthur/.codex/worktrees/p1-games/myelin-chess`, branch
`codex/p1-command-recovery`, baseline `e984005`. Report: `/home/arthur/.cache/myelin-p1-chess-claims-v1/report.json`,
SHA-256 `9f7a848a4cc1f568ed66b37cd8d1593ed699b0b7b45e5105754d44bb2ed94110`; source
`99987baacb21ae7f4d1b3d392de56df5cc583a91fa67b912fe239d0a665f3c91`; original executable
`8fbdc120c776bb1165fe03b26216bed515bd5ba9b90c3eef94278e698fcf7993`. All nine existing steps passed:
format, strict native Clippy, four native tests, native/web builds, nine frontend
tests, two host tests, controller and real-native HTTP checks. All eleven retained
artifacts and both unchanged source inventories were independently verified.

Explicit `create_room` / `join_room` commands admit both seats before moves.
SQLite holds routing and projections; native replay enforces actor authorization.
Lost replies before projection, service restart, competing stale seat requests,
changed payload/actor and cross-account queries are exercised. Original-executable
replay verifies both admissions, two business rejections and four legal moves.
The old room importer, implicit first-move admission, empty new/join requests and
color-only create API are removed. The duplicate room mock suite was consolidated
into the existing real-native flow. Controller checks use DOM/transport doubles.

Miner worktree: `/home/arthur/.codex/worktrees/p1-games/myelin-neon-miner`, branch
`codex/p1-command-recovery`, baseline `a73dfb40fcc64558ed062f7652f867208ac9b89e`.
Report: `/home/arthur/.cache/myelin-p1-miner-claims-v1/report.json`, SHA-256
`f12d91c3ad7352a1b01fbdb4642df5b92350d322cd1a1375e3a0fcf6862bac49`; source
`a228a11734010afeed2637404a1a05ef259ec7f7c5f18ed6c23b9836b91ea73f`; original executable
`40494365cb79e7325d543a08e44fbfd673025ddcb3cf3495b043219cfbe441ae`. All ten existing steps passed:
format, strict native Clippy, eight native tests, native/web builds, thirteen
frontend tests, two host tests, loader, engine conformance and real-native HTTP.
All 16 artifacts and unchanged source inventories were independently rehashed.

Conformance compares 64 held/random input tapes and two full 60-sector campaigns
with Rust. The signed HTTP campaign executes 795 commands and 87
purchases through all 60 sectors, comparing each game state to the browser engine.
A further committed business rejection gives 796 replayed frames. Tests cover
duplicate purchases, lost command/status replies, SIGKILL/restart, client reload,
stale/cross-account requests, historical original receipts and lost new-game
responses. After the producing databases are deleted, the retained original
executable verifies the export offline. The actual controller runs with DOM/native
Canvas and the same authenticated host; it covers purchasing, pause/blur, artwork
retry, replay and pending-command recovery. Controller checks run before the long
campaign. New-game fault injection targets command-status recovery, which is the
actual path taken by a lost creation reply.

One 1–60-sample tape is one command; only the unchanged 30 Hz integer game engine
advances physics. Purchases and control commands do not add simulation ticks.
The profile pins RNG policy and the complete executable. Results retain a compact
game digest and business fields, with the full current view returned separately.
The explicit journal budget is 10,001 commands including terminal creation and
32 MiB encoded bytes. Export limits remain independent. Rust game rules differ
from baseline only by rustfmt; the browser engine is byte-identical.

The duplicate `live-miner` campaign and unrelated multi-game shell suite are
deleted; applicable shell assertions run in the actual controller. A repeated
native conformance invocation, unrelated room quota branch and external-checkout
test-library fallback are removed. There is no extra production gate. The existing
Chromium layout fixture passed six display sizes, inverse pointer mapping, bounded
backing resolution, portrait pause and in-scene controls; its evidence is retained
at `/home/arthur/.cache/myelin-p1-miner-layout-v1/evidence.json`. It is layout-fixture
evidence, not physical-device or native command-root evidence.

Both original downstream checkouts remain untouched. Miner's eleven pre-existing
UI files and original HEAD were rehashed against the preserved baseline; those
UI changes were copied into the isolated worktree before integration. Both game
exports report ordinary PoA/application replay under their declared authority and
`canonical_fencing: not-verified`; they do not replace the portable bundle protocol
or establish CKB settlement. Their Rust rule/session sources remain unchanged
from these frozen debug-program runs. Later release recipe/checksum/documentation
changes have their own source inventories and container evidence below; those
release binaries do not replace the original executables in these earlier reports.
No commit, push or production deployment was performed.

### Independent source and container releases — #19

Both game repositories now pin `myelin-source.tar` through
`deploy/myelin-source.sha256`. The framework archive is 22,753,280 bytes, SHA-256
`7533432e85f2a2554f8158f4d15caf1a81d656bf43cdabbbd4a3ae4165331a5c`, source tree
`56d8c0cf6e72cc33e01b41158fa30e6aeb349eaeeee50cf519363d3231464c2a`.
It identifies the exact dirty candidate at the recorded baseline, not a published
Git release. The existing archive tool now exports this source package, checks
every member against the frozen inventory and rejects source changes. The eleven
existing Python boundary checks pass; no additional validation lane was added.

Each independent release tar includes one complete game tree and `framework/`,
with the exact source tar and its inventory. The Dockerfile verifies the game's
checksum before extracting/compiling the framework. Rust 1.92.0 and Node 22.23.1
base image digests are pinned. Cargo/npm use lockfiles; compilation defaults to
four jobs. A mutated framework archive is rejected by the actual Dockerfile
before extraction or compilation. The build uses only these staged inputs, with
no original source checkout or host target directory mounted.

Retained report: `/home/arthur/.cache/myelin-p1-downstream-release-v5/report.json`,
SHA-256 `d53649707506d97047d9019cd37b10948a1b5186218e4e580e0f65edebd03037`.
All nine steps pass (the mutation probe's expected nonzero exit is recorded),
and all 278 artifact identities were independently rehashed. The report retains
source inventories, commands, image inspections, original binaries, HTTP records,
exports, offline results, both complete source-release tars and both saved images.

| Game | Release source tree SHA-256 | Source-release tar SHA-256 | Built Linux amd64 image ID |
| --- | --- | --- | --- |
| Chess | `901a09bbb5b9c9c7be930b0e805678da82d2a6322706a795c95b14edeaac00dc` | `d1f3a07d82d4d25e9ddbf611d8fa4df949084f8f0d72528e781083fb22aac357` | `ed4704d6373644d9779f6cbb211d8d08917080fbe3d32898cf205ab3c9733651` |
| Neon Miner | `8ef2ecd397631f8eb60df37bf68b7cb03c136322dbaeb138d621c94cf83d778b` | `4af249edd739a5e010c89c45342d8268a08da78681a03095ee5d105cf70e9a07` | `17e720400981921c47ab0386ba3dafb62d23e8ffdf517519f79a71e5da377429` |

The release native binaries have SHA-256
`580e6a6979038de0cd6efc38112903f47763baeda9dcd974cb2c82fcc2dc1b67`
(Chess) and `fcc497203ccf2546b73aba9bb04fdd0851f9591e0d067cc00b712cc4d9430bd7`
(Miner). Both actual images run as `node` with a read-only root, dropped
capabilities, no-new-privileges, temporary `/tmp` and their own persistent volume.
Only the web port is published. The checks submit two commands, compare a late
duplicate's original receipt, keep another account unchanged, SIGKILL and recreate
the whole container, and recover the exact head/game and receipt. Chess also
retains explicit two-player room admissions across this restart. After the data
volume is removed, an offline network-disabled container uses that same image's
original executable to replay its exported two frames to the exact state root.
These are release packaging/runtime checks, not another full Miner campaign,
browser rendering pass, latency benchmark or CKB settlement proof.

The initial uncached Chess build is retained in the failed `release-v3` run;
the passing run reuses that same immutable image/build cache. Earlier reports
remain failed: v1 was interrupted to correct anonymous readiness checking after
cookie-preserving restart; v2 could not reach the configured host-loopback proxy;
v3 expected 200 instead of the room API's actual 201; v4 omitted the verifier's
required writable temporary replay directory. The passing run fixes these harness
conditions, uses host networking only for build-time proxy access, and retains
isolated runtime networking. No application rules were changed to satisfy them.

This establishes independently buildable/runnable local release artifacts for the
two adopters. It does not claim bit-identical image reproduction, a new public
upstream release or a production rollout. See
[downstream release procedure](../operations/downstream-release.md).

### Reference recovery objectives — #18

[Workload measurements](../operations/workload-measurements.md#reference-deployment-recovery-objectives)
now define zero lost acknowledged outcomes and a 60 s process-recovery objective
for the measured local 4,096-block reference envelope with surviving authoritative
volumes. Locally retained archive verification has a separate 60 s objective
relative to its independently pinned archive head. The method records full outage
and nested recovery phases, exact acknowledged/unknown originals and archive lag.
These operational targets are informed by the retained 2.45 s DB recovery and
2.19 s archive-replay observations; they are not a demonstrated end-to-end SLA.
Unavailable authority or required evidence keeps writes pending, without an
automatic disk-loss takeover promise. Deployment qualification uses existing
drills; it does not add a correctness gate.

### Previous validation — multi-epoch dependencies

The multi-epoch test replacement and its integration into the existing report
passed the core profile at
`/home/arthur/.cache/myelin-p1-multi-epoch-core-v1/report.json`, SHA-256
`c370759efe137c1f26110626caf6f6d80aa200a2e0aec1797694d07b4494e63f`.
Its source tree SHA-256 is
`66c77bf43e04fceae82496a04352495065021ee3dcebd5f785580e717973e05a`.
All 793 Rust tests across 72 groups passed with zero failures/ignored tests,
along with strict workspace Clippy, eleven Python checks and the transfer CLI
acceptance. The default three-rotation report is retained in the same output.
Source identities match and all 132 artifacts were independently rechecked.
The test count is unchanged; no new gate was added. Only this delivery record
was updated after the run. External lanes remain explicit skips.

### Previous validation — serialization and long-history measurements

The serialization cleanup and archive-depth correction passed the earlier core
profile at
`/home/arthur/.cache/myelin-p1-current-only-workload-core-v2/report.json`, SHA-256
`766d81d5c7316dcbeff8218a7fa79a5d60b4a4209946824aecb4ab4fcb763fe7`.
Its source tree SHA-256 is
`9db03a1aa1bb543899f89ab54109786980d8af8ca841ea30cc6d9678cc840099`.
The run passed 793 Rust tests across 72 groups, zero failures/ignored tests,
strict all-target/all-feature workspace Clippy, eleven Python checks and 44
transfer CLI process invocations. Source identities match and all 93 retained
artifacts were rechecked. The rewritten serialization example also ran directly
from the executable built by that workspace pass. External lanes remain explicit
skips; no new acceptance gate was added.

The earlier host measurement matrix completed all 72 normal cases but failed
its 4,096-block export at the dependency-depth limit. Its report remains failed:
`/home/arthur/.cache/myelin-p1-workload-matrix-v1/report.json`, SHA-256
`c9bdad032cb1fcddf3606b7307942ba2bdd0e8f165bcb15d553bbe6720c00b77`, source
`1f2ac07f197ef267859841bfcbe23d854bb2beebaa648303d877002292e769c6`.
All 20,592 retained artifact identities were rechecked. The original executable
is preserved separately as `myelin-p1-workload-matrix-v1-executable` in the same
cache parent, SHA-256
`4e566502d4ad61cf2d2aa514be2c812b9319654e3f75a558368b800156997c14`.

Dependency depth includes consecutive history blocks. The runner now explicitly
allows the selected command count plus eight metadata nodes. The generic default
remains 1,024; the absolute ceiling for this iterative traversal now matches the
262,144-node ceiling instead of imposing an additional 4,096-depth ceiling that
cannot fit 4,096 blocks plus metadata. Cycles, missing dependencies and supplied
node/edge/depth/byte limits still reject. There is no new format or old-data import.

The focused corrected run passed all eight selected cases: two small tick
baselines, 4,096-block archive recovery, exit after first commit, and slow/Unknown
execution in shared and separate-process layouts. Its report is
`/home/arthur/.cache/myelin-p1-workload-drills-v2/report.json`, SHA-256
`9d8e78e0071d0f06acb4612daaa506b1e88f4548aa1e723930e62601819b143e`.
It uses the same source snapshot as the latest core run; all 6,947 artifacts and
the before/after source identity were rechecked. The executable is also retained
as `myelin-p1-workload-drills-v2-executable`, SHA-256
`5ff7b5c1f837e751c35a86944d9a6a40443be4f0f2cbcbe741493319996922d4`.
The normal 72-case matrix was not rerun on this newer snapshot.

Long-history recovery and full replay took 2.45 s; export took 5.21 s; fresh-process
archive audit/replay after source-database removal took 2.19 s. The archive has
4,104 blobs, and sampled export RSS peaked at 74.2 MiB. Process-exit recovery found
exactly one committed command and retained all 32 originals. Each layout recovered
40 injected Unknown outcomes over 4,096 commands without changing the final
head/state. These are single-host observations, with no OS-cache purge or
production RPO/RTO claim. The [measurement guide](../operations/workload-measurements.md)
contains the three-scale/four-trigger values, phase costs and cautious settings
for each game mode. Large offered loads showed seconds of backlog; a passing
correctness/recovery case is not a realtime-capacity claim.

The multi-epoch test replacement below follows this snapshot. Remote claim
terminal reconciliation, deployment RPO/RTO targets, two independent downstream
games and the remaining #20 workload/fault classes still require work.

### Multi-epoch dependency measurement

The single-successor refund test is replaced by one parameterized test of real
source locks across independently signed authority epochs. It covers both
consume/retire and cancel/refund, exact replay of every session after original
database deletion, and rejection of a missing intermediate seal dependency.
The default core case uses three rotations; the existing core lane now retains
its report. No extra test or gate is added.

Release measurements passed at 2, 8 and 32 authority rotations, the last reaching
the current owner-lineage limit. The report is
`/home/arthur/.cache/myelin-p1-multi-epoch-v3/report.json`, SHA-256
`df192ebe5edcb5ce29222cd8fc8a8450f8c724a476961d179d8ab524bbd04ba6`, source
`98eb01069bc8c065061d27e33fe32c865288471123a0fcb0ce535afe272b83de`.
All 296 retained artifacts and unchanged source identities were rechecked.
The 32-rotation case covers 34 sessions and 37 blocks: archive audit took 40.10 ms
and subsequent restore/full replay 106.52 ms, with an OS-reported process peak RSS
of 59.01 MiB. The source/target balances remain 75/25 with no remaining locks.
This is same-process trusted host replay after source-database deletion, with
synthetic build packages and no page-cache purge. It is separate from the retained
clean-container/complete-build evidence and makes no deployment RPO/RTO claim.
The [measurement guide](../operations/workload-measurements.md#dependencies-across-authority-epochs)
contains the full table, raw accounting boundaries and reproduction command.

### Previous validation — archive collection

The collection change passed the existing core profile at
`/home/arthur/.cache/myelin-p1-collection-validation-v1/report.json`, SHA-256
`b37b97a244dfaeb2015629e03e6840092b0eb5a2de0ed5d30e6774af710918bf`, with source
tree SHA-256 `a3336c07ae8a813a827065b94c3dcad23d4c3ce76d9390d4cd22575f7a8d5104`.
This local Linux run passed 813 Rust tests across 72 groups, zero failures/ignored
tests, strict all-target/all-feature workspace Clippy, eleven Python tests and
44 transfer CLI invocations in two modes. All 93 retained artifact identities
were rechecked. External lanes remain explicit skips.

Final review then removed repeated keeper-index construction, repeated catalog
budget counting and duplicate collection-plan validation. Coverage/index work
now participates in cooperative budgets. Reader locks cover both root and payload
directories, so another mount of the payload cannot evade exclusion. The final
51 bundle tests across seven groups, strict workspace Clippy, formatting and
whitespace checks passed at
`/home/arthur/.cache/myelin-p1-collection-final-v2/report.json`, SHA-256
`6ae968a539a1187aa8b6bf63d154bff0b28df29bd01eef4f321cf2d16180a3d5`, with source
tree SHA-256 `fc2f1b8def9e5d6a247b790d5a5ceac41b5bb2d9e7ea02dabc27afe381a830c9`.
Its six retained artifact identities were rechecked. The final edits have this
targeted validation; the earlier full-core report identifies its own frozen
snapshot. Only this delivery record changed after final validation.

The final `archive-collection/storage-and-readers.json` SHA-256 is
`a0997ad4bbca40cc2bdc49489001450f38ad17c4bf734fc25860b76626e47bd9`.
It records 17 removed blobs and 331,204,806 unlinked file bytes, with live-database
and checkpoint inventories preserved. `archive-collection/process-restart.json`
has SHA-256 `d0608c721bd726d36d7a482066d99dc442a4c2c04bb6a2eedf96b7aace1bae2d`:
one blob was removed before SIGKILL and the restarted CLI removed the remaining
14. A subsequent retry removed none. These are controlled development fixtures;
neither report claims physical disk savings or independent-machine acceptance.

The archive collector uses the existing current-format manifests, catalog audit,
work budgets, CLI and core profile. It adds no storage decoder, compatibility
branch or acceptance lane. Two end-to-end cases cover actual file deletion and
recovery rather than mirrored constructor/constant tests. Database/checkpoint
pins, live and parked readers, protected roots, missing historical dependencies,
shorter history, unexpected files and symlinks reject. The subprocess case kills
the collector after an unlink, checks that a tampered keeper blocks restart,
then completes via the real CLI and verifies idempotent retry. All database keys
remain unchanged; retained history and transfer state still recover and replay.

Collection is Linux-only and scoped to the explicitly supplied databases and
managed immutable archive directories. It retains the canonical `gc.pending`
manifest and empty payload directory for restart. Shared locks coordinate library
readers; they are not filesystem write protection. Logical unlinked file bytes
are not physical space measurements. This does not complete the remaining #18
lifetime storage, multi-epoch dependency, RPO or RTO acceptance requirements.

### Previous validation — isolated migration and successor conflicts

The migration/archive change passed the existing core profile at
`/home/arthur/.cache/myelin-p1-migration-validation-v1/report.json`.
Its SHA-256 is `f385eacdacd320916788b558d80c6937bb727ef41271dcdd082546f42caac40b`
and its source tree SHA-256 is
`800ed2fb25a3b56f8dafb144cfa903a7b9d313ab0c24390bd62733e4baa9a20c`.
This local Linux run used the existing Cargo cache: 811 tests across 72 groups,
zero failures/ignored tests, strict all-target/all-feature workspace Clippy,
eleven Python boundaries and all 44 transfer CLI process calls passed. The 91
retained artifact identities were rechecked after completion. External lanes
remain explicit skips in this core report.

`BundleStore::read_artifact` now gives historical interpreters a bounded,
digest-checked read of declared artifact bytes. It rechecks every read after
catalog audit and confers no execution or finality claim. The old process-local
append migration archive test is replaced by one isolated bytecode test, keeping
the test count unchanged. Both original databases and original input artifacts
are removed before replay. The source conversion and target genesis restoration
have separate reports; the target's zero-block result cannot replace source
migration execution. The real replay CLI matches the isolated source result.

The retained `isolated-migration/valid/report.json` SHA-256 is
`885553d223a071d0fef31947324e5005489cc777353aee54ad6c47120dabc45c`.
The `isolated-migration/signed-invalid-output/report.json` SHA-256 is
`caec2e00f29470aaf803e96d841e6c3dafbf335e1ae592dd7476c5b01cb1ee8c`.
The latter has valid finality but fails actual archived-program execution.
Tampered/missing artifact reads, byte ceilings and substituted mandatory edges
also reject. These are development-key fixtures with synthetic build packages
and no custody/production-recognition claim; economic preservation is separately
covered by the signed reward-ledger and transfer tests.

After that core pass, the existing successor atomicity test gained direct cases
for all four predecessor bindings, an already occupied target and two competing
successors. Failed commits preserve source head/checkpoint/block/outbox state and
the pre-existing target. Exactly one competing successor is created. The expanded
test, strict workspace Clippy, formatting and whitespace checks passed at
`/home/arthur/.cache/myelin-p1-migration-atomicity-final/result.json` (SHA-256
`614548fde4613f964919cbb56ea0949e53173658e8c6d0f416e189de45077338`) with source
tree SHA-256 `d999c93edfe2787fa0846f886c7179a288d4facac1f63396b5a3403dc77b186b`.
Only this test file changed after the full core pass; final documentation follows
the targeted validation. No storage format, compatibility path or extra gate was
introduced.

The #14 local acceptance mapping is now:

| Requirement | Inspected implementation and executed evidence |
| --- | --- |
| Explicit support matrix and rejection of unapproved rules/codecs | `session-succession.md`, `SuccessorState` and `same_codec_program_and_policy_changes_require_explicit_successor_rules` |
| Complete target authorization, declarations/vectors and conversion receipt bindings | Signed ledger hook, reference transfer rightsholder approvals, `migration_root_receipt_inputs_and_resource_tampering_are_rejected` and fixed successor commitment corpus |
| New authority cannot rewrite old history | `rotated_authority_cannot_reinterpret_or_finalize_the_old_session` checks both proof domains, recovery and source-bound signatures |
| Wrong predecessors, occupied targets, sealed writes and competing successors reject | Expanded `successor_is_created_atomically_and_seals_the_predecessor` and the process-exit migration drill |
| New command namespace cannot duplicate awards or external obligations | Signed reward-ledger conversion preserves balances/nullifiers and resets only score; transfer succession checks old-signature/nonce rejection, inherited lock ownership and original outbox IDs |
| Incompatible conversion verifies offline and remains atomic across crashes | Isolated migration positive/forged-output archives, signed migration-input replay rejection, `migration_process_exit_preserves_atomic_source_seal_and_unique_target` |
| Successful and unsupported CLI paths | Both modes of the retained 44-call transfer report execute continuation and reject `unsupported-upgrade`, followed by unchanged-state replay |

The remote issue was re-read and remains open. Local implementation/evidence is
not publication or remote closure. #18 measurements, downstream adoption, remote claim reconciliation and the
remaining #20 workloads/faults remain part of the active P1 goal. CI and release
evidence must match the lanes actually selected and executed.

## Previous validation — portable gate cleanup and game archives

The final core profile passed in a fresh offline, unprivileged Linux container
with empty Cargo/target directories and no host bind mounts. One workspace pass
with VM IPC and all four process-fixture features passed 811 tests across 72
groups, with zero failures or ignored tests. The eight removed/consolidated
execution tests are already reflected in that total. One all-target/all-feature
workspace Clippy pass with warnings denied, formatting and diff checks passed.
Eleven Python boundary tests passed: seven build-kit and four validation-report
cases. The transfer CLI used the executable already built by the workspace tests;
both modes and all 44 process invocations passed without a second build.

The final report is
`/home/arthur/.cache/myelin-p1-validation-v1/clean-core-v3/evidence/report.json`,
SHA-256 `c946ab0ff5495306d12fe1e1e73b9d8a5c9ea45875b0a13ec1736b9ec7e95d41`.
Its 522-file source tree SHA-256 is
`c2a599b531e0216bdf262da4c1afbf3aa1afe2051d9ed26ba97ec7a62386b441`.
All 42 retained artifact identities were checked again after collection, and
source identities before/after execution match. The exact prepared toolchain
image is `7363710523de58c32db4a92ddd1b159dd169b68ca7553aa1f4046ea87267afa0`.
The adjacent input inventory binds source, baseline Git bundle, vendored
dependencies and driver bytes. The test container was removed successfully.
That acceptance record used the frozen snapshot; the newer migration/API work is
validated separately above.

The first `clean-core` attempt retained a failed report because outer OCI proc
masking prevented nested bubblewrap startup. The corrected outer policy permits
namespace/proc creation; the inner application sandbox and tests are unchanged.
`clean-core-v2` passed but still performed a redundant 178-second example rebuild.
The final `clean-core-v3` removes that step and proves the workspace-produced
example runs directly. All attempts retain their own reports and cleanup records.
The hosted workflow YAML was parsed locally, but Linux/macOS hosted execution is
still pending. Neither a core-only pass nor a configured workflow is a full release
profile pass; all six external lanes are explicitly skipped in the core report.

The archive API now requires current typed file packages for both build and
toolchain roots. An independently recognized opaque root fails format validation
before assembly creates its output. The conditional prefix collector and duplicate
example-level package decoder are gone. Existing package, archive, retention,
backup and isolated-runner tests pass with explicit synthetic current packages;
those fixtures do not claim complete build inputs.

All three native examples now export canonical history, the exact running
program/replayer, profile rules, complete build kit and explicit runtime libraries.
The same original executable implements the existing sandbox runner ABI. Trust
anchors originate in the controlled source process and supplied artifact plan,
not a downloaded manifest. Original-command authentication and shared application
rules run again during historical replay. Journal queues, ACKs and authority
ownership are not exported as recoverable operational state.

The complete build acceptance passed at
`/home/arthur/.cache/myelin-p1-games-acceptance-v2/acceptance.json`.
Its SHA-256 is
`16caabf7fffd6b0d444f2af72f0626835b8c8ef456d6c80a7266f4a270c73b46`.
The recognized build-kit manifest SHA-256 is
`f2484d8c075156bb10c744cb16dbde73a6152f6ae193c10f08db10cb273e0df4`;
the typed package root is
`7e24f7a77b30aef386f42069b0ff9fa25e10208f136a6182324d770c0fe7a009`.
The kit contains 518 source files, vendored dependencies, the fixed build script
and the toolchain OCI image. Two fresh offline, unprivileged containers rebuilt
all five executables from the restored kit. All fourteen output files, including
runtime libraries and tool versions, matched the separately built original
inventory exactly. The original historical verifier also audited and replayed
the reference source/target transfer history.

A separate fresh offline container created the three game histories, exported
their bundles and removed the fixture databases. After that container was
removed, isolated replay matched each original state root and complete range
receipt: inventory has 3 blocks, turn-based play 7, and fixed ticks 12. Program
and runtime hashes match the reproduced inventory. Both build containers and the
game source container were removed successfully.

The initial fixed-tick export paused at the default 8 GiB work budget during
package checking. Its failed attempt and cancelled obsolete rebuild remain in
`/home/arthur/.cache/myelin-p1-games-acceptance`; neither counts as acceptance.
The final example input requires an explicit finite work budget. This lane grants
100,000 steps, 16 GiB accounted I/O and 30 seconds per export; no default library
limit or pause behavior was weakened. Final accounted I/O was 7,079,621,674,
8,153,442,549 and 9,495,741,888 bytes respectively. These include conservative
record reservations and repeated validation; they are not physical-disk or
throughput measurements.

The tested kit is the exact frozen runtime source snapshot. It predates later
execution-test consolidation, validation orchestration/CI, the artifact-read API and documentation
changes; those are not attributed to the archived source bytes. The existing `bundle-build` lane
covers all five programs and three game archives, without a separate game gate.
Storage remains MREC 13, block 6, RocksDB 15, journal 2, bundle v2 and backup v1,
with no earlier-format upgrade/import path.

Chrome/WASM conformance now also passes through the new entry point at
`/home/arthur/.cache/myelin-p1-validation-v1/browser/report.json` (SHA-256
`2b7ccfe754e11a249d0c5036ba7c1139515b9d80c6a0827ee0569048bac06286`).
It records 64 exact comparisons across ten vectors, plus twenty rejected
modified-input/foreign-profile probes. Chrome is 157.0.8103.38 and wasm-bindgen
is 0.2.118. The source tree SHA-256 is
`0fb0f38d251205c7a9e82f7874deb27eecccb353db1eb40f627743e28cf9d85a`;
the report records all other lanes as unselected skips. It covers shared example
rules and checkpoint restoration, not browser authentication, journal or CKB
settlement. Separate CLI probes confirm a missing selected browser fails with a
retained report and old `RUN_*` flags are rejected before commands run.
External CellScript, CKB devnet, Teeworlds and the separate bundle-container/build
lanes were not rerun for the validation-entry-point change.

These are local clean-container and Linux-sandbox results with public development
keys. Physical multi-host deployment and official application recognition were not
exercised; they are not extra acceptance gates for #13/#21. CKB settlement and
adoption by two independent downstream games retain their separately declared scope.
At that earlier validation, archive GC and long-history measurements remained
open. The newer collection, multi-epoch and workload measurements are tracked above
with their own exact source and report identities. Deployment qualification
outside these reference workloads remains a separate operator responsibility.
