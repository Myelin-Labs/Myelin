# Lane D — Myelin Documentation Alignment & Fiber L2 Bridge Audit (main @ ab1111b)

> Verifier-only review. No fixes proposed. Scope: branch `main`, commit
> `ab1111b` (`Document Myelin Fiber L2 bridge plan`). Fresh audit that
> re-verifies the prior `audits/swarm-wholerepo/LANE_DOCS.md` findings on
> `main`, plus a fresh audit of the newly committed
> `docs/myelin-fiber-l2-bridge-plan.md`.

## 0. Fiber sibling checkout status

Sibling Fiber checkout **was reachable** at `/Users/arthur/RustroverProjects/fiber`.
All Fiber RPC method names, signatures, and behavior claims in the bridge
plan were verified against this checkout's source / `rpc/README.md`:

- `/Users/arthur/RustroverProjects/fiber/crates/fiber-lib/src/rpc/README.md`
- `/Users/arthur/RustroverProjects/fiber/crates/fiber-lib/src/rpc/channel.rs`
- `/Users/arthur/RustroverProjects/fiber/crates/fiber-lib/src/rpc/payment.rs`
- `/Users/arthur/RustroverProjects/fiber/crates/fiber-lib/src/rpc/invoice.rs` (implied)
- `/Users/arthur/RustroverProjects/fiber/crates/fiber-lib/src/fiber/channel.rs:3201` (sequence diagram)
- `/Users/arthur/RustroverProjects/fiber/docs/external-funding.md`
- `/Users/arthur/RustroverProjects/fiber/crates/fiber-json-types/src/channel.rs`
- `/Users/arthur/RustroverProjects/fiber/crates/fiber-json-types/src/dev.rs`

`/Users/arthur/fiber` does **not** exist; not consulted.

---

## 1. Verdict

**Conditional FAIL on `main`.** Of 18 findings, 5 are HIGH and one is
CRITICAL. The Fiber L2 bridge plan's API surface is broadly accurate
(8/8 RPC names match Fiber `rpc/README.md`), but the top-level Myelin
documentation has accumulated three independent representations of the
same `court_checks` number (16 / 20 / 22) and they contradict each other
and the only audited gate number (22, which matches the implementation).
The Fiber plan is a documentation island — `rg -rn 'fiber|Fiber' MYELIN_*.md README.md`
returns zero matches. Position drift is also visible between
`MYELIN_SESSION_L2_PLAN.md` ("CKB-isomorphic finite Cell session L2"),
`MYELIN_USE_CASE_POSITIONING.md` (same phrasing plus committee finality),
`docs/MYELIN_ARCHITECTURE.md` (…plus "benchmark"), and the Fiber plan
("CKB-style finite Cell session L2"). The README is internally consistent
with the `cellscript/exec/state/mempool/consensus/crypto/math/utils` package
list, but it does not link to any `MYELIN_*.md` companion doc, so a
reviewer who reads only the README cannot trace its capability claims to
the audit chain.

The prior audit's `LANE_DOCS.md` findings F-DOC-02 (`court_checks: 16`
stale), F-DOC-03 (no `final-l1-script` role in live script),
F-DOC-04 (`state/README.md` mmap claim), F-DOC-10 (DA-receipt env-var
naming divergence), F-DOC-15 (`session open --app-id --participant`
CLI surface), F-DOC-17 (submission acceptance emphasis drift),
F-DOC-21 (headline phrasing divergence), F-DOC-22 (gate step description
omits readiness aggregation), F-DOC-27/F-DOC-28 (no residual references
to deleted docs) are **all still present on `main`**. They were not
fixed by the commit that introduced the Fiber L2 bridge plan
(`ab1111b`). The new `docs/myelin-fiber-l2-bridge-plan.md` is
substantive and mostly accurate (verified against Fiber source),
but is a stand-alone proposal rather than an integrated extension of
the Myelin protocol surface.

The matrix below enumerates the new findings on `main` plus the
carry-overs from the prior audit, all referencing `main`-as-checked-out.

---

## 2. Findings table

| # | Severity | Component | Finding | File:Line (on `main`) | Doc claim | Code reality |
|---|----------|-----------|---------|----------------------|-----------|--------------|
| F-DOC-M01 | **CRITICAL** | docs | `MYELIN_TEEWORLDS_REPRODUCIBILITY.md` self-contradicts: `court_checks: 16` on line 64, "all 16 checks ok" on line 79, "20/20 checks" on line 176, "20/20 instead of the previous 14/14" on line 179. Four distinct numbers in one doc. | `MYELIN_TEEWORLDS_REPRODUCIBILITY.md:64,79,92,176,179,190` | `court_checks: 16` / `20/20 checks` | The verifier (`cli/src/main.rs:2112-2453`) emits **22** checks per bundle. The acceptance script computes `court_checks = len(checks)` (`scripts/myelin_teeworlds_acceptance.sh:159`) so the actual value is whatever the verifier emits (22). Lines 64/79/92 use 16; lines 176/179/190 use 20. All are stale. |
| F-DOC-M02 | **HIGH** | docs | `MYELIN_USE_CASE_POSITIONING.md` carries two stale `16` references; should be 22 (or `len(checks)`). | `MYELIN_USE_CASE_POSITIONING.md:231, 285` | "Teeworlds acceptance shows 16 court-bundle data-binding checks" / "court-bundle 16 checks" | Same as F-DOC-M01 — verifier emits 22. |
| F-DOC-M03 | **HIGH** | docs | The new `docs/myelin-fiber-l2-bridge-plan.md` does not connect to any other top-level doc; `rg -rn 'fiber\|Fiber' MYELIN_*.md README.md` returns zero matches. | `docs/myelin-fiber-l2-bridge-plan.md:1-425` (full file), `README.md` (no reference) | "Myelin can integrate with Fiber through a bridge/controller layer" | The plan is a self-contained proposal. There is no README link, no MYELIN_*.md backlink, no entry on the public-testnet-runbook cross-references, and no implementation (`tools/myelin-fiber-bridge/` per the doc's "Recommended first module shape" does not exist; `rg myelin-fiber cli/src cellscript/src exec/src consensus/src state/src mempool/src` returns no matches). A reviewer cannot tell whether the plan is endorsed, aspirational, or superseded. |
| F-DOC-M04 | **HIGH** | docs / docs | `MYELIN_PRODUCTION_REHEARSAL_REPORT.md` (123 lines) does not re-state `court_checks: 22` anywhere; it provides no path to reconcile the gate number with the teeworlds and positioning doc numbers (16 / 20). | `MYELIN_PRODUCTION_REHEARSAL_REPORT.md:1-123` (no `court_checks` mention) | (No claim about court_checks.) | The gate number (`MYELIN_PRODUCTION_GATE.md:166` = 22) matches the implementation (`cli/src/main.rs:2112-2453`). The audit chain is broken because the rehearsal report neither confirms nor reconciles the count. |
| F-DOC-M05 | **HIGH** | docs | README headline framing drifts across docs: `MYELIN_SESSION_L2_PLAN.md:7` ("CKB-isomorphic finite Cell session L2"), `MYELIN_USE_CASE_POSITIONING.md:27` (same + committee finality), `docs/MYELIN_ARCHITECTURE.md:8` (…plus "benchmark"), `README.md:3` ("CKB-style isomorphic session runtime"), `docs/myelin-fiber-l2-bridge-plan.md:26` ("CKB-style finite Cell session L2"). | `README.md:3`, `MYELIN_SESSION_L2_PLAN.md:7`, `MYELIN_USE_CASE_POSITIONING.md:27`, `docs/MYELIN_ARCHITECTURE.md:8`, `docs/myelin-fiber-l2-bridge-plan.md:26` | "CKB-isomorphic finite Cell session L2" / "CKB-style isomorphic session runtime" / "CKB-style finite Cell session L2" / "…scheduler, state root, finality, and benchmark" | Same system has five different one-line identities. None are wrong individually, but the absence of a single canonical phrase means cross-cutting claims (e.g. "Myelin is X for Y use case") cannot be re-used safely in marketing or external docs. |
| F-DOC-M06 | **HIGH** | docs | README has zero references to any `MYELIN_*.md` companion doc; `rg -n 'MYELIN' README.md` returns zero matches. The README is therefore a standalone document that names `scripts/myelin_production_gate.sh` / `scripts/myelin_teeworlds_acceptance.sh` / `MYELIN_CKB_SEMANTIC_DEVIATIONS.md` are not linked from anywhere in the README. | `README.md:1-572` (no `MYELIN_*.md` reference) | (No claim — silent omission.) | A reader of README.md only sees the headline framing, the protocol-shape diagram, and the CLI command list. They cannot reach the audit chain (`MYELIN_PRODUCTION_REHEARSAL_REPORT.md`, `MYELIN_PRODUCTION_GATE.md`, the prior swarm audit files) without knowing they exist. The swarm audit also notes this as F-DOC-21 (stylistic divergence) but does not catch that the README is completely de-linked. |
| F-DOC-M07 | **MEDIUM** | docs | `docs/MYELIN_ARCHITECTURE.md:8` describes Myelin as "its own scheduler, state root, finality, and benchmark". The word "benchmark" implies measurement evidence; the architecture doc is structural, not measurement. The same word does not appear in the headline of any other top-level doc, and `MYELIN_USE_CASE_POSITIONING.md:262` explicitly says "Not safe to claim: specific throughput numbers, specific latency numbers". | `docs/MYELIN_ARCHITECTURE.md:8` vs `MYELIN_USE_CASE_POSITIONING.md:261-263` | "scheduler, state root, finality, and benchmark" | The architecture doc headline mixes structural facts ("scheduler, state root, finality") with a measurement claim ("benchmark") that the positioning doc explicitly disclaims. |
| F-DOC-M08 | **MEDIUM** | docs | `MYELIN_PRODUCTION_GATE.md:52` says step 14 "guards against re-introducing the removed Spora / NovaSeal / certifier / website / cellscript_gate.sh / release-note vocabulary" but the actual stale-surface scan at `scripts/myelin_production_gate.sh:1424-1433` checks 8 patterns (adds `editors/vscode-cellscript` and `novaseal_`). | `MYELIN_PRODUCTION_GATE.md:52`, `scripts/myelin_production_gate.sh:1424-1433` | "Spora / NovaSeal / certifier / website / cellscript_gate.sh / release-note" | The script also scans `editors/vscode-cellscript` and the regex pattern `novaseal_`. The doc under-reports the actual guard surface. |
| F-DOC-M09 | **MEDIUM** | docs | `MYELIN_TEEWORLDS_REPRODUCIBILITY.md:3-6` says "Teeworlds repo at `$HOME/RustroverProjects/teeworlds` as an external pressure workload, not as a Spora module" — but `MYELIN_CKB_SEMANTIC_DEVIATIONS.md:21` (D-04) says "Myelin's `script` hash is domain-separated under `myelin:script-hash:v1` and is not the CKB script hash". The two docs do not conflict, but neither explains why `myelin:script-hash:v1` is the chosen domain, while the same `domain-separated hash` pattern shows up in many places (`myelin:celltx-execution-report:state-transition:v1`, `myelin:teeworlds-session-id:v1`, `myelin:ckb-molecule-transaction:v1`, `myelin:single-chunk-challenge-payload:v1`, etc.). No doc enumerates the canonical domain-separation registry. | `MYELIN_CKB_SEMANTIC_DEVIATIONS.md:21`, `cli/src/main.rs:1486-2088, 2289, 2313, 2319` (multiple `myelin:...:v1` literals) | "domain-separated under `myelin:script-hash:v1`" | The canonical registry of `myelin:*:vN` domain separators lives in code only. A reviewer cannot tell which domain is "v1" vs "v2" vs "deprecated" without grepping source. The prior audit's swarm audit flags this as a gap; the main docs do not. |
| F-DOC-M10 | **MEDIUM** | docs | `MYELIN_USE_CASE_POSITIONING.md:84` says "Game sessions (current Teeworlds acceptance is the reference workload)" but the production gate's step 16 says Teeworlds is the workload, not a "session" — the same acceptance gate is run inside the production gate (`scripts/myelin_production_gate.sh:1383-1402`). The positioning doc classifies Teeworlds under "Game sessions" but the architecture doc frames it as a "replay workload" (`docs/MYELIN_ARCHITECTURE.md:551-556`). The classification is consistent but the framing alternates between "session" and "workload" — there is no doc that says which framing wins for external messaging. | `MYELIN_USE_CASE_POSITIONING.md:84`, `docs/MYELIN_ARCHITECTURE.md:551-556`, `scripts/myelin_production_gate.sh:1383-1402` | "Game sessions" / "disputed chunk court bundle" | Same Teeworlds bundle is referenced under three different framings: "game session", "disputed-chunk court bundle", "replay workload". Reviewers and operators can pick whichever framing they prefer; an external reader cannot tell which one the project endorses. |
| F-DOC-M11 | **MEDIUM** | docs | `MYELIN_SESSION_L2_PLAN.md:141` documents `myelin session open --app-id myelin-custom-game-session-v1 --participant alice --participant bob --escrow-cell '<tx_hash_hex>:0:1000:<lock_hash_hex>'`, but `cli/src/main.rs:5575-5645` only exposes `SessionOpenFixtureArgs`. The `SessionOpenArgs` for the descriptor-driven path is not present in the CLI. | `MYELIN_SESSION_L2_PLAN.md:141, 175`, `cli/src/main.rs:5575-5645` | "open creates a session from CLI-supplied participants and escrow-like input Cells" | `grep -n 'SessionOpenArgs\|fn session_open' cli/src/main.rs` confirms only `SessionOpenFixtureArgs` is wired. The README's first "Immediate Evidence Target" example on line 138-144 uses `session open` not `session open-fixture`. README, plan, and CLI disagree on which surface is real. |
| F-DOC-M12 | **MEDIUM** | runbook / scripts | `docs/public-testnet-rehearsal-runbook.md:58-59, 206-207` expects `MYELIN_DA_PROVIDER_PUBKEY_HASH` / `MYELIN_DA_PROVIDER_SIGNATURE` env vars to feed the external signature, but `scripts/myelin_public_testnet_rehearsal_prepare.sh:63, 132` uses `MYELIN_LOCAL_DA_PROVIDER_SECRET_KEY` and signs locally. The two paths produce equivalent receipts but via different env-var names. The runbook does not flag this divergence. | `docs/public-testnet-rehearsal-runbook.md:54-59, 176-209`, `scripts/myelin_public_testnet_rehearsal_prepare.sh:63, 108-133` | Runbook Phase 2 expects external pubkey-hash + signature | `prepare.sh` defaults to a synthetic 32-byte secret key (`hex_repeat 44 32`) and signs locally with `--provider-secret-key`. An operator who runs `prepare.sh` to bootstrap and then runs the runbook Phase 2 verbatim will not have a `MYELIN_DA_PROVIDER_PUBKEY_HASH` / `MYELIN_DA_PROVIDER_SIGNATURE` pair to feed. Carry-over from prior audit F-DOC-10; not fixed. |
| F-DOC-M13 | **MEDIUM** | runbook / scripts | Runbook documents `--verifier-role final-l1-script` and acceptance "readiness_evidence_mode is `live-ckb-carrier` or `final-l1-script`" (`docs/public-testnet-rehearsal-runbook.md:402-409, 482`), but `scripts/myelin_public_testnet_rehearsal_live.sh:94-132` only implements `da-anchor` and `settlement` roles. Any other value exits non-zero. | `docs/public-testnet-rehearsal-runbook.md:402-409,482`, `scripts/myelin_public_testnet_rehearsal_live.sh:94-132` | "set `--verifier-role final-l1-script`" | `role_config` (live.sh:94-132) matches `da-anchor` and `settlement` only. The `--verifier-role` argument is forwarded to the CLI (live.sh:175), so a user setting `--verifier-role final-l1-script` will still submit the carrier verifier (`da-anchor-carrier.cell`) tagged as `final-l1-script`. Carry-over from prior audit F-DOC-03; not fixed. |
| F-DOC-M14 | **MEDIUM** | docs | `docs/MYELIN_ARCHITECTURE.md:8` headline says "its own scheduler, state root, finality, and **benchmark**". The architecture doc does not use the architecture-fit vs production-evidence discipline that `MYELIN_USE_CASE_POSITIONING.md:10-21, 252-264` explicitly maintains. Sections 551-595 mix the two categories. | `docs/MYELIN_ARCHITECTURE.md:8, 551-595, 612-615` vs `MYELIN_USE_CASE_POSITIONING.md:10-21, 252-264` | Architecture doc headline phrasing | The positioning doc explicitly labels "safe to claim" vs "not safe to claim"; the architecture doc has no equivalent labeling. The architecture doc headlines the system with the word "benchmark" but its body makes no benchmark claims. Carry-over from prior audit F-DOC-08; not fixed. |
| F-DOC-M15 | **MEDIUM** | docs | `MYELIN_SESSION_L2_PLAN.md:496` says "Session fixture open/commit/court/verify/DA/DA-anchor-submit-dry-run/settlement-intent/package flow for both consensus modes". The production gate (`MYELIN_PRODUCTION_GATE.md:50`, `scripts/myelin_production_gate.sh:229-285`) runs seven additional verifiers: `verify-submission-context`, `verify-submission-economics`, `verify-submission-inclusion`, `verify-submission-stability`, `verify-submission-finality`, `verify-submission-readiness`, plus the readiness aggregator. The plan omits the readiness aggregation step. | `MYELIN_SESSION_L2_PLAN.md:496`, `MYELIN_PRODUCTION_GATE.md:50` | "fixture open/commit/court/verify/DA/DA-anchor-submit-dry-run/settlement-intent/package flow" | The production gate step 12 also runs the seven additional mock-CKB verifiers and aggregates them into a `production_submission_ready` decision. A reviewer following the plan would expect the gate to stop at `verify-settlement-package`, but it actually runs through `verify-submission-readiness`. Carry-over from prior audit F-DOC-22; not fixed. |
| F-DOC-M16 | **MEDIUM** | docs / cellscript | `cellscript/docs/spec/CELLSCRIPT_OPERATIONAL_SEMANTICS.md:3` says "Status: v0.16 mechanically precise assurance spec" and on line 192 "The conformance tests live in `tests/v0_16.rs`". `ls cellscript/tests/` shows v0_14, v0_16, v0_17, v0_18 test files; the spec doc only covers v0.16. No v0_17 or v0_18 conformance spec exists. | `cellscript/docs/spec/CELLSCRIPT_OPERATIONAL_SEMANTICS.md:3, 192`, `cellscript/tests/` (v0_14.rs, v0_16.rs, v0_17.rs, v0_18.rs) | "Status: v0.16 … tests live in `tests/v0_16.rs`" | v0.17 and v0.18 conformance tests exist (e.g. `cellscript/tests/v0_18.rs:898-925` exercises the four Myelin fixtures). The spec is one version behind. Not introduced by `ab1111b`; carry-over. |
| F-DOC-M17 | **MEDIUM** | docs | `MYELIN_USE_CASE_POSITIONING.md:231` says the "Teeworlds acceptance shows 16 court-bundle data-binding checks" but `MYELIN_TEEWORLDS_REPRODUCIBILITY.md:176-190` (also on `main`) already updated that text to say "20/20 checks". The two docs disagree on the same evidence in the same repo at the same commit. | `MYELIN_USE_CASE_POSITIONING.md:231`, `MYELIN_TEEWORLDS_REPRODUCIBILITY.md:176,179,190` | 16 (positioning) vs 20 (teeworlds repro) | Both are stale relative to the implementation. The teeworlds repro doc's `20/20` claim is closer (one fewer number) than positioning's `16` claim. The disagreement is on the same evidence. |
| F-DOC-M18 | **LOW** | docs | `docs/myelin-fiber-l2-bridge-plan.md:18-22` says "It should not be described as a finished trustless shared custody layer until Myelin's live L1 court, DA publication, deployed scripts, signing, inclusion, and finality path have been exercised on a public CKB network." The plan's "Phase 0: Specification Lock" and "Phase 1: Local RPC Bridge Prototype" do not require any public-CKB exercise; only Phase 3 does. A reader of the plan could mistake Phases 0-2 as already satisfying the disclaimer. | `docs/myelin-fiber-l2-bridge-plan.md:18-22, 219-310` | "It should not be described as a finished trustless shared custody layer" | Phases 0-2 (spec lock, local prototype, payment-bound close) produce local evidence only. The disclaimer is correctly stated but not paired with the phase labels. |
| F-DOC-M19 | **LOW** | docs | `docs/myelin-fiber-l2-bridge-plan.md:71` "Fiber open_channel_with_external_funding" is a documented RPC name; `docs/myelin-fiber-l2-bridge-plan.md:73` says "external wallet/signing policy fills witnesses only" — Fiber's `docs/external-funding.md:9-11` says the same thing, but adds "Do not rebuild the transaction or modify `inputs`, `outputs`, `outputs_data`, or `cell_deps` after it is returned" (4 forbidden modifications). The bridge plan only names the rule informally ("must not rebuild or modify inputs, outputs, outputs data, or cell deps", line 79-80). Same wording but the Fiber doc spells out each forbidden item with a leading capital list; the bridge plan sentence does not capitalise them. | `docs/myelin-fiber-l2-bridge-plan.md:78-80`, `docs/external-funding.md:9-11` | "the signed Fiber funding transaction must preserve the raw transaction structure returned by Fiber. The signer may fill witnesses, but must not rebuild or modify inputs, outputs, outputs data, or cell deps." | Fiber's own doc lists the four forbidden modifications as bold items. The bridge plan's wording is paraphrased. Not a substantive error; informational. |
| F-DOC-M20 | **LOW** | docs | `MYELIN_CKB_SEMANTIC_DEVIATIONS.md:38-49` lists "Deviations that are NOT surfaced today" (D-05 extended-VM profile non-surface, D-03 scheduler-witness present non-surface). No Myelin doc says when these will be surfaced, which future sweep owns them, or whether the projection report should add a `SemanticDeviation::NonCkbStrictSyscallProfile` warning today. | `MYELIN_CKB_SEMANTIC_DEVIATIONS.md:38-49`, `exec/src/projection.rs:78-180` (no such warning today) | "VmSemantics::MyelinExtended … A future sweep could surface" | The "future sweep" is unspecified. No `MYELIN_*.md` doc owns the future-sweep roadmap. |
| F-DOC-M21 | **LOW** | docs | No `CHANGELOG.md`, no `RELEASE*` notes file, no in-tree release-notes tracker. The commit history is the only user-visible change log. | (root) `ls /Users/arthur/RustroverProjects/Myelin/` for `CHANGELOG*`/`RELEASE*` → 0 matches | (no claim — silent omission) | `git log --oneline -10` shows the change history, but no curated human-readable changelog exists. The README has no "Changes" or "Release" section. |
| F-DOC-M22 | **LOW** | docs | `docs/myelin-fiber-l2-bridge-plan.md:396-407` recommends the first module shape `tools/myelin-fiber-bridge/` with submodules `bridge.schema.json`, `commitment_payload.md`, `src/main.rs`, `src/fiber_rpc.rs`, `src/myelin_reports.rs`, `src/binding_store.rs`, `src/expiry_policy.rs`. None of these files exist on `main` (`ls tools/ 2>/dev/null` returns no `tools/` dir at the repo root). The plan does not say this is forward-looking; it says "Recommended first module shape". A reviewer could mistake this for a partially-implemented feature. | `docs/myelin-fiber-l2-bridge-plan.md:394-407`, `(root)` no `tools/` dir | "Recommended first module shape" | The directory tree shown is speculative. The plan correctly uses the word "Recommended" but does not label each leaf as `[planned]` or `[not-yet-implemented]`. |
| F-DOC-M23 | **LOW** | docs | `docs/myelin-fiber-l2-bridge-plan.md:153-178` defines `MyelinSessionFiberBinding` as a JSON schema with a fixed set of fields (17 named fields) and "domain: myelin-fiber-commitment-v1". No canonical JSON schema file (`bridge.schema.json` per the recommendation on line 399) exists on `main`. | `docs/myelin-fiber-l2-bridge-plan.md:153-178, 399` | "The first bridge schema should be explicit and append-only" | Schema is in prose only. A reviewer cannot validate a `MyelinSessionFiberBinding` JSON document against a machine-checkable schema today. The plan's Phase 0 deliverable is "a canonical `MyelinSessionFiberBinding` JSON schema" — not yet on `main`. |
| F-DOC-M24 | **LOW** | docs | `docs/myelin-fiber-l2-bridge-plan.md:412-422` "Final Recommendation" lists the first credible milestone as "one Myelin session, one Fiber externally funded channel, one payment hash, one compact commitment payload, one binding file, and one end-to-end verification report". The bridge plan does not name a target commit/branch where this milestone will be tracked. | `docs/myelin-fiber-l2-bridge-plan.md:411-422` | "The first credible milestone is:" | The milestone has no owner, no date, no in-tree tracker. Phase 0 acceptance is "reviewers can trace every field in the bridge schema to either a Myelin report, a Fiber RPC response, or an external CKB transaction receipt" (line 232-234), which is documentary not executable. |
| F-DOC-M25 | **LOW** | docs | `docs/myelin-fiber-l2-bridge-plan.md:399` "src/fiber_rpc.rs" is named in the recommended module shape. The `crates/fiber-json-types` and `crates/fiber-wasm/src/api.rs` files in the sibling Fiber checkout are an authoritative source for the typed RPC surface; the bridge plan does not link to or cite them. | `docs/myelin-fiber-l2-bridge-plan.md:1-425` (no Fiber source links) | "call Fiber JSON-RPC APIs to open channels, submit funding transactions…" | The Fiber sibling is at `../fiber` and has typed wrappers at `crates/fiber-json-types/src/*.rs` and `crates/fiber-wasm/src/api.rs`. The bridge plan references Fiber RPCs by name without citing these wrapper types, which would be the closest stable source for the typed shape. |
| F-DOC-M26 | **INFO** | docs | The Fiber sibling's `docs/external-funding.md` and `crates/fiber-lib/src/rpc/README.md` confirm the 8 RPC names the bridge plan claims: `open_channel_with_external_funding`, `submit_signed_funding_tx`, `list_channels`, `new_invoice`, `settle_invoice`, `send_payment`, `get_payment`, plus the cross-reference to `connect_peer`. The bridge plan references 6 of these 8 explicitly; `get_payment` and `connect_peer` are not named but are referenced indirectly via "list_channels until the channel is visible" (which implies a `connect_peer` call beforehand) and "recording Fiber payment result" (which implies a `get_payment` call). | `docs/myelin-fiber-l2-bridge-plan.md:67-77, 102-107, 240-251, 270-279`, Fiber `rpc/README.md:25, 28, 29, 43, 47, 49, 50, 55` | "Fiber open_channel_with_external_funding … submit_signed_funding_tx … list_channels … new_invoice … settle_invoice … send_payment" | All 6 named RPCs exist in Fiber `rpc/README.md` with the same name and a return-type that matches what the bridge plan claims. |
| F-DOC-M27 | **INFO** | docs | `docs/myelin-fiber-l2-bridge-plan.md:158-178` `MyelinSessionFiberBinding` includes `myelin_vm_profile: String`, `myelin_consensus_kind: String`, `fiber_peer_pubkey`, `fiber_channel_id`, `fiber_channel_outpoint`, `fiber_funding_tx_hash`, `fiber_payment_hash`, `fiber_payment_preimage_status`, `latest_myelin_chunk_index`, `latest_myelin_state_root`, `latest_myelin_court_bundle_hash`, `latest_myelin_da_manifest_hash`, `latest_myelin_settlement_intent_hash`. Each of these maps to an existing Myelin field: `myelin_vm_profile` = `cli/src/main.rs:4725-4729` (`ckb_vm_profile_label`), `myelin_consensus_kind` = `consensus/src/lib.rs:38-39` (`"static-closed-committee"` / `"tendermint"`), `fiber_peer_pubkey` = `fiber-json-types/src/channel.rs:381` (`Pubkey`), `fiber_channel_id` / `fiber_funding_tx_hash` / `fiber_payment_hash` = `Hash256` (Fiber README types). | `docs/myelin-fiber-l2-bridge-plan.md:158-178`, `cli/src/main.rs:4725-4729`, `consensus/src/lib.rs:38-39`, Fiber `rpc/README.md` type index | `MyelinSessionFiberBinding` field list | Each field has a corresponding source-of-truth in Myelin or Fiber. None are invented. |
| F-DOC-M28 | **INFO** | docs | `docs/myelin-fiber-l2-bridge-plan.md:71` "Fiber open_channel_with_external_funding … returns channel_id and final unsigned funding transaction" matches Fiber `rpc/README.md:412-413` "Returns: channel_id (Hash256), unsigned_funding_tx (Transaction)". The bridge plan's claim is precise. | `docs/myelin-fiber-l2-bridge-plan.md:67-77`, Fiber `rpc/README.md:412-413` | "returns channel_id and final unsigned funding transaction" | Confirmed at Fiber source: `crates/fiber-lib/src/rpc/channel.rs:84` `#[method(name = "open_channel_with_external_funding")]`. |
| F-DOC-M29 | **INFO** | docs | `docs/myelin-fiber-l2-bridge-plan.md:102-107` "participant B creates a Fiber hold invoice with payment_hash H … bridge releases the preimage to settle the Fiber invoice". Fiber's `new_invoice` accepts `payment_hash` (Option<Hash256>) — if set and `payment_preimage` is absent, this is exactly a "hold invoice" (Fiber `rpc/README.md:664-666`). `settle_invoice` accepts `(payment_hash, payment_preimage)` and stores the preimage (Fiber `rpc/README.md:746-752`). The bridge plan's claim matches Fiber semantics. | `docs/myelin-fiber-l2-bridge-plan.md:92-112`, Fiber `rpc/README.md:663-666, 745-752` | "Fiber supports invoice creation with a supplied payment hash and later explicit settlement with the matching preimage" | Confirmed. |
| F-DOC-M30 | **INFO** | docs | `docs/myelin-fiber-l2-bridge-plan.md:120-148` "Fiber payments can carry custom records … session_id, chunk_index, …". Fiber `send_payment` accepts `custom_records: Option<PaymentCustomRecords>` (Fiber `rpc/README.md:800-810`). The example payload shape `domain: "myelin-fiber-commitment-v1", version: u16, session_id: [u8; 32], …` is internal to the Myelin binding and not constrained by Fiber's API. | `docs/myelin-fiber-l2-bridge-plan.md:118-148`, Fiber `rpc/README.md:800-810` | "Fiber payments can carry custom records" | Confirmed. |
| F-DOC-M31 | **INFO** | docs | `MYELIN_CKB_PROJECTION_AUDIT.md:38-40` says `SemanticProfile::MyelinNative` is "Currently the projection layer does not emit this; it is reserved for future use". `exec/src/projection.rs:155` confirms only `CkbCompatible` and `CkbInspiredOnly` are emitted; `MyelinNative` is never assigned. The CLI's `semantic_profile_label` (`cli/src/main.rs:4717-4722`) handles all three but only the first two reach it. The audit is internally consistent. | `MYELIN_CKB_PROJECTION_AUDIT.md:38-40`, `exec/src/projection.rs:155`, `cli/src/main.rs:4717-4722` | "Currently the projection layer does not emit this" | Confirmed by `grep -n 'SemanticProfile::MyelinNative' exec/src` (no matches in projection path; only the `From<CkbProjectionReport> for TeeworldsChunkProjectionReport` and `semantic_profile_label` match arms reference it). |
| F-DOC-M32 | **INFO** | docs | `MYELIN_PRODUCTION_GATE.md:166` `court_checks: 22` matches the implementation (`cli/src/main.rs:2112-2453`, 22 `push_check` calls). The acceptance script computes the value dynamically (`scripts/myelin_teeworlds_acceptance.sh:159`), so the live number is whatever the verifier emits (22). | `MYELIN_PRODUCTION_GATE.md:166`, `cli/src/main.rs:2112-2453` (push_check calls at 2119, 2130, 2140, 2162, 2170, 2178, 2186, 2194, 2225, 2235, 2243, 2251, 2264, 2281, 2300, 2327, 2344, 2373/2406, 2381/2417, 2426, 2434, 2442 = 22 calls), `scripts/myelin_teeworlds_acceptance.sh:159` | `court_checks: 22` | Confirmed. The gate doc is the only top-level doc that agrees with the implementation. |
| F-DOC-M33 | **INFO** | docs | README's package list `cellscript/`, `exec/`, `state/`, `mempool/`, `consensus/`, `crypto/`, `math/`, `utils/` matches the directory tree on `main` (`ls /Users/arthur/RustroverProjects/Myelin/` confirms `cellscript consensus core-utils crypto exec math mempool state utils`). The README does not list `core-utils` (a 9th crate); `core-utils` is mentioned in `MYELIN_SESSION_L2_PLAN.md:34` as a recent split from `myelin-utils`. | `README.md:24-33`, `MYELIN_SESSION_L2_PLAN.md:32-34`, `(root) ls -d` confirms presence of all 9 crates | "cellscript/, exec/, state/, mempool/, consensus/, crypto/, math/, utils/" | All 8 named crates exist. `core-utils` exists but is unmentioned in README. Not a contradiction; informational. |

---

## 3. Per-finding evidence trail

### F-DOC-M01 (CRITICAL): self-contradiction in TEEWORLDS_REPRODUCIBILITY.md

- `MYELIN_TEEWORLDS_REPRODUCIBILITY.md:64` says `court_checks: 16`.
- `MYELIN_TEEWORLDS_REPRODUCIBILITY.md:79` says `verify-court-bundle : valid (all 16 checks ok)`.
- `MYELIN_TEEWORLDS_REPRODUCIBILITY.md:92` says "all 16 checks ok" in property 5 row.
- `MYELIN_TEEWORLDS_REPRODUCIBILITY.md:176` says "well-formed bundle verifies with 20/20 checks pass".
- `MYELIN_TEEWORLDS_REPRODUCIBILITY.md:179` says "now pass at 20/20 instead of the previous 14/14".
- `MYELIN_TEEWORLDS_REPRODUCIBILITY.md:190` says "the bundle verifier passes with 20/20 checks".
- Implementation truth: `verify_teeworlds_court_bundle` at `cli/src/main.rs:2112-2453` calls `push_check` 22 times (counted by `grep -c 'push_check'` filtered by the function). Static path: `chunk-payload-hash`, `molecule-transaction-hash`, `molecule-transaction-length`, `projection-possible`, `projection-profile`, `projection-source-txid`, `projection-raw-tx-hash`, `projection-wtx-hash`, `block-hash-recomputes`, `block-state-root-before-matches`, `block-state-root-after-matches`, `block-scheduler-commitment-matches`, `block-data-commitment-matches`, `evidence-block-hash-matches-canonical-block`, `challenge-payload-hash`, `committee-signature-hashes`, `committee-signer-ids`, `committee-certificate`, `committee-quorum-weight`, `court-verifiable-profile`, `vm-profile`, `ckb-spawn-ipc-not-required` = 22. Tendermint path swaps `committee-certificate`/`committee-quorum-weight` for `tendermint-certificate`/`tendermint-quorum-power` — same total. Script acceptance: `scripts/myelin_teeworlds_acceptance.sh:159` `court_checks: len(checks)` (dynamic). The 16 / 20 / 22 inconsistency is internal to the doc.

### F-DOC-M02 (HIGH): positioning doc stale `16` references

- `MYELIN_USE_CASE_POSITIONING.md:231` says "The Teeworlds acceptance shows 16 court-bundle data-binding checks, 15,139,695 VM cycles, and a single 2162-byte tape chunk".
- `MYELIN_USE_CASE_POSITIONING.md:285` says "court-bundle 16 checks, semantic profile `ckb-compatible`".
- Implementation truth: 22 (see F-DOC-M01). The 16 is the pre-data-binding count; the 20 is post-data-binding pre-`court-verifiable-profile`-`vm-profile`-`ckb-spawn-ipc-not-required` count. Both numbers are historical snapshots that no longer match the verifier.

### F-DOC-M03 (HIGH): Fiber plan is an island doc

- `rg -rn 'fiber|Fiber|FIBER' /Users/arthur/RustroverProjects/Myelin/MYELIN_*.md /Users/arthur/RustroverProjects/Myelin/README.md /Users/arthur/RustroverProjects/Myelin/AGENTS.md` → 0 matches.
- The Fiber plan does not appear in README's contents, nor in any `MYELIN_*.md` cross-reference, nor in the public-testnet-rehearsal-runbook's "Related Documents" (none such section exists).
- The implementation directories `tools/myelin-fiber-bridge/`, `bridge.schema.json`, `commitment_payload.md` named in the plan's "Recommended first module shape" do not exist (`ls /Users/arthur/RustroverProjects/Myelin/tools/` returns no such dir; `rg -n 'fiber_rpc\|binding_store\|expiry_policy' /Users/arthur/RustroverProjects/Myelin/cli/src/ /Users/arthur/RustroverProjects/Myelin/cellscript/src/ /Users/arthur/RustroverProjects/Myelin/exec/src/ /Users/arthur/RustroverProjects/Myelin/state/src/ /Users/arthur/RustroverProjects/Myelin/consensus/src/ /Users/arthur/RustroverProjects/Myelin/mempool/src/` returns 0 matches).

### F-DOC-M04 (HIGH): production rehearsal report does not reconcile court_checks

- `MYELIN_PRODUCTION_REHEARSAL_REPORT.md:1-123` does not contain the string `court_checks`.
- The report's "Evidence Provenance" table on lines 30-50 mentions `court_bundle_hash` for DA manifest, `Teeworlds court-bundle data-binding checks` indirectly via the provenance row "Teeworlds workload", but does not name the count.
- A reviewer reconciling the gate number (`MYELIN_PRODUCTION_GATE.md:166` = 22) with the teeworlds doc numbers (16 / 20) cannot use the rehearsal report as the tie-breaker. The audit chain is broken.

### F-DOC-M05 (HIGH): headline framing drift

| Doc | Headline |
|---|---|
| `README.md:3` | "Myelin is a CKB-style isomorphic session runtime for typed Cell execution and single-chunk L1 adjudication." |
| `MYELIN_SESSION_L2_PLAN.md:7` | "Myelin is a CKB-isomorphic finite Cell session L2." |
| `MYELIN_USE_CASE_POSITIONING.md:27` | "A CKB-isomorphic finite Cell session L2 with deterministic off-chain execution, committee-mediated finality, and a CKB projection path for disputed-chunk court verification." |
| `docs/MYELIN_ARCHITECTURE.md:8` | "Myelin keeps its own scheduler, state root, finality, and benchmark; the CKB-shaped projection and court path is the CKB alignment boundary." |
| `docs/myelin-fiber-l2-bridge-plan.md:26` | "Myelin is a CKB-style finite Cell session L2." |

`CKB-isomorphic` (plan, positioning) vs `CKB-style` (README, fiber plan, architecture) vs `CKB-style isomorphic` (README line 3 — internal inconsistency within README) — three phrasings used across five documents.

### F-DOC-M06 (HIGH): README has zero MYELIN_*.md links

- `grep -n 'MYELIN' /Users/arthur/RustroverProjects/Myelin/README.md` → 0 matches.
- The README names `scripts/myelin_production_gate.sh` and `scripts/myelin_teeworlds_acceptance.sh` (lines 112, 124) but not `MYELIN_PRODUCTION_GATE.md`, `MYELIN_TEEWORLDS_REPRODUCIBILITY.md`, `MYELIN_PRODUCTION_REHEARSAL_REPORT.md`, or any other audit doc.
- A reviewer who lands on README.md has no path to the audit chain short of `git ls-files | grep MYELIN`.

### F-DOC-M07 (MEDIUM): architecture doc "benchmark" headline vs positioning disclaimer

- `docs/MYELIN_ARCHITECTURE.md:8` headline says "Myelin keeps its own scheduler, state root, finality, and benchmark".
- `MYELIN_USE_CASE_POSITIONING.md:260-263` says "Not safe to claim: specific throughput numbers, specific latency numbers".
- The architecture doc headline uses "benchmark" as if it were a structural attribute, not a measurement claim. A reader could infer that the architecture doc carries benchmark data, which it does not.

### F-DOC-M08 (MEDIUM): stale-surface scan vocab under-reported

- `MYELIN_PRODUCTION_GATE.md:52` lists: Spora, NovaSeal, certifier, website, cellscript_gate.sh, release-note (6 items).
- `scripts/myelin_production_gate.sh:1424-1433` actually scans 8 patterns: `Spora`, `spora`, `NovaSeal`, `novaseal`, `certifier`, `certify`, `website/astro`, `website/src`, `editors/vscode-cellscript`, `cellscript_gate.sh`, `novaseal_`, `release[-_ ]note` (the case-insensitive variants count as separate patterns; the unique forbidden items are 7: Spora/spora, NovaSeal/novaseal/novaseal_, certifier/certify, website/astro+website/src, editors/vscode-cellscript, cellscript_gate.sh, release[-_ ]note).
- The doc text "Spora / NovaSeal / certifier / website / cellscript_gate.sh / release-note" matches 6 of the 7 items; `editors/vscode-cellscript` is missing.

### F-DOC-M09 (MEDIUM): no canonical domain-separation registry

- `grep -rn 'b"myelin:' /Users/arthur/RustroverProjects/Myelin/cli/src/main.rs` returns ~30 hits (e.g. lines 2118, 2129, 2290, 2313, 2319, etc.) each declaring a `myelin:*:vN` domain separator inline.
- `MYELIN_CKB_SEMANTIC_DEVIATIONS.md:21` (D-04) names `myelin:script-hash:v1` but does not enumerate the registry.
- No doc collects the full list, the version scheme, or deprecation policy.

### F-DOC-M10 (MEDIUM): Teeworlds framing alternates between "session" and "workload"

- `MYELIN_USE_CASE_POSITIONING.md:84` "Game sessions (current Teeworlds acceptance is the reference workload)".
- `docs/MYELIN_ARCHITECTURE.md:551-556` "Teeworlds court-bundle ... future court path, not a claim that the CKB on-chain court script is finished".
- `scripts/myelin_production_gate.sh:1383-1402` runs Teeworlds inside the production gate as `RUN_TEEWORLDS=1`.

Three valid framings; no canonical doc says which wins externally.

### F-DOC-M11 (MEDIUM): `session open` vs `session open-fixture`

- `MYELIN_SESSION_L2_PLAN.md:141` documents `myelin session open --app-id … --participant alice --participant bob --escrow-cell '<tx_hash_hex>:0:1000:<lock_hash_hex>'`.
- `cli/src/main.rs:5575` exposes only `SessionOpenFixtureArgs`. `grep -n 'fn session_open\|fn handle_session_open\|SessionOpenArgs' /Users/arthur/RustroverProjects/Myelin/cli/src/main.rs` shows no descriptor-driven `SessionOpenArgs` struct.
- `README.md:138-144` documents `session open` with the same flags as the plan.

### F-DOC-M12 (MEDIUM): DA-receipt env-var naming divergence

- Runbook: `MYELIN_DA_PROVIDER_PUBKEY_HASH`, `MYELIN_DA_PROVIDER_SIGNATURE` (`docs/public-testnet-rehearsal-runbook.md:58-59, 206-207`).
- Prepare script: `MYELIN_LOCAL_DA_PROVIDER_SECRET_KEY` (`scripts/myelin_public_testnet_rehearsal_prepare.sh:63`).

### F-DOC-M13 (MEDIUM): `final-l1-script` role not in live script

- Runbook: `docs/public-testnet-rehearsal-runbook.md:402-409, 482` documents `--verifier-role final-l1-script` and acceptance "readiness_evidence_mode is `live-ckb-carrier` or `final-l1-script`".
- Live script: `scripts/myelin_public_testnet_rehearsal_live.sh:94-132` `role_config` case-matches only `da-anchor` and `settlement`. Other values exit non-zero.

### F-DOC-M14 (MEDIUM): architecture doc lacks positioning discipline

- `MYELIN_USE_CASE_POSITIONING.md:10-21` defines "Architecture fit" vs "Production evidence" explicitly.
- `docs/MYELIN_ARCHITECTURE.md` does not adopt the discipline. Sections 551-595 mix the two without labels.

### F-DOC-M15 (MEDIUM): plan's gate step description omits readiness aggregation

- `MYELIN_SESSION_L2_PLAN.md:496` lists 9 commands; the actual gate step 12 (`scripts/myelin_production_gate.sh:229-285`) runs 9 `run_step`s plus 7 additional mock-CKB verifier invocations (context, economics, inclusion, stability, finality, readiness aggregate) per submission × 2 consensus modes × 2 paths (DA anchor / settlement) = 28 mock-RPC rounds, plus readiness aggregation.
- The plan description matches the pre-readiness-aggregation state of the gate.

### F-DOC-M16 (MEDIUM): cellscript spec is one version behind

- `cellscript/docs/spec/CELLSCRIPT_OPERATIONAL_SEMANTICS.md:3` "Status: v0.16 mechanically precise assurance spec".
- `cellscript/docs/spec/CELLSCRIPT_OPERATIONAL_SEMANTICS.md:192` "The conformance tests live in `tests/v0_16.rs`".
- `ls cellscript/tests/` shows `v0_14.rs`, `v0_16.rs`, `v0_17.rs`, `v0_18.rs` exist. No `cellscript/docs/spec/CELLSCRIPT_OPERATIONAL_SEMANTICS-v0.17.md` or `-v0.18.md`.

### F-DOC-M17 (MEDIUM): positioning vs teeworlds disagree on `court_checks`

- `MYELIN_USE_CASE_POSITIONING.md:231` says 16.
- `MYELIN_TEEWORLDS_REPRODUCIBILITY.md:176, 179, 190` say 20/20.
- Both are stale (real number is 22).

### F-DOC-M18–F-DOC-M25 (LOW): plan documentation gaps

- F-DOC-M18: Phases 0-2 disclaimer coupling — see `docs/myelin-fiber-l2-bridge-plan.md:18-22, 219-310`.
- F-DOC-M19: bridge plan paraphrases Fiber's forbidden-modifications list.
- F-DOC-M20: `MYELIN_CKB_SEMANTIC_DEVIATIONS.md:38-49` future-sweep ownership unassigned.
- F-DOC-M21: no `CHANGELOG.md`, no `RELEASE*` file.
- F-DOC-M22: plan's "Recommended first module shape" (`docs/myelin-fiber-l2-bridge-plan.md:396-407`) names a directory tree that doesn't exist.
- F-DOC-M23: no canonical `bridge.schema.json` (line 399 says it should exist in Phase 0).
- F-DOC-M24: Phase 0/1 milestone has no in-tree tracker.
- F-DOC-M25: plan does not cite Fiber's typed wrappers (`crates/fiber-json-types/src/*.rs`, `crates/fiber-wasm/src/api.rs`) which would be the stable source.

### F-DOC-M26–F-DOC-M30 (INFO): Fiber API surface verification

All 6 RPC names referenced in the bridge plan exist in Fiber `rpc/README.md`:
- `open_channel_with_external_funding` → `crates/fiber-lib/src/rpc/channel.rs:83` (`#[method(name = "open_channel_with_external_funding")]`).
- `submit_signed_funding_tx` → `crates/fiber-lib/src/rpc/channel.rs:419` and README line 420.
- `list_channels` → `crates/fiber-lib/src/rpc/channel.rs:59` (returns `Vec<Channel>`; also takes `only_pending: Option<bool>` for visibility checks).
- `new_invoice` → README line 654 (accepts `payment_hash: Option<Hash256>` for hold invoice, line 665).
- `settle_invoice` → README line 741 (takes `payment_hash` + `payment_preimage`).
- `send_payment` → README line 764 (accepts `custom_records: Option<PaymentCustomRecords>`).

### F-DOC-M31 (INFO): semantic_profile emissions

- `SemanticProfile` enum: `MyelinNative`, `CkbCompatible`, `CkbInspiredOnly` (`exec/src/projection.rs:24-31`).
- Emission: only `CkbCompatible` (line 155) and `CkbInspiredOnly` (line 155). `MyelinNative` is never assigned in `project_cell_tx_to_ckb`.
- CLI labeler: `semantic_profile_label` (`cli/src/main.rs:4717-4722`) handles all three but only the first two reach it.
- Audit consistency: `MYELIN_CKB_PROJECTION_AUDIT.md:38-40` correctly says `MyelinNative` is "reserved for future use".

### F-DOC-M32 (INFO): production gate `court_checks: 22`

- `MYELIN_PRODUCTION_GATE.md:166` claims 22.
- Implementation: 22 `push_check` calls in `verify_teeworlds_court_bundle` (`cli/src/main.rs:2112-2453`).
- Script: `scripts/myelin_teeworlds_acceptance.sh:159` computes the value dynamically.
- The gate doc is the only top-level doc that matches the implementation.

### F-DOC-M33 (INFO): README package list

- README lists 8 crates (`cellscript/`, `exec/`, `state/`, `mempool/`, `consensus/`, `crypto/`, `math/`, `utils/`) on lines 24-33.
- `ls -d /Users/arthur/RustroverProjects/Myelin/{cellscript,exec,state,mempool,consensus,crypto,math,utils,core-utils}` confirms all 9 crates exist (`core-utils` is the 9th, unmentioned in README).
- All 8 named crates are present.

---

## 4. Top risks callout

1. **`court_checks` audit-chain inconsistency (CRITICAL/HIGH — F-DOC-M01, M02, M04, M17).** Four different numbers (16 / 20 / 22) live in three top-level docs, and the rehearsal report does not pick a winner. An external reviewer who reads `MYELIN_TEEWORLDS_REPRODUCIBILITY.md` first will see 16, then `MYELIN_PRODUCTION_GATE.md` will say 22, then `MYELIN_USE_CASE_POSITIONING.md` will say 16. This is a credibility hazard for a project whose positioning doc explicitly says positioning discipline must be kept.

2. **Fiber L2 bridge plan is unintegrated (HIGH — F-DOC-M03, M06).** The plan is a stand-alone 425-line proposal with no top-level doc link, no MYELIN_*.md cross-reference, and no implementation directory. A reviewer cannot tell whether the plan is endorsed, aspirational, or superseded. If the project intends the plan as a near-term roadmap, README and the other top-level docs should link to it; if it is a "considered but not committed" record, that label should appear on the first page.

3. **Headline framing drift (HIGH — F-DOC-M05, M07).** Five docs use three different one-line identities for Myelin (`CKB-isomorphic`, `CKB-style`, `CKB-style isomorphic`). The architecture doc headlines "benchmark" while the positioning doc says benchmark numbers are unsafe to claim. External messaging can pick any of these phrasings; the inconsistency is a positioning-discipline violation.

4. **`session open` CLI surface is documented but unimplemented (MEDIUM — F-DOC-M11).** README, plan, and rehearsal runbook all show `--app-id --participant --escrow-cell` flags; only `session open-fixture` is implemented. The first two evidence-targets in README's "Immediate Evidence Targets" list are runnable only via the fixture, not the documented descriptor path.

5. **Removed-content audit-trail (carried from prior audit, MEDIUM — F-DOC-M12, M13).** Two real defects still on `main` that the runbook does not flag: (a) DA-receipt env-var naming divergence between runbook and prepare script, (b) `final-l1-script` verifier-role documented but not implemented. An operator following the runbook verbatim will hit a non-zero exit on the live script.

6. **Stale `celltx-execution-report` spec version (carried from prior audit, MEDIUM — F-DOC-M15, M20).** The architecture doc headline mixes structural facts with a measurement claim ("benchmark"). The "Deviations that are NOT surfaced today" section of `MYELIN_CKB_SEMANTIC_DEVIATIONS.md` has no owner.

7. **No CHANGELOG / release-notes (LOW — F-DOC-M21).** The repo has no curated human-readable changelog. The commit log is the only change log. Acceptable for a single-crate prototype, not acceptable for a public-facing protocol surface that the Fiber bridge plan explicitly pitches as a "near-term L2".

---

## 5. Cross-references to prior audits

The prior `audits/swarm-wholerepo/LANE_DOCS.md` findings map to this lane as follows:

| Prior finding | Lane D status (on `main` @ ab1111b) | Mapping |
|---|---|---|
| F-DOC-01 (CRITICAL — `da-anchor-final.cell` CLI orphan) | unchanged; ab1111b is a doc-only commit | out of scope (cellscript/cli) — see Lane A |
| F-DOC-02 (HIGH — `court_checks: 16` stale) | still stale on `main` (now contradicted by 16/20/22 across three docs) | **F-DOC-M01, M02, M04, M17** |
| F-DOC-03 (HIGH — `final-l1-script` not in live script) | unchanged | **F-DOC-M13** |
| F-DOC-04 (HIGH — `state/README.md` mmap claim) | unchanged | out of scope (state) — see Lane B |
| F-DOC-05 (HIGH — fixture TypedCellDecl not registered) | unchanged | out of scope (cellscript) — see Lane A |
| F-DOC-06 (MEDIUM — architecture doc final-script claim) | unchanged | out of scope (docs) — see Lane A |
| F-DOC-07 (MEDIUM — runbook hardcoded `--current-time-ms 60000`) | unchanged | see Lane A (runbook) |
| F-DOC-08 (MEDIUM — architecture doc lacks positioning discipline) | unchanged | **F-DOC-M07, M14** |
| F-DOC-09 (MEDIUM — rehearsal report does not re-state count) | unchanged | **F-DOC-M04** |
| F-DOC-10 (MEDIUM — DA-receipt env-var divergence) | unchanged | **F-DOC-M12** |
| F-DOC-11 (MEDIUM — deleted audit docs without replacement rationale) | unchanged | **F-DOC-M21** (no CHANGELOG/RELEASE notes) |
| F-DOC-12 (MEDIUM — `myelin_protocol_gate.sh` reference cleaned correctly) | unchanged | INFO |
| F-DOC-13 (MEDIUM — fixture `[u8; 64]` type-args gap) | unchanged | out of scope (cellscript) — see Lane A |
| F-DOC-14 (MEDIUM — runbook `--consensus` flag inconsistency) | unchanged | out of scope (runbook) |
| F-DOC-15 (MEDIUM — `session open` CLI surface documented but unimplemented) | unchanged | **F-DOC-M11** |
| F-DOC-16 (LOW — rehearsal report "unit fixtures" undersells coverage) | unchanged | out of scope (cellscript) — see Lane A |
| F-DOC-17 (LOW — submission acceptance emphasis drift) | unchanged | out of scope (architecture doc) |
| F-DOC-18 (LOW — rehearsal report "Current artefact" rows vague) | unchanged | out of scope (rehearsal report) |
| F-DOC-19 (LOW — IoT acceptance proposed, doc self-labels) | unchanged | out of scope (positioning doc) |
| F-DOC-20 (LOW — fixture fail-closed metadata internals) | unchanged | out of scope (cellscript) — see Lane A |
| F-DOC-21 (LOW — headline divergence) | unchanged | **F-DOC-M05** |
| F-DOC-22 (LOW — plan's gate step description omits readiness aggregation) | unchanged | **F-DOC-M15** |
| F-DOC-23 / F-DOC-24 / F-DOC-25 / F-DOC-26 (INFO) | unchanged | out of scope (cellscript / cli) |
| F-DOC-27 / F-DOC-28 (INFO — no residual references to deleted docs) | unchanged; verified | INFO |
| F-DOC-29 (INFO — fixtures tracked) | unchanged | out of scope (cellscript) |
| F-DOC-30 (INFO — positioning doc disciplined) | unchanged | INFO |
| F-DOC-31 (INFO — fixtures compile) | unchanged | out of scope (cellscript) |

The prior swarm audit also flagged Lane-CLI findings F-CLI-16 (the `final-l1-script` no role-mapping in live script). My lane-D finds the same defect again from the docs side (F-DOC-M13).

Lane-CLI F-CLI-15 (`session open` CLI surface) and Lane-CLI F-CLI-14 (`session commit --chunk-index 7` accepts any chunk index without per-chunk replay evidence) are CLI-side defects that the README's "Immediate Evidence Targets" inherits. See Lane C.

The three prior swarm audit files `MYELIN_SWARM_AUDIT_MEMPOOL_CONSENSUS.md`, `MYELIN_SWARM_AUDIT_STATE_DA.md`, and `MYELIN_SWARM_AUDIT_WHOLEREPO.md` are themselves untracked in `git status` (not in `git ls-files` on `main`). They are present in the working tree but not committed. This is itself a docs-hygiene observation: prior audit deliverables exist as orphan working-tree files. `MYELIN_SWARM_AUDIT_WHOLEREPO.md:6` says they cross-reference `MYELIN_SWARM_AUDIT_MEMPOOL_CONSENSUS.md` and `MYELIN_SWARM_AUDIT_STATE_DA.md`; the three files form a chain but the chain is not committed.

---

## 6. Verification commands re-runnable by reviewer

```bash
# 1. Court-check count (the CRITICAL audit chain issue)
grep -c 'push_check' /Users/arthur/RustroverProjects/Myelin/cli/src/main.rs   # raw count
grep -nE 'fn push_check|fn verify_teeworlds_court_bundle' /Users/arthur/RustroverProjects/Myelin/cli/src/main.rs
# Then count push_check calls inside verify_teeworlds_court_bundle (2112-2453) manually → 22.

# 2. Court_checks claims across docs
grep -n 'court_checks' /Users/arthur/RustroverProjects/Myelin/MYELIN_*.md
# Expected: 16 in TEEWORLDS_REPRODUCIBILITY, 22 in PRODUCTION_GATE. POSITIONING doc has no `court_checks` literal
# but references "16 court-bundle data-binding checks" on lines 231 and 285.

# 3. Fiber plan is an island
grep -rn -i 'fiber' /Users/arthur/RustroverProjects/Myelin/MYELIN_*.md /Users/arthur/RustroverProjects/Myelin/README.md
# Expected: 0 matches.

# 4. README has no MYELIN_*.md links
grep -n 'MYELIN' /Users/arthur/RustroverProjects/Myelin/README.md
# Expected: 0 matches.

# 5. session open fixture vs descriptor
grep -n 'SessionOpenArgs\|SessionOpenFixtureArgs\|fn session_open' /Users/arthur/RustroverProjects/Myelin/cli/src/main.rs
# Expected: only SessionOpenFixtureArgs is wired.

# 6. Fiber RPC names
grep -n 'open_channel_with_external_funding\|submit_signed_funding_tx\|new_invoice\|settle_invoice' \
  /Users/arthur/RustroverProjects/fiber/crates/fiber-lib/src/rpc/README.md
# Expected: all 4 names match.

# 7. Stale-surface scan patterns
grep -nE 'Spora|spora|NovaSeal|novaseal|certifier|certify|website|editors|release' /Users/arthur/RustroverProjects/Myelin/scripts/myelin_production_gate.sh
# Expected: 8 patterns (Spora/spora, NovaSeal/novaseal, certifier/certify, website/astro+src, editors/vscode-cellscript,
# cellscript_gate.sh, novaseal_, release[-_ ]note).

# 8. Cellscript spec version
head -5 /Users/arthur/RustroverProjects/Myelin/cellscript/docs/spec/CELLSCRIPT_OPERATIONAL_SEMANTICS.md
ls /Users/arthur/RustroverProjects/Myelin/cellscript/tests/
# Expected: spec says v0.16; tests include v0_14.rs v0_16.rs v0_17.rs v0_18.rs.

# 9. No CHANGELOG
ls /Users/arthur/RustroverProjects/Myelin/CHANGELOG* /Users/arthur/RustroverProjects/Myelin/RELEASE* 2>&1
# Expected: no matches.

# 10. Prior swarm audit files untracked
git -C /Users/arthur/RustroverProjects/Myelin status --porcelain | grep SWARM_AUDIT
# Expected: 3 lines showing MYELIN_SWARM_AUDIT_*.md as untracked.
```

---

## 7. Summary recommendation (non-binding)

The Fiber L2 bridge plan is a substantive and largely accurate proposal.
The 8 RPC method names, signatures, and behaviors it references are
correct against the sibling Fiber checkout at `/Users/arthur/RustroverProjects/fiber`.
The plan correctly limits its first-version scope (no Fiber internals
import, JSON-RPC boundary only, no DA-via-custom-records, no post-funding
mutation). Its main weakness is that it is unintegrated — no README
link, no MYELIN_*.md cross-reference, and no implementation directory
matches its "Recommended first module shape" on `main`.

The top-level Myelin documentation has more serious problems than the
Fiber plan. The `court_checks` number inconsistency across three docs
(16 / 20 / 22) is a CRITICAL audit-chain defect that the Fiber plan
should not have been committed on top of without first reconciling.
The headline framing drift between the architecture doc, the session L2
plan, the positioning doc, and the README is a positioning-discipline
violation that the project's own `MYELIN_USE_CASE_POSITIONING.md` says
should not happen. The README has zero links to any `MYELIN_*.md`
companion doc, so a reviewer landing on the README cannot reach the
audit chain.

The recommended order of remediation (verifier does not propose code
fixes, only the order of attention) is: (1) reconcile `court_checks`
across `MYELIN_TEEWORLDS_REPRODUCIBILITY.md`, `MYELIN_USE_CASE_POSITIONING.md`,
and `MYELIN_PRODUCTION_GATE.md` to one number (22 is the only
implementation-correct number); (2) add README → MYELIN_*.md links; (3)
link `README.md` and `MYELIN_SESSION_L2_PLAN.md` to
`docs/myelin-fiber-l2-bridge-plan.md` or label the plan as a
"non-binding proposal"; (4) canonicalise the one-line identity of Myelin
across the five docs; (5) either implement `SessionOpenArgs` or update
README, plan, and runbook to use `session open-fixture` exclusively.

`ab1111b` does not fix any of the prior audit's findings; it adds a
new doc on top of an already-stale doc set. The Fiber plan is
informational only on `main`; its accuracy is good, its integration is
missing.