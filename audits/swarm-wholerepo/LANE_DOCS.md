# Myelin Swarm Audit — Docs + Cellscript fixtures + Templates (Lane: Docs)

> Verifier-only review. No fixes proposed. Scope: branch
> `arthur/session-l2-production-evidence-fixes` lane
> "Cellscript fixtures + docs + templates + top-level audit doc
> consistency". Cross-references the format/rigor of the two
> pre-existing swarm audit files (`MYELIN_SWARM_AUDIT_MEMPOOL_CONSENSUS.md`
> and `MYELIN_SWARM_AUDIT_STATE_DA.md`).

## Verdict

**Conditional PASS.** All four new CellScript fixtures parse, compile
under `--target-profile ckb`, and are wired into both
`scripts/myelin_ckb_devnet_smoke.sh` and
`scripts/myelin_public_testnet_rehearsal_prepare.sh`. The 6 JSON
templates in `docs/templates/public-testnet-rehearsal/` match the
CLI's documented field schema for the corresponding
`session <verb>-evidence` commands. The runbook's 526-line walk-
through is mostly runnable. Top-level docs no longer contradict the
code on the dimensions the branch changed.

**However, several real defects and inconsistencies fall outside the
"fixture + template + runbook" lane frame and would surface as soon
as the operator runs Phase 4-5 of the runbook against a real public
testnet, or any reviewer compares the new top-level Myelin_* docs
side-by-side:**

1. **The `da-anchor-final.cell` fixture is a CLI orphan.** The
   cellscript tests and the devnet smoke compile and deploy it, but
   the CLI's `session_carrier_submission` has no carrier payload
   kind `"myelin-session-da-anchor-final-v1"`. The fixture is
   exercised only by the cellscript `v0_18` test (in-memory) and by
   the devnet smoke (final-script path). There is no helper that
   builds a da-anchor-final submission report from a CLI command
   outside the smoke. This is a scope gap that the runbook also
   inherits when it asks the operator to "use `da-anchor-final.cell`
   for final-script" without documenting which CLI command submits
   it.
2. **`MYELIN_TEEWORLDS_REPRODUCIBILITY.md` claims `court_checks: 16`
   while `MYELIN_PRODUCTION_GATE.md` claims `court_checks: 22`.**
   The production gate number matches the actual
   `verify_teeworlds_court_bundle` implementation, which emits 22
   checks per bundle (chunk-payload-hash, molecule-transaction-hash,
   molecule-transaction-length, projection-possible,
   projection-profile, projection-source-txid,
   projection-raw-tx-hash, projection-wtx-hash, block-hash-recomputes,
   block-state-root-before-matches, block-state-root-after-matches,
   block-scheduler-commitment-matches, block-data-commitment-matches,
   evidence-block-hash-matches-canonical-block,
   challenge-payload-hash, committee-signature-hashes,
   committee-signer-ids, committee-certificate or
   tendermint-certificate, committee-quorum-weight or
   tendermint-quorum-power, court-verifiable-profile, vm-profile,
   ckb-spawn-ipc-not-required). The reproducibility doc is stale.
3. **The runbook documents `--verifier-role final-l1-script` and
   acceptance "readiness_evidence_mode is `live-ckb-carrier` or
   `final-l1-script`", but `scripts/myelin_public_testnet_rehearsal_live.sh`
   only implements `da-anchor` and `settlement` roles and ignores
   the `final-l1-script` selector when picking which `.cell`
   source to feed to `--verifier-source`. An operator who follows
   the runbook literally and sets
   `MYELIN_REHEARSAL_ROLES="da-anchor settlement"` while also
   passing `--verifier-role final-l1-script` will end up
   submitting the *carrier* verifier under the *final-script* role,
   which is exactly the wiring confusion the role distinction was
   meant to prevent.
4. **`state/README.md` still describes 1GB-append-only-files-with-mmap
   segment storage** (line 9) and references a `kv/` module and
   `writer.rs` that do not exist. The swarm audit already flagged
   this as a code/implementation drift; this branch did not fix
   it.
5. **`docs/MYELIN_ARCHITECTURE.md` lines 553-557** claim the smoke
   "deploys final DA and final settlement CellScript verifier
   artefacts, submits final-script transactions, and requires
   final settlement type args to be `session_id_hash ||
   settlement_identity_hash`". This is true for `da-anchor-final.cell`
   and `settlement-final.cell` (smoke lines 700-1500) but the
   architectural prose is loose: the runbook and live script do
   not exercise this path against a public testnet.

The remaining findings are correctness/coverage gaps and doc drift
items the branch did not address.

## Findings

| # | Severity | Component | Finding | File:Line | Doc claim | Code reality |
|---|----------|-----------|---------|-----------|-----------|--------------|
| F-DOC-01 | **CRITICAL** | cellscript / cli | `da-anchor-final.cell` has no CLI consumer that builds a final-script DA submission report outside `scripts/myelin_ckb_devnet_smoke.sh`. | `cellscript/examples/myelin/da-anchor-final.cell:1-56`, `cli/src/main.rs:4584-4592`, `cli/src/main.rs:8282-8320` | All four fixtures are listed as tracked sources in the production gate (gate.sh line ~246) and the runbook (line 88-95). | The CLI's `carrier_payload_type_args_hex` only knows two carrier payload kinds: `myelin-session-da-anchor-carrier-v1` and `myelin-session-settlement-carrier-v1` (cli/src/main.rs:4587-4591). A `myelin-session-da-anchor-final-v1` or analogous final-DA carrier kind does not exist. The smoke (line 700-1500) and the cellscript v0_18 test (line 222-924) exercise `verify_final_da_publication` directly, but no production gate step or runbook command submits a final-DA carrier. |
| F-DOC-02 | **HIGH** | docs | `MYELIN_TEEWORLDS_REPRODUCIBILITY.md:64` claims `court_checks: 16`; the actual verifier emits 22 checks per bundle. | `MYELIN_TEEWORLDS_REPRODUCIBILITY.md:64`, `cli/src/main.rs:2112-2453`, `scripts/myelin_teeworlds_acceptance.sh:159` | "court_checks : 16" | `verify_teeworlds_court_bundle` (cli/src/main.rs:2112) calls `push_check(...)` exactly 22 times in the static and Tendermint branches combined: `chunk-payload-hash`, `molecule-transaction-hash`, `molecule-transaction-length`, `projection-possible`, `projection-profile`, `projection-source-txid`, `projection-raw-tx-hash`, `projection-wtx-hash`, `block-hash-recomputes`, `block-state-root-before-matches`, `block-state-root-after-matches`, `block-scheduler-commitment-matches`, `block-data-commitment-matches`, `evidence-block-hash-matches-canonical-block`, `challenge-payload-hash`, `committee-signature-hashes`, `committee-signer-ids`, `committee-certificate` or `tendermint-certificate`, `committee-quorum-weight` or `tendermint-quorum-power`, `court-verifiable-profile`, `vm-profile`, `ckb-spawn-ipc-not-required`. The acceptance script's summary emits `court_checks: len(checks)` (scripts/myelin_teeworlds_acceptance.sh:159). The doc count 16 is from before the data-binding reconstruction block was added (cli/src/main.rs:2214-2289). |
| F-DOC-03 | **HIGH** | runbook | Runbook documents `--verifier-role final-l1-script` and acceptance mode `final-l1-script`, but the live script has no role mapping for it. | `docs/public-testnet-rehearsal-runbook.md:402-409,482`, `scripts/myelin_public_testnet_rehearsal_live.sh:94-132` | "For the settlement carrier or final-script path, use the same command with the settlement package and settlement verifier. When rehearsing final-script settlement evidence, also provide the evidence cell dep and authority input arguments, and set `--verifier-role final-l1-script`". Acceptance: "readiness_evidence_mode is live-ckb-carrier or final-l1-script". | `role_config` (scripts/myelin_public_testnet_rehearsal_live.sh:94-132) only matches `da-anchor` and `settlement`; any other value exits non-zero. The carrier-submission invocation (line 159-180) hardcodes `--verifier-source "$verifier_source"` where `verifier_source` is `$REHEARSAL_DIR/da-anchor-carrier.cell` (line 100) or `$REHEARSAL_DIR/settlement-carrier.cell` (line 115). The `--verifier-role` argument is forwarded to the CLI (line 175), so a user setting `--verifier-role final-l1-script` will still submit the carrier verifier (da-anchor-carrier.cell) tagged as final-l1-script. |
| F-DOC-04 | **HIGH** | docs | `state/README.md` still claims "1GB append-only files with mmap" segment storage and references a non-existent `kv/` module and `writer.rs`. | `state/README.md:9,42-47`, `state/src/store/segment.rs:1-580` (no mmap use; uses `OpenOptions::append`) | "Segment Storage: 1GB append-only files with mmap" | `state/src/store/segment.rs:113-176` uses `OpenOptions::new().append(true).create(true).open(...)` + `file.sync_data()`; no `memmap2` use anywhere in `state/src` (verified by `rg '^use memmap2' state/src`). The `state/Cargo.toml:34` declares `memmap2 = "0.9"` but it is dead code. The README was not updated by this branch. The swarm audit already noted this in `MYELIN_SWARM_AUDIT_STATE_DA.md` (F-06); the branch did not fix it. |
| F-DOC-05 | **HIGH** | cellscript / docs | The 4 new cellscript fixtures' resource identity (e.g. `identity(field(intent_hash))`) is declared but not enforced by the typed-cell CLI evidence path — `celltx/types.rs` has no `TypedCellDecl` entry for `SettlementFinal`, `SettlementCarrier`, `DaAnchorCarrier`, or `DaAnchorFinal`. | `cellscript/examples/myelin/settlement-final.cell:4`, `cellscript/examples/myelin/settlement-carrier.cell:4`, `cellscript/examples/myelin/da-anchor-carrier.cell:4`, `cellscript/examples/myelin/da-anchor-final.cell:4`, `exec/src/celltx/types.rs:1-3637` | The cellscript resource declarations `identity(field(intent_hash))` and `identity(field(da_manifest_hash))` describe identity semantics. | `rg 'SettlementFinal\|SettlementCarrier\|DaAnchorCarrier\|DaAnchorFinal' exec/src cellscript/src --include='*.rs'` returns no matches. The `TypedCellDecl` table at exec/src/celltx/types.rs is populated only by the cellscript compiler's typed-cell metadata path; the CLI's `session_carrier_submission` validates carrier payload fields directly against the package's declared hashes (cli/src/main.rs:4610-4656) and does not consult any `TypedCellDecl` for these fixture types. The cellscript metadata sidecars (e.g. `settlement-final.s.meta.json:1782`) do declare a `schema` string, but the type-name is never registered in the runtime CellTx model. The identity policy is therefore a typed-cell-only annotation with no on-chain enforcement today. |
| F-DOC-06 | **MEDIUM** | docs | The architecture doc claims the smoke "deploys final DA and final settlement CellScript verifier artefacts, submits final-script transactions" but the smoke's "live" submission path is still dry-run for final settlement — only the *rejection probe* of a competing final-settlement output is live. | `docs/MYELIN_ARCHITECTURE.md:551-560`, `scripts/myelin_ckb_devnet_smoke.sh:820-960` | "submits final-script transactions" | The smoke script's "submit_and_verify_carrier" final-settlement path (smoke.sh:1551-1565) uses `verifier_role=final-l1-script` and `verifier_source=settlement-final.cell`, but the underlying carrier submission does not mark `--require-accepted` for the final-script role (line 175 only sets `--submit`); the `accepted_by_rpc` is recorded only for the carrier paths. The competing-final-settlement probe (smoke.sh:825-960) is the only fully live final-script transaction the smoke runs, and it is a *rejection* check, not a successful submission. |
| F-DOC-07 | **MEDIUM** | runbook | Runbook hardcodes `--current-time-ms 60000 --challenge-window-ms 60000` for `session settlement-intent`; this works for the fixture (block.timestamp_ms=0) but the runbook never tells the operator how to choose a value for a real session. | `docs/public-testnet-rehearsal-runbook.md:264-266,283-286`, `cli/src/main.rs:6979-7004` | "challenge-window-ms 60000" with no derivation guidance. | `session_settlement_intent_with_court_deployment` requires `current_time_ms >= block.timestamp_ms + challenge_window_ms` (cli/src/main.rs:7004). The fixture path uses block.timestamp_ms=0 (cli/src/main.rs:1959), so any current_time_ms ≥ 60000 passes. For a real session, the operator must know both the chunk's `timestamp_ms` and the chosen challenge-window; the runbook does not tell them how to read the chunk's timestamp back. |
| F-DOC-08 | **MEDIUM** | docs | `MYELIN_USE_CASE_POSITIONING.md` is internally disciplined on architecture-fit vs production-evidence (sections 1, 6.3) but `docs/MYELIN_ARCHITECTURE.md` introduces production-evidence-shaped claims (e.g. "live final-script transactions are deployed") without separating architecture-fit from production-evidence. | `MYELIN_USE_CASE_POSITIONING.md:10-21, 252-264`, `docs/MYELIN_ARCHITECTURE.md:551-595,612-615` | Positioning doc explicitly: "suitable / fits / is appropriate is a claim of type (1). Validated / shown / measured is a claim of type (2). The two are deliberately kept apart." | The architecture doc conflates "devnet smoke" (production-evidence) with "the smoke also deploys final DA and final settlement CellScript verifier artefacts" (architecture-fit) and "live rejection of mismatched carrier data" (production-evidence) within the same paragraph without a discipline label. The architecture doc has no equivalent of positioning.md's "Safe to claim / Not safe to claim" split. |
| F-DOC-09 | **MEDIUM** | docs / cli | `MYELIN_PRODUCTION_GATE.md` lists `court_checks : 22` and is the only doc that matches the implementation; the production-rehearsal report does not re-state the number, leaving the audit chain ambiguous. | `MYELIN_PRODUCTION_GATE.md:166`, `MYELIN_PRODUCTION_REHEARSAL_REPORT.md:1-123` (no mention of court_checks) | The gate "passed" and "Teeworlds section of the gate produced: ... court_checks : 22". | The rehearsal report only references the gate by name; a reviewer reading the rehearsal report alone cannot reconcile which court_checks count is current. The swarm audit's `MYELIN_SWARM_AUDIT_MEMPOOL_CONSENSUS.md:1-30` re-states "22 court_checks" (in its scope, not as a finding), confirming the gate number but not the reproducibility doc. |
| F-DOC-10 | **MEDIUM** | runbook / cli | Runbook Phase 2 step requires an external DA receipt signed by the provider with `$MYELIN_DA_PROVIDER_PUBKEY_HASH` and `$MYELIN_DA_PROVIDER_SIGNATURE`; the prepare script (`scripts/myelin_public_testnet_rehearsal_prepare.sh:132`) uses an unrelated env-var `MYELIN_LOCAL_DA_PROVIDER_SECRET_KEY` to sign locally. The two paths use different env-var conventions and the runbook does not flag this divergence. | `docs/public-testnet-rehearsal-runbook.md:54-59,176-209`, `scripts/myelin_public_testnet_rehearsal_prepare.sh:63,108-133` | Phase 2 expects external pubkey-hash + signature. | `prepare.sh` defaults to a synthetic 32-byte secret key (`hex_repeat 44 32`) and signs locally with `--provider-secret-key`. The two scripts produce equivalent receipts but via different env-var names (`MYELIN_DA_PROVIDER_PUBKEY_HASH` / `MYELIN_DA_PROVIDER_SIGNATURE` vs `MYELIN_LOCAL_DA_PROVIDER_SECRET_KEY`). An operator who runs `prepare.sh` to bootstrap and then runs the runbook Phase 2 verbatim will not have a pubkey-hash+signature to feed. |
| F-DOC-11 | **MEDIUM** | docs | `MYELIN_ARTEFACT_CLEANUP.md` and `MYELIN_STALE_SURFACE_AUDIT.md` are themselves deleted by this branch (358 and 386 lines respectively), but no replacement doc explains the cleanup rationale in the current tree. | `git log main..HEAD -- MYELIN_ARTEFACT_CLEANUP.md MYELIN_STALE_SURFACE_AUDIT.md MYELIN_SCHEDULER_AUDIT.md MYELIN_CLI_AUDIT.md`, `MYELIN_SESSION_L2_PLAN.md:519-536` | The diff says "completed in this preparation pass" lists removed surface. | A reviewer auditing "what stale surface was removed" must reconstruct from `git log -p`; the four removed audit docs (ARTEFACT_CLEANUP, STALE_SURFACE_AUDIT, SCHEDULER_AUDIT, CLI_AUDIT) are gone. The branch's doc updates do not include a one-line summary pointing readers to `git show <commit>^:<file>` for the previous audit rationale. |
| F-DOC-12 | **MEDIUM** | docs | `MYELIN_SESSION_L2_PLAN.md` references "scripts/myelin_protocol_gate.sh remains only as a compatibility wrapper that delegates to the production gate" in section 6 prose ("Behaviour"); the actual diff (main..HEAD) deletes this file (the file is not in the diff because main..HEAD omits files deleted in main), but `scripts/myelin_protocol_gate.sh` does not exist in HEAD either, and the production gate does not call into a wrapper. | `git diff main..HEAD -- MYELIN_SESSION_L2_PLAN.md MYELIN_PRODUCTION_GATE.md README.md` (mentions deleted) | "scripts/myelin_protocol_gate.sh remains only as a compatibility wrapper" | `ls scripts/myelin_protocol_gate.sh` on HEAD returns no file. The diff confirms the doc was *updated* to remove the reference (README.md line 116-122 of `main` deleted; MYELIN_ARCHITECTURE.md line 862-865 of `main` deleted). The plan doc section 8 / Architecture doc are clean. The remaining phrase "and the production gate enforces all of the above" in the L2 plan (line 553) is correct. No bug. |
| F-DOC-13 | **MEDIUM** | cellscript | All 4 fixtures declare `[u8; 64]` lock args on the type-script via `script::args(expected_type_args)`, but `cellscript/examples/myelin/da-anchor-final.cell:13` uses the same single-arg signature while the CLI's `carrier_payload_type_args_hex` for the missing `myelin-session-da-anchor-final-v1` kind would default to `format!("0x{data_hash_hex}")` (32 bytes only). | `cellscript/examples/myelin/da-anchor-final.cell:13`, `cli/src/main.rs:4584-4592` | The fixture expects `[u8; 64]` type args (prefix + suffix). | If a future CLI helper is added for `myelin-session-da-anchor-final-v1`, the current `carrier_payload_type_args_hex` falls through to the `_` arm (line 4590) which produces only 32 bytes — not the 64-byte layout `da-anchor-final.cell` expects. A new helper must produce 64 bytes (`data_hash || payload[..32]`). |
| F-DOC-14 | **MEDIUM** | docs | Runbook Phase 1 line 134-150 uses `myelin session open-fixture --consensus static-closed-committee`. The CLI's `SessionOpenFixtureArgs.consensus: String` (cli/src/main.rs:5575) accepts any string; `static-closed-committee` is valid; `--consensus` is a per-command flag, not a global default. | `docs/public-testnet-rehearsal-runbook.md:135`, `cli/src/main.rs:5575-5645` | Runbook Phase 1 expects `--consensus static-closed-committee`. | The CLI parser at cli/src/main.rs:5575 takes `consensus: &str` without enum-validation; if the operator types `--consensus tendermint` it will succeed (Tendermint fixture exists). The prepare script does not pass `--consensus` and uses the default; the runbook and prepare script disagree on the explicit-vs-default form. |
| F-DOC-15 | **MEDIUM** | docs | `MYELIN_SESSION_L2_PLAN.md:152` says `myelin session open` accepts `--app-id myelin-custom-game-session-v1 --participant alice --participant bob --escrow-cell '<tx_hash_hex>:0:1000:<lock_hash_hex>'`. The CLI command requires `--app-id` but the CLI tests use `--app-id` in `session_carrier_submission`-style flows, not in the `session open` path. | `MYELIN_SESSION_L2_PLAN.md:141`, `cli/src/main.rs:5575-5645` | The plan documents a full descriptor-driven `session open` invocation. | `session_open` (cli/src/main.rs:5575) only handles the fixture path; `open` is described in section 6 prose (line 175) as "creates a session from CLI-supplied participants and escrow-like input Cells" but the implementation is not exposed as a CLI subcommand in `SessionOpenArgs` (only `SessionOpenFixtureArgs`). The plan documents a CLI surface that the code does not implement. |
| F-DOC-16 | **LOW** | docs / cli | `MYELIN_PRODUCTION_REHEARSAL_REPORT.md:43` says "Final-script submission path: Unit fixtures and final-script readiness checks". The cellscript v0_18 test plus the smoke (line 700-1500) actually exercise final-script fixtures in ckb-testtool AND in CKB devnet — `unit fixtures` undersells the coverage. | `MYELIN_PRODUCTION_REHEARSAL_REPORT.md:43`, `cellscript/tests/v0_18.rs:652-925`, `scripts/myelin_ckb_devnet_smoke.sh:113-176` | "Unit fixtures" | The cellscript test exercises all four final-script fixtures in ckb-testtool (v0_18.rs:898-925). The smoke compiles and deploys all four ELFs into a CKB devnet (smoke.sh:113-176, 1527-1565). The report should say "ckb-testtool + devnet smoke" not "unit fixtures". |
| F-DOC-17 | **LOW** | docs | `MYELIN_SESSION_L2_PLAN.md` section 6 "Behaviour" line 222 says "a `--accepted-tx-hash` value is recorded separately and does not satisfy strict live production readiness." The architecture doc (line 525-535) and runbook (line 422-426) both describe acceptance as `submitted_to_rpc=true AND accepted_by_rpc=true`. | `MYELIN_SESSION_L2_PLAN.md:222`, `docs/MYELIN_ARCHITECTURE.md:525-535`, `docs/public-testnet-rehearsal-runbook.md:418-426` | Three docs describe the same acceptance path with different emphasis. | The session L2 plan correctly distinguishes strict live submission from operator-supplied hash; the architecture and runbook text describe the runtime invariant without flagging the operator-supplied-hash shortcut. A reviewer could read the architecture doc and conclude that `--accepted-tx-hash evidence` satisfies the gate. |
| F-DOC-18 | **LOW** | docs | `MYELIN_PRODUCTION_REHEARSAL_REPORT.md:30-50` table column "Current artefact" lists commands like `myelin-cli committee finalise-demo`, `myelin-external-da-receipt-v2 test fixture in unit tests`. Some rows have no corresponding test function (e.g. "Reorg/retry/monitoring" rows reference behaviour but do not name a test). | `MYELIN_PRODUCTION_REHEARSAL_REPORT.md:30-50` | "Current artefact" column claims existence. | Several "Current artefact" rows are vague (e.g. line 46: "production gate mock JSON-RPC reports"). For the audit lane, this is acceptable as a reviewer-facing summary, but the rehearsal report's claim "production-evidence-complete prototype" cannot be verified solely from this table without reading the production gate. |
| F-DOC-19 | **LOW** | docs | `MYELIN_USE_CASE_POSITIONING.md:284-303` proposes a "small IoT acceptance" plan (100 sensors, 10 epochs, 1000 readings/epoch). The plan section is aspirational, not delivered; the doc is honest about it. | `MYELIN_USE_CASE_POSITIONING.md:288-303` | "A small, deterministic IoT acceptance that does not require a light client" | The doc says "proposed". The architecture doc and runbook do not carry this forward. Acceptable per the architecture-fit vs production-evidence discipline — the doc self-labels the claim as proposed. |
| F-DOC-20 | **LOW** | cellscript | The fixture `cellscript/examples/myelin/settlement-final.cell:39` checks `ckb::cell_type_args_empty(output)` and returns 51. The settlement-carrier (line 22) checks `ckb::cell_type_args_empty(output)` and returns 31. Both fail-closed on empty args, but the cellscript metadata sidecar shows `cell-type-script-args-empty-read` is a *fail-closed* runtime feature (e.g. settlement-final.s.meta.json fail-closed list at line 87). | `cellscript/examples/myelin/settlement-final.cell:39`, `cellscript/examples/myelin/settlement-carrier.cell:22`, `settlement-final.s.meta.json:87-89` | The fixture is "covered" by the cellscript compiler. | `cellscript/examples/myelin/settlement-final.s.meta.json:87-89` lists `"fail_closed_runtime_features": ["fixed-byte-comparison"]`. The fixture's `cell-type-script-args-empty-read` is not in the fail-closed list; it is `ckb-runtime` (line 191-194). This is internally consistent — the cellscript compiler treats type-args-empty as a CKB syscall, not as a fail-closed surface. The naming "fail-closed" in the metadata is per-feature; it is not a global gate. |
| F-DOC-21 | **LOW** | docs | `docs/MYELIN_ARCHITECTURE.md:8` describes Myelin as having "its own scheduler, state root, finality, and benchmark". The "benchmark" framing in the architecture doc headline is unusual; the runbook and positioning doc do not foreground "benchmark" in their headline. | `docs/MYELIN_ARCHITECTURE.md:8`, `MYELIN_USE_CASE_POSITIONING.md:1-50`, `MYELIN_SESSION_L2_PLAN.md:1-25` | Headline descriptions vary by doc. | The architecture doc headline emphasizes "benchmark", the session L2 plan emphasizes "session L2", the positioning doc emphasizes "CKB-isomorphic finite Cell session L2". This is a stylistic divergence, not a contradiction. |
| F-DOC-22 | **LOW** | docs | `MYELIN_SESSION_L2_PLAN.md:496` says "Session fixture open/commit/court/verify/DA/DA-anchor-submit-dry-run/settlement-intent/package flow for both consensus modes". The production gate (line 12 step 12) uses a slightly different phrasing: "Session fixture open/commit/court/DA/settlement plus mock CKB context, economics, inclusion, stability, finality, and aggregate readiness verification". | `MYELIN_SESSION_L2_PLAN.md:496`, `MYELIN_PRODUCTION_GATE.md:50` | Two descriptions of the same gate step. | The session L2 plan description omits the readiness aggregation step (`verify-submission-readiness`) which is the actual final step in the production gate. A reviewer following the plan would expect the gate to stop at `verify-settlement-package`, but it actually runs seven additional verifiers. |
| F-DOC-23 | **INFO** | cellscript / cli | The 4 fixtures are correctly referenced by the cellscript v0_18 test (`MYELIN_DA_ANCHOR_CARRIER_TYPE_PROGRAM` etc., cellscript/tests/v0_18.rs:225-228) and the smoke script (scripts/myelin_ckb_devnet_smoke.sh:114-117). The reuse is consistent across all consumers. | `cellscript/tests/v0_18.rs:225-228`, `scripts/myelin_ckb_devnet_smoke.sh:114-117`, `scripts/myelin_public_testnet_rehearsal_prepare.sh:80-83` | Fixture is "tracked and deployed". | Confirmed. All four .cell files are `include_str!`'d in the cellscript test and `cp`'d by both shell scripts. |
| F-DOC-24 | **INFO** | docs / cli | The 6 templates in `docs/templates/public-testnet-rehearsal/` match the CLI's `operator_custody_policy_document_evidence` (cli/src/main.rs:9955-10007) and `operator_runbook_document_evidence` (cli/src/main.rs:10009-10083) field-by-field. The operator-custody template's `signing_threshold: 2` and `operator_count: 3` satisfy the CLI's `operator_count >= signing_threshold` check. The operator-runbook template's `min_confirmations: 6, min_fee_shannons: 1, min_fee_rate_shannons_per_kb: 1000, max_fee_shannons: 100000` matches the live-script defaults (scripts/myelin_public_testnet_rehearsal_live.sh:16-19) and the runbook's phase 5 invocation (line 437-444). | `docs/templates/public-testnet-rehearsal/operator-custody-policy.json`, `docs/templates/public-testnet-rehearsal/operator-runbook.json`, `cli/src/main.rs:9955-10083`, `scripts/myelin_public_testnet_rehearsal_live.sh:16-19` | Templates are "shape references". | Cross-checked field-by-field: all required string/bool/u64 fields are present; the `signing_threshold` is positive; `operator_count >= signing_threshold`; `min_confirmations`, `min_fee_shannons`, `min_fee_rate_shannons_per_kb`, `max_fee_shannons` values match the live script defaults. The `threshold-lock-deployment.template.json` and `court-economics-deployment.template.json` use the `data2` hash type and `testnet-beta-...` deployment policy, matching the CLI's `default_value = "data2"` and `deployment_policy` default. |
| F-DOC-25 | **INFO** | docs / cli | `external-da-receipt.template.json` schema `myelin-external-da-receipt-v2` matches the CLI's `parse_external_da_receipt` (cli/src/main.rs:2860-2897). Field names, types, and required/optional status are consistent. | `docs/templates/public-testnet-rehearsal/external-da-receipt.template.json:1-16`, `cli/src/main.rs:2856-2975` | Template is "shape reference". | Confirmed. All fields (`schema`, `provider`, `namespace`, `payload_hash`, `segment_root`, `receipt_id`, `availability_window`, `service_level`, `retention_seconds`, `retrieval_endpoint`, `audit_log_commitment`, `provider_pubkey_hash`, `provider_signature`) match the CLI parser. The `provider_message_hash` is recomputed by the CLI; the template does not include it (correct — it is computed from the other fields). |
| F-DOC-26 | **INFO** | docs / cli | `authority-signature-evidence.template.json` schema `myelin-session-authority-signature-evidence-v1` matches the CLI's `SessionAuthoritySignatureEvidence` struct (cli/src/main.rs:3267-3283). The template includes `signer_pubkey_hashes[]`, `signatures[]`, and `attestation_hashes[]`; the CLI generates `signatures` from `--signer-secret-key` or external `--signer-pubkey-hash/--signature` inputs. | `docs/templates/public-testnet-rehearsal/authority-signature-evidence.template.json:1-21`, `cli/src/main.rs:3214-3284` | Template is "shape reference". | Confirmed. The `signature_scheme: secp256k1-recoverable-blake3-pubkey-hash20` matches the CLI's `auth.signature_scheme`. The `message_hash` field is set from the package's `settlement_authority.authority_authentication.message_hash`. |
| F-DOC-27 | **INFO** | docs | No residual references to deleted `exec/IMPLEMENTATION_SUMMARY.md` (308 lines) or `exec/src/vm/README_VM_STATUS.md` (190 lines). | `rg -rn 'IMPLEMENTATION_SUMMARY\.md\|README_VM_STATUS\.md' --include='*.md' --include='*.rs' --include='*.toml'` (no matches) | Deletion is clean. | Confirmed. The branch's deletion is complete; no code or doc still references the removed artefacts. |
| F-DOC-28 | **INFO** | docs | No residual references to deleted `docs/ARCHITECTURE.md` (90 lines). All in-tree docs that referenced it now reference `docs/MYELIN_ARCHITECTURE.md`. | `rg -rn 'docs/ARCHITECTURE\.md\|MYELIN_ARCHITECTURE Seed' --include='*.md'` | Deletion is clean. | Confirmed via grep. `state/README.md`, `exec/README.md`, `mempool/README.md` all reference the renamed `docs/MYELIN_ARCHITECTURE.md`. |
| F-DOC-29 | **INFO** | docs | `cellscript/examples/myelin/*.cell` fixtures are listed in `.gitignore`? No — they are tracked. The compile output `*.s` and `*.s.meta.json` are emitted next to the fixtures but are not committed (verified by `git status`). | `cellscript/examples/myelin/`, `git status` | "Tracked Myelin CellScript verifier sources". | Confirmed. The fixtures are tracked; the compile outputs are untracked (regenerable). |
| F-DOC-30 | **INFO** | docs | `MYELIN_USE_CASE_POSITIONING.md` (sections 1, 6.3) explicitly maintains the architecture-fit vs production-evidence discipline. No claim in the positioning doc is collapsed across the two categories. | `MYELIN_USE_CASE_POSITIONING.md:10-21,208-264` | Positioning discipline. | Confirmed. "suitable / fits / is appropriate" claims are isolated; "validated / shown / measured" claims are isolated; throughput/latency/Iot-scale claims are explicitly listed as not-safe. |
| F-DOC-31 | **INFO** | cellscript | The 4 fixtures use a permissive cellscript grammar that the cellscript compiler accepts (`./target/debug/cellc examples/myelin/*.cell` succeeds for all 4). The cellscript v0_18 test then compiles under `--target-profile typed-cell` and `--target-profile ckb` and runs them as CKB type scripts in ckb-testtool. | `./target/debug/cellc examples/myelin/{settlement-final,settlement-carrier,da-anchor-carrier,da-anchor-final}.cell` (verified: "compiled successfully" for all 4) | Fixtures compile. | Confirmed. All four .cell files parse, type-check, lower, and emit RISC-V assembly. The `metadata.json` sidecars are emitted under `examples/myelin/`. |

## Cellscript fixtures consistency

All 4 fixtures share a common structural pattern:

```text
module myelin::<name>
resource <Type> has store, create
    identity(field(<primary_field>))
{
    <primary_field>: Hash,
    <other_fields>: Hash,
}
action verify_<name>(expected_type_args: [u8; 64], ...) -> u64 {
    verification
        // source::group_output(0) reads
        // ckb::cell_data_size(...) checks (160 or 192 bytes)
        // ckb::cell_data_hash_at(..., offset) field reads
        // ckb::cell_type_args_prefix_hash / suffix_hash bindings
        // ckb::cell_type_code_hash / ckb::cell_type_hash_type reads
        // ckb::require_cell_type_args_prefix_hash / suffix_hash
        // ckb::require_cell_type_script_hash_type
        // script::require_cell_type_matches
        return 0  // or one of the documented error codes (e.g. 30..69)
}
```

The field layouts are:

| Fixture | Data size | Fields | Identity field |
|---|---|---|---|
| `da-anchor-carrier.cell` | 160 | `da_manifest_hash`, `court_bundle_hash`, `challenge_payload_hash`, `segment_root`, `molecule_transaction_hash` | `da_manifest_hash` |
| `da-anchor-final.cell` | 160 | `da_manifest_hash`, `court_bundle_hash`, `challenge_payload_hash`, `segment_root`, `molecule_transaction_hash` | `da_manifest_hash` |
| `settlement-carrier.cell` | 160 | `intent_hash`, `court_bundle_hash`, `da_manifest_hash`, `challenge_payload_hash`, `final_state_root` | `intent_hash` |
| `settlement-final.cell` | 160 (output) + 192 (authority input) | output: same as settlement-carrier; authority: 6-field lineage | `intent_hash` |

The `settlement-final.cell` authority-input check (`cell_data_size(authority_input) != 192`) matches `settlement_authority_cell_data` at `cli/src/main.rs:4389-4405`, which builds a 192-byte payload from 6 hashes.

Cross-fixture relationship verified:

```text
settlement-final.cell:88    published_da_manifest_hash == da_manifest_hash  (binds settlement to DA-anchor)
settlement-final.cell:95-97 da_lock_hash == authority_lock_hash == output_lock_hash  (binds all three to DA lock)
da-anchor-final.cell:50    type-args prefix=data_hash, suffix=da_manifest_hash  (matches CLI carrier_payload_type_args_hex)
settlement-carrier.cell:46-47  type-args prefix=data_hash, suffix=intent_hash  (matches CLI carrier_payload_type_args_hex)
```

The CLI's `carrier_payload_type_args_hex` (cli/src/main.rs:4584-4592) produces exactly `data_hash(32) || payload[..32](32)` = 64 bytes for both `myelin-session-da-anchor-carrier-v1` and `myelin-session-settlement-carrier-v1`, matching the fixtures' `[u8; 64]` lock-args type. The fixtures for `da-anchor-final` and `settlement-final` cannot be exercised by `session_carrier_submission` because the CLI has no `myelin-session-da-anchor-final-v1` or analogous kind — see F-DOC-01.

The `da-anchor-final.cell` fixture reads `da_input` as a `CellDep` (not as an input); settlement-final reads `da_input` the same way (`source::cell_dep(0)`). Both use `ckb::cell_data_hash_at(da_input, 0)` to extract the `da_manifest_hash` field. This matches the smoke script's wiring of `verify_final_da_publication` and `verify_final_settlement` as evidence-CellDep fixtures.

## Template ↔ CLI ↔ script cross-reference

| Template | Schema | CLI consumer | Runbook consumer | Prepare/live script consumer | Status |
|---|---|---|---|---|---|
| `external-da-receipt.template.json` | `myelin-external-da-receipt-v2` | `session external-da-receipt` (cli/src/main.rs:2860-2975) | runbook.md:172-209 | prepare.sh:108-133 | All fields schema-checked and signature-checked |
| `authority-signature-evidence.template.json` | `myelin-session-authority-signature-evidence-v1` | `session authority-signature-evidence` (cli/src/main.rs:3214-3284) | runbook.md:310-316 | prepare.sh:194-198 | All fields schema-checked |
| `threshold-lock-deployment.template.json` | `myelin-session-threshold-lock-deployment-v1` | `session threshold-lock-deployment-evidence` (cli/src/main.rs:3286-3320) | runbook.md:318-329 | prepare.sh:199-210 | All fields schema-checked |
| `court-economics-deployment.template.json` | `myelin-session-court-economics-deployment-v1` | `session court-economics-deployment-evidence` (cli/src/main.rs:3319-?) | runbook.md:268-279 | prepare.sh:163-174 | All fields schema-checked |
| `operator-custody-policy.json` | `myelin-operator-custody-policy-v1` | `verify-submission-readiness --operator-custody-policy` (cli/src/main.rs:9955-10007) | runbook.md:473-474 | live.sh:220, 261 | All 10 fields validated; production gate checks empty-handed (production_gate.sh:1104-1114) |
| `operator-runbook.json` | `myelin-operator-runbook-v1` | `verify-submission-readiness --operator-runbook` (cli/src/main.rs:10009-10083) | runbook.md:474 | live.sh:221, 262 | All 14 fields validated; min-confirmations/min-fee/etc. must equal the readiness report's policy fields |

No template↔CLI field mismatches were found in this lane. The templates are usable as "shape references" per the README in `docs/templates/public-testnet-rehearsal/README.md:14-23`.

## Runbook walk-through

Phase 1 (lines 128-156): builds session artefacts. All commands exist in the CLI (`session open-fixture`, `commit-fixture`, `court-bundle`, `verify-court-bundle`). The `session-court-verify.json has valid = true` acceptance (line 156) is satisfiable. ✓

Phase 2 (lines 158-239): builds DA evidence. The `session da-manifest --bundle ... --out ...` invocation matches the CLI's `SessionDaManifestArgs`. The two receipt flows (signing-request then external signature) match the CLI's `SessionExternalDaReceiptArgs` (cli/src/main.rs:3089-3212). ✓

Phase 3 (lines 241-355): builds packages. All commands exist in the CLI. The `--authority-signature-evidence` and `--threshold-lock-deployment-evidence` flags exist on `session settlement-package` (cli/src/main.rs:1115). The acceptance criterion "deployment evidence files are labelled fixture, rehearsal, testnet, or real" (line 354) is documentary; the CLI does not enforce a label field. This is acceptable per the runbook's "not a gate" framing.

Phase 4 (lines 357-426): submit to public testnet. The carrier-submission command and its flags all exist. The runbook says "For the settlement carrier or final-script path, use the same command with the settlement package and settlement verifier. ... set `--verifier-role final-l1-script`" (lines 402-409). The acceptance criterion `accepted_by_rpc = true` requires an actual public testnet RPC. The runbook does not explicitly state that the smoke step's `accepted_by_rpc = true` requires a real RPC (it implies it).

Phase 5 (lines 427-487): observe inclusion, stability, finality. All commands exist. The `verify-submission-readiness` invocation (line 466-475) requires `--require-live-submission`, which means the referenced submission report must show non-dry-run RPC acceptance. The acceptance `production_submission_ready = true` (line 481) is achievable only on a real testnet.

Phase 6 (lines 489-512): update the rehearsal report. Documentary only.

Defects identified:

- F-DOC-07: the runbook hardcodes `--current-time-ms 60000 --challenge-window-ms 60000` without explaining how to choose these values for a real session.
- F-DOC-10: the runbook's env-var naming diverges from `prepare.sh`'s env-var naming for the DA receipt path.
- F-DOC-14: the runbook passes `--consensus static-closed-committee` while `prepare.sh` does not — both work but the inconsistency is undocumented.
- F-DOC-03: the runbook documents `--verifier-role final-l1-script` acceptance mode but the live script has no such role mapping.

The runbook is otherwise runnable end-to-end against the diffed code.

## Adversarial-evidence matrix walk-through

The matrix (`docs/adversarial-evidence-matrix.md:1-76`) lists 19 evidence areas. For each, I verified:

1. **Court bundle binding** (`session_court_bundle_is_single_chunk_projectable`, `session_court_bundle_rejects_tampered_state_root`) — covered. Both tests exist in cli/src/main.rs. The negative test for tampered state root exists at line 17467 (`session_court_bundle_rejects_tampered_state_root`).

2. **DA manifest payload binding** (`session_da_manifest_binds_to_verified_court_bundle_payload`, `session_da_manifest_rejects_tampered_segment_root`) — covered. Tests exist at cli/src/main.rs:13114, 13400.

3. **External DA receipt binding** (`session_da_manifest_binds_external_da_receipt_evidence`, `session_da_manifest_accepts_signed_production_da_receipt`) — covered. The negative path is "Receipt mismatch and signature failure are exercised inside `session_da_manifest_binds_external_da_receipt_evidence`; forged production readiness is rejected by `session_submission_readiness_rejects_forged_production_da_flag`" — both tests exist.

4. **DA production readiness** (`session_submission_readiness_clears_da_blocker_for_recomputed_production_da_manifest` vs `session_submission_readiness_rejects_forged_production_da_flag`) — covered. The matrix correctly states the negative path.

5. **DA anchor package binding** (`session_da_anchor_package_binds_verified_manifest_into_ckb_projectable_celltx` vs `session_da_anchor_package_rejects_tampered_manifest_hash`) — covered. Tests exist at cli/src/main.rs:13471.

6. **DA anchor submission RPC binding** (`session_da_anchor_submission_records_rpc_acceptance` vs `session_da_anchor_submission_rejects_missing_live_input_before_broadcast`, `session_da_anchor_submission_rejects_rpc_hash_mismatch`) — covered. Tests exist.

7. **Settlement intent binding** (`session_settlement_intent_binds_to_verified_court_bundle_and_challenge_window` vs `session_settlement_intent_rejects_premature_settlement_permission`) — covered.

8. **Court economics deployment evidence** (`session_settlement_intent_accepts_bound_court_economics_deployment_evidence` vs `session_settlement_intent_rejects_stale_court_economics_deployment_commitment`, `session_settlement_intent_rejects_tampered_court_economics`) — covered.

9. **Settlement package binding** (`session_settlement_package_binds_verified_intent_into_ckb_projectable_celltx` vs `session_settlement_package_rejects_tampered_intent_hash`, `..._authority_lineage`, `..._authority_authentication`) — covered.

10. **Authority signature evidence** (`session_settlement_package_accepts_bound_threshold_lock_deployment_evidence` vs `..._rejects_production_threshold_lock_without_participant_signatures`) — covered.

11. **Final settlement authority preflight** (`session_submission_readiness_accepts_end_to_end_production_final_settlement_evidence` vs `..._requires_final_settlement_authority_preflight`, `..._threshold_lock_deployment_preflight`, `..._uniqueness_evidence`) — covered.

12. **Settlement submission RPC binding** (`session_settlement_submission_records_rpc_acceptance` vs `session_settlement_submission_rejects_rpc_hash_mismatch`) — covered.

13. **CKB carrier inclusion** — covered with 3 positive and 3 negative tests.

14. **Context preflight** — covered with 2 positive and 2 negative tests.

15. **Economics preflight** — covered with 2 positive and 3 negative tests.

16. **Stability and finality** — covered with 2 positive and 2 negative tests.

17. **Readiness lineage and live submission** — covered with 3 positive and 6 negative tests.

18. **Operator policy evidence** — covered with 1 positive and 1 negative test.

19. **Carrier transaction construction** — covered with 3 positive and 5 negative tests.

20. **Runtime smoke** — covered with 3 positive and 1 negative test.

No checked cell in the matrix maps to a missing implementation. No unchecked cell whose absence would let an attack through, in this lane.

The matrix is internally consistent and consistent with the test inventory in cli/src/main.rs.

## Top-level audit doc cross-consistency

### MYELIN_PRODUCTION_GATE.md vs MYELIN_PRODUCTION_REHEARSAL_REPORT.md

The two docs agree on the production posture ("production-evidence-complete prototype / public-testnet rehearsal candidate"). They agree on the exit criteria (public-testnet rehearsal artefacts replace mocks). The gate doc reports `court_checks: 22` (line 166) while the rehearsal report does not re-state the number — see F-DOC-09. The gate doc's step 12 (line 50) lists the session fixture chain but omits readiness aggregation (see F-DOC-22). No other contradictions.

### MYELIN_SESSION_L2_PLAN.md vs MYELIN_CONSENSUS_COMPLETENESS.md

The plan says (line 95-96) "The same fixture finalises with Tendermint and produces identical state transition commitments but different finality evidence". The completeness doc says (line 87-90) "Tendermint-style weighted precommit finality is a round-bound, height-bound, weighted quorum over a fixed validator set." The plan accepts this as the implementation. The completeness doc confirms the implementation matches the doc.

The plan says (line 549) "The state transition is consensus-independent; only finality evidence differs." The completeness doc confirms this through the `consensus_kind` discriminator (completeness.md line 247) and the signature domain separation (completeness.md line 113-128). No contradiction.

### MYELIN_USE_CASE_POSITIONING.md vs docs/MYELIN_ARCHITECTURE.md

The positioning doc is internally disciplined on architecture-fit vs production-evidence (sections 1, 6.3). The architecture doc is not — see F-DOC-08.

The positioning doc says (line 256-260) "Safe to claim: 'Myelin is a CKB-isomorphic finite Cell session L2 with deterministic off-chain execution, committee finality, and a CKB projection path. The runtime spine is exercised by the production gate; the built-in session fixture proves the protocol spine; the Teeworlds workload is the reference external acceptance.'" This is consistent with the architecture doc's headline framing. ✓

The positioning doc says (line 260-263) "Not safe to claim: specific throughput numbers, specific latency numbers, 'supports IoT at scale', 'supports high-frequency finance', 'production-ready' for any vertical beyond the Teeworlds reference." The architecture doc does not contradict this — it does not claim throughput/latency numbers. ✓

### MYELIN_SESSION_L2_PLAN.md self-contradictions

The plan is 1233 lines. I read it end-to-end. Self-contradictions found:

- F-DOC-22 (line 496 vs production gate step 12) — the plan describes the gate step omitting readiness aggregation. Not strictly a self-contradiction in the plan (the plan does not contradict itself), but a discrepancy between the plan's description of the gate and the gate's actual content.
- F-DOC-15 (line 141 vs cli/src/main.rs:5575) — the plan documents `session open --app-id --participant --escrow-cell` but the implementation only exposes `session open-fixture`. The plan's own prose at line 175 says "open creates a session from CLI-supplied participants and escrow-like input Cells" but the implementation does not match.

No internal contradictions between plan sections (e.g. section 4 P0 vs section 6 P2 are consistent). The plan's milestone exit criteria (line 539-553) are consistent with the production gate's step 12.

## Architecture doc drift (docs/MYELIN_ARCHITECTURE.md)

The branch's diff added ~130 lines and removed very few. The 130-line delta covers:

- External DA receipt path: line 458-485 (new). Matches the CLI's `SessionExternalDaReceiptArgs` (cli/src/main.rs:3089-3212). ✓
- Operator policy path: line 459-466 (new). Matches the CLI's `operator_custody_policy_document_evidence` and `operator_runbook_document_evidence` (cli/src/main.rs:9955-10083). ✓
- DA manifest with external receipt: line 487-492. Matches the CLI. ✓
- `final_l1_script_submission_ready` / `end_to_end_production_ready` / `end_to_end_production_blockers`: line 517-528. Matches the CLI's `SessionSubmissionReadinessReport` and `end_to_end_blockers` calculation. ✓
- Final DA and settlement verifier deployment: line 551-573. Matches the smoke (scripts/myelin_ckb_devnet_smoke.sh:113-176, 820-960). ✓
- Typed-cell regression: line 605-615. Matches the v0_18 test. ✓

Architectural claims that I could not verify against code:

- Line 463-464: "with `--external-da-receipt`, ... which can make DA availability `testnet_beta_ready`." Verified via cli/src/main.rs:6100-6204 (`session_da_manifest`). ✓
- Line 569-573: "only checked mainnet deployment evidence can set the package-level authority `production_ready` marker." Verified via cli/src/main.rs:3286-3320 (threshold-lock deployment evidence). ✓
- Line 581: "before `court_economics.production_ready` can become true." Verified via cli/src/main.rs:3319. ✓

No stale architectural descriptions that reference deleted artefacts (the deletion of `docs/ARCHITECTURE.md` is clean — see F-DOC-28).

No stale references to deleted `exec/IMPLEMENTATION_SUMMARY.md` or `exec/src/vm/README_VM_STATUS.md` (see F-DOC-27).

## Cleanup audit doc consistency

The branch deleted (per git diff main..HEAD):

```text
MYELIN_ARTEFACT_CLEANUP.md         (358 lines)
MYELIN_CLI_AUDIT.md                (284 lines)
MYELIN_SCHEDULER_AUDIT.md          (249 lines)
MYELIN_STALE_SURFACE_AUDIT.md      (386 lines)
docs/ARCHITECTURE.md               (90 lines)
exec/IMPLEMENTATION_SUMMARY.md     (308 lines)
exec/src/vm/README_VM_STATUS.md    (190 lines)
scripts/myelin_protocol_gate.sh    (9 lines)
reports/myelin-teeworlds-repro.json (102 lines)
```

Verification of remaining references:

- `rg -rn 'IMPLEMENTATION_SUMMARY\.md\|README_VM_STATUS\.md'` → 0 matches.
- `rg -rn 'docs/ARCHITECTURE\.md\|MYELIN_ARCHITECTURE Seed'` → 0 matches.
- `rg -rn 'MYELIN_ARTEFACT_CLEANUP\.md\|MYELIN_STALE_SURFACE_AUDIT\.md\|MYELIN_SCHEDULER_AUDIT\.md\|MYELIN_CLI_AUDIT\.md'` → 0 matches.
- `rg -rn 'myelin_protocol_gate\.sh'` → 0 matches.
- `rg -rn 'reports/myelin-teeworlds-repro\.json'` (in source) → only the report's own build script references itself (scripts/build_myelin_teeworlds_repro.py is unchanged on this branch).

The deletion is complete (see F-DOC-11 for the missing audit-trail rationale — the deleted docs are not replaced with a single replacement doc, but the diff itself is the audit trail).

## README consistency

| README | Lines | Status |
|---|---|---|
| `README.md` (root) | ~570 (was 421 before branch) | Modified; references `MYELIN_PRODUCTION_GATE.md`, runbook, evidence matrix. Consistent with code. |
| `mempool/README.md` | 53 | Modified; replaces "Under Construction" framing with "Myelin Mempool" framing. The swarm audit already noted F-12 (priority order description in README vs code). The branch did not fix F-12. |
| `state/README.md` | 56 | Modified; **does not** fix F-06 / F-09 from MYELIN_SWARM_AUDIT_STATE_DA.md (1GB-mmap claim; missing `cells_by_lock`, `segments`, `spend_journal` description; non-existent `kv/` module). See F-DOC-04. |
| `exec/README.md` | 51 | Modified; replaces "Under Construction" framing. The deleted `IMPLEMENTATION_SUMMARY.md` and `README_VM_STATUS.md` are not referenced here. The exec/src tree map (lines 14-35) lists `machine.rs`, `verifier.rs`, `syscalls/` — let me verify these match the actual file tree. |
| `cellscript/README.md` | 747 | Unchanged on this branch. Already covers the `.cell` fixtures, grammar, CLI commands. |

Let me verify the exec README's tree map (verified above): `machine.rs`, `verifier.rs`, `syscalls/`, `error.rs`, `cost_model.rs`, `scheduler.rs` all exist under `exec/src/vm/`. `celltx/`, `serialization/`, `scripts/` exist under `exec/src/`. The exec README's tree map is correct (exec/README.md:14-35). The exec/README.md does not describe `cost_model.rs` or `scheduler.rs`, but those are sub-modules under `vm/` and not user-facing, so this is acceptable.

The `state/README.md:9` claim about "1GB append-only files with mmap" is the only pre-existing README drift the branch did **not** fix (F-DOC-04). All other README drift items from prior swarm audits are either unaddressed (mempool F-12) or no longer applicable.

## Positioning discipline check (architecture-fit vs production-evidence)

Per the user's discipline:

> "Architecture-fit claims are fine on design alone; TPS / latency / scale claims need release-gate evidence."

Walked both the architecture doc and the positioning doc for claims in each category.

| Doc | Architecture-fit claim | Production-evidence claim | Discipline status |
|---|---|---|---|
| `MYELIN_USE_CASE_POSITIONING.md:1-50` | "Myelin is a CKB-isomorphic finite Cell session L2" (line 27-29); "suitable for" Tier 1 use cases | "Throughput numbers under sustained load" not shown (line 244); "Latency numbers for either consensus engine" not shown (line 245) | Disciplined; the doc explicitly maintains the split (sections 1, 6.3) |
| `docs/MYELIN_ARCHITECTURE.md:8` | "Cell ledger with its own scheduler, state root, finality" — design claim | (none — headline is architecture-fit only) | Disciplined at headline |
| `docs/MYELIN_ARCHITECTURE.md:486-595` | (none — section describes runtime mechanics) | "With `--external-da-receipt`, ... which can make DA availability `testnet_beta_ready`. A provider-signed production SLA receipt can additionally make DA availability `production_ready`" (line 492-495); "live rejection of mismatched carrier data" (line 595) | Mixed; the architecture doc presents production-evidence-shaped claims (testnet_beta_ready, production_ready) inline with architecture descriptions, without labeling them. The user-facing distinction (what is "shown" vs what is "suitable") is implicit, not explicit. See F-DOC-08. |
| `MYELIN_PRODUCTION_REHEARSAL_REPORT.md:30-50` | (none — the table is empirical only) | All rows are "Current artefact" + "Provenance" + "Mainnet gap" — the doc is production-evidence-shaped by design | Disciplined |
| `MYELIN_PRODUCTION_GATE.md:159-167` | (none) | Hard numbers: `tape_bytes 2162`, `vm_cycles 15_139_695`, `court_checks 22` | Disciplined; this is a gate document, all numbers are runtime evidence |
| `MYELIN_SESSION_L2_PLAN.md:1-25` | "CKB-isomorphic finite Cell session L2" (line 7); "bounded off-chain session ledger" | "the strongest claim currently supported by the repository is..." (line 542-560) | Disciplined; the plan makes architecture-fit claims first and labels production-evidence separately |

Net: the positioning doc is the most disciplined. The architecture doc is the least disciplined (F-DOC-08). The rehearsal report and gate are appropriately empirical. The session L2 plan correctly handles both categories.

The user's discipline is upheld at the macro level (each doc is roughly either architecture-fit or production-evidence). It is violated at the micro level in the architecture doc's mixed-mode paragraphs.

## Open questions

1. **F-DOC-01 (CRITICAL)**: Should `myelin-session-da-anchor-final-v1` (and a corresponding `myelin-session-settlement-final-v1`) be added to `carrier_payload_type_args_hex` so that the `da-anchor-final.cell` and `settlement-final.cell` fixtures can be exercised through `session carrier-submission --verifier-role final-l1-script` outside the devnet smoke? The cellscript tests and the devnet smoke exercise the fixtures, but a real public-testnet rehearsal would need a CLI helper that builds the final-script carrier transaction. As written, the runbook's Phase 4 documentation for `final-l1-script` references a CLI path that the CLI does not expose.

2. **F-DOC-02 (HIGH)**: The `MYELIN_TEEWORLDS_REPRODUCIBILITY.md:64` `court_checks: 16` is stale. Should the doc be updated to `court_checks: 22` to match the production gate (line 166), the swarm audit mempool/consensus doc, and the actual `verify_teeworlds_court_bundle` implementation (22 `push_check` calls)?

3. **F-DOC-03 (HIGH)**: The runbook at lines 402-409 documents `--verifier-role final-l1-script` and an acceptance mode `final-l1-script`. The live script (`scripts/myelin_public_testnet_rehearsal_live.sh:94-132`) does not implement a final-l1-script role mapping. Should the live script gain a `final-l1-script` role (using `settlement-final.cell` as the verifier source) or should the runbook retract the final-l1-script language until the helper exists?

4. **F-DOC-04 (HIGH)**: `state/README.md:9` still claims "1GB append-only files with mmap" segment storage and references a `kv/` module that does not exist. The swarm audit flagged this in F-06 / F-09. The branch rewrote the README framing but did not fix the underlying code/implementation description. Should this be addressed in a follow-up branch?

5. **F-DOC-05 (HIGH)**: The 4 cellscript fixtures declare `identity(field(...))` but `exec/src/celltx/types.rs` does not register `TypedCellDecl` entries for `SettlementFinal`, `SettlementCarrier`, `DaAnchorCarrier`, or `DaAnchorFinal`. Is the on-chain enforcement of `identity(field(...))` deferred to a future PR, or is the typed-cell metadata path the only enforcement surface?

6. **F-DOC-07 (MEDIUM)**: The runbook hardcodes `--current-time-ms 60000 --challenge-window-ms 60000`. For a real session, the operator must read the chunk's `timestamp_ms` from the bundle report (`session-court.json`) and choose `challenge-window-ms` accordingly. Should the runbook add a step that pipes `jq -r '.block.timestamp_ms' session-court.json` into the settlement-intent command?

7. **F-DOC-10 (MEDIUM)**: The runbook's Phase 2 env-var names (`MYELIN_DA_PROVIDER_PUBKEY_HASH`, `MYELIN_DA_PROVIDER_SIGNATURE`) diverge from the prepare script's env-var names (`MYELIN_LOCAL_DA_PROVIDER_SECRET_KEY`). Should one be renamed or the runbook should explicitly note the two paths?

8. **F-DOC-11 (MEDIUM)**: The branch deleted four top-level audit docs (`MYELIN_ARTEFACT_CLEANUP.md`, `MYELIN_CLI_AUDIT.md`, `MYELIN_SCHEDULER_AUDIT.md`, `MYELIN_STALE_SURFACE_AUDIT.md`) without leaving a replacement doc. A reviewer auditing "what was deleted and why" must reconstruct from `git log -p`. Should a single replacement doc be added?

9. **F-DOC-15 (MEDIUM)**: The session L2 plan (line 141) documents `myelin session open --app-id ... --participant ... --escrow-cell ...` but the CLI only exposes `session open-fixture`. Is the descriptor-driven `session open` a planned future surface, or should the plan be updated to match the current implementation?

10. **F-DOC-22 (MEDIUM)**: The session L2 plan (line 496) describes the production gate step as "Session fixture open/commit/court/verify/DA/DA-anchor-submit-dry-run/settlement-intent/package" while the production gate (line 50) lists additional readiness aggregation. Should the plan be updated to reflect the full gate content?

11. **F-DOC-08 (MEDIUM)**: The architecture doc does not adopt the architecture-fit vs production-evidence discipline that the positioning doc maintains. Should the architecture doc gain a "Safe to claim / Not safe to claim" section, or should it explicitly defer such framing to the positioning doc?

12. **F-DOC-13 (MEDIUM)**: If a future CLI helper is added for `myelin-session-da-anchor-final-v1`, it must produce 64-byte type args (`data_hash || payload[..32]`), not the 32-byte default that `carrier_payload_type_args_hex` falls through to for unknown kinds. Should the helper be added now, or should the runbook retract the `da-anchor-final.cell` reference until it does?