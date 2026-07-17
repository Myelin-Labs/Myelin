# Myelin Swarm Audit — Whole Repo

> Verifier-only review. No fixes proposed. Scope: branch
> `arthur/session-l2-production-evidence-fixes` (vs `main`, 43 files
> changed, ~9,613 insertions, ~2,250 deletions). Cross-references
> `MYELIN_SWARM_AUDIT_MEMPOOL_CONSENSUS.md` and `MYELIN_SWARM_AUDIT_STATE_DA.md`
> (pre-existing audits on this branch, covering `mempool/` and `state/`).
>
> This whole-repo audit covers the rest of the diff: `cli/`,
> `scripts/`, `cellscript/examples/myelin/*.cell`, `docs/`, the
> top-level `MYELIN_*.md` audits, and the underlying `exec/`,
> `crypto/`, `math/` primitives the CLI signs evidence over.
> Workspace compiles clean (`cargo check --workspace --all-targets` ✓).
>
> The audit was split into four parallel reviewer lanes. Findings
> are numbered per lane (`F-CLI-NN`, `F-SCRIPT-NN`, `F-PRIM-NN`,
> `F-DOC-NN`) so each lane keeps its own evidence trail; the per-lane
> deliverable files are kept under `audits/swarm-wholerepo/`.

## Verdict

**Conditional PASS, with four blockers that should gate release.**

The branch adds a coherent production-evidence machinery on top of
the existing CLI: signed external DA receipts, threshold-lock
deployment evidence, authority-signature evidence, court-economics
deployment evidence, operator-custody / runbook documents, final-L1
preflight checks, a public-testnet rehearsal runner, and a 1,785-line
CKB devnet smoke. **Internally the fixture path closes end-to-end** —
every new evidence command has a schema, parser, verifier, and
binding to the manifest/package it claims to extend; every cellscript
fixture compiles under `--target-profile ckb` and `--target-profile
typed-cell`; the templates match the CLI field-by-field; the runbook
is mostly runnable.

**Externally the production-evidence machinery has four serious gaps.**

The four blockers:

1. **Cross-lane orphan-fixture defect (F-DOC-01 / F-CLI-01 / F-SCRIPT-14):**
   `da-anchor-final.cell` and `settlement-final.cell` are wired into
   the cellscript v0_18 test (`cellscript/tests/v0_18.rs:898-925`) and
   into the CKB devnet smoke (`scripts/myelin_ckb_devnet_smoke.sh:113-176,
   700-1500`), but **no CLI helper exists to build a
   `myelin-session-da-anchor-final-v1` or
   `myelin-session-settlement-final-v1` carrier submission report
   outside the smoke**. `carrier_payload_type_args_hex`
   (`cli/src/main.rs:4584-4592`) only knows the two carrier kinds;
   `session_carrier_submission` falls through to a 32-byte default
   for unknown kinds. The runbook's Phase 4 (lines 402-409, 482)
   documents `--verifier-role final-l1-script` and acceptance
   `readiness_evidence_mode is live-ckb-carrier or final-l1-script`,
   but the live script's `role_config`
   (`scripts/myelin_public_testnet_rehearsal_live.sh:94-132`) only
   implements `da-anchor` and `settlement` and forwards
   `--verifier-role` without switching the `.cell` source. **Result:**
   an operator who follows the runbook verbatim with
   `--verifier-role final-l1-script` will submit the carrier verifier
   under the final-script role, and the production gate's step 12
   (`scripts/myelin_production_gate.sh:229-1399`) never exercises
   the final-script fixtures. Three lanes (Docs, CLI, Scripts)
   converge on the same defect.

2. **Production-gate ↔ rehearsal-script signed-receipt disagreement
   (F-CLI-01 ↔ F-SCRIPT-14):** The production gate asserts
   `external_receipt_count == 0`, `external_receipt_checked == False`,
   `external_receipt is None`, `production_ready is False` on the
   `availability` block (`scripts/myelin_production_gate.sh:1198-1204`),
   but the rehearsal scripts (`..._prepare.sh:121-138`,
   `..._live.sh`) produce exactly the opposite shape on the public-
   testnet path. Additionally, the gate's dry-run path asserts
   `operator-custody-policy-missing` and `operator-runbook-missing`
   are present in `end_to_end_production_blockers`
   (`scripts/myelin_production_gate.sh:1085-1089`) but **does NOT
   assert `real-da-availability-guarantee-missing`**, which the CKB
   devnet smoke (`scripts/myelin_ckb_devnet_smoke.sh:1142, 1147`)
   does assert. The CLI was modified in commit `3fda2ab` to recompute
   `da_availability_production_ready` via
   `final_l1_da_availability_preflight_ready`
   (`cli/src/main.rs:9850-9957`), but the production gate does not
   exercise the recompute. **Result:** the gate passes with a
   silently no-op'd recompute; the smoke catches what the gate
   misses.

3. **External-DA-receipt signature domain gap (F-CLI-02):**
   `external_da_receipt_provider_message_hash`
   (`cli/src/main.rs:3019-3037`) covers the typed receipt fields
   (`schema`, `provider`, `namespace`, `payload_hash`,
   `segment_root`, `service_level`, `retention_seconds`,
   `retrieval_endpoint`, `audit_log_commitment`) but **not
   `receipt_id` or `availability_window`**. The doc claims the
   provider's secp256k1 signature covers the SLA fields
   (`MYELIN_PRODUCTION_REHEARSAL_REPORT.md:36`); it covers most of
   them but not all. A provider can sign `payload_hash /
   segment_root` once and then re-emit the receipt with a fresh
   `receipt_id` and a new `availability_window` (claiming a new
   retention label) without invalidating the signature. The
   `receipt_commitment` cross-check covers the typed fields but is
   not signature-covered (`F-CLI-03`).

4. **Type-cell identity collision on the signed-evidence path
   (F-PRIM-01):** `compute_conflict_hash` and `compute_typed_data_hash`
   (`exec/src/celltx/types.rs:299-307, 316-324`) hash `args` and
   `data` concatenated **without length-prefixing**, so
   `(code_hash=H, hash_type=0, args="X", data="")` collides with
   `(code_hash=H, hash_type=0, args="", data="X")`. The contract
   `Script::hash_v1` (`exec/src/celltx/types.rs:1458-1467`) DOES
   length-prefix `args` and is therefore inconsistent with these two
   helpers. The collision is on the type-cell identity path the CLI
   signed evidence bundles traverse (`execution_report.rs:99` and the
   typed-DAG `build_from_typed` in `dag.rs:150`). This is a
   foundational defect, not an audit-introduced artefact: the branch
   did not touch any in-scope source file.

There are several smaller HIGH/MEDIUM defects that should be
addressed before the rehearsal scripts are treated as more than a
paper trail; they are listed in the findings table below.

### Cross-audit cross-reference

This audit complements the two pre-existing swarm audits on the
same branch:

| Pre-existing audit | Scope | Does NOT overlap with this audit |
|---|---|---|
| `MYELIN_SWARM_AUDIT_MEMPOOL_CONSENSUS.md` (377 lines, 19 findings) | `mempool/src/{lib,cellpool,scorer}.rs`, `consensus/src/lib.rs`, `mempool/Cargo.toml`, `consensus/Cargo.toml` | CLI evidence commands; scripts; cellscript fixtures; docs; exec; crypto; math |
| `MYELIN_SWARM_AUDIT_STATE_DA.md` (247 lines, 25 findings) | `state/src/{lib,cell_tree,molecule}.rs`, `state/src/index/{cell_db,script_index,mod}.rs`, `state/src/store/{mod,segment,proof}.rs` | Same exclusions. Note F-DOC-04 confirms `state/README.md:9` mmap claim is still unfixed (carry-over from STATE_DA F-06 / F-09). |

**Cross-audit invariants not yet broken:**

- Determinism of the CLI's evidence-recompute path: every primitive
  on the recompute path is deterministic; no `SystemTime::now`,
  `Instant::now`, `thread_rng`, `getrandom`, or atomic counter is
  called on the path (`cli/src/main.rs:2150-2210` → molecule decode →
  `project_cell_tx_to_ckb` → `compute_txid` → `ckb_raw_transaction_hash_molecule`,
  all blake3/blake2b). `MuHash::data_to_element` is seeded from a
  blake3 hash, not from OS RNG. (F-PRIM-13 still flags CKB-VM cost-
  model version as a future risk.)
- Domain separation across the three consensus hashers,
  `myelin:block:v1`, `myelin:static-committee-signature:v1`,
  `myelin:tendermint-precommit:v1` — confirmed in MEMPOOL_CONSENSUS
  audit. This whole-repo audit confirms the new
  `myelin:external-da-receipt-provider-signature:v2`,
  `myelin:session-da-availability-attestation-message:v1`,
  `myelin:session-settlement-authority-cell-auth:v1` domains are
  similarly registered (`cli/src/main.rs:3019-3037, 3457-3467,
  3672-3676`).
- Adversarial-evidence matrix coverage: the 19 evidence areas in
  `docs/adversarial-evidence-matrix.md` map cleanly to CLI test
  functions (`session_*_rejects_*` and `session_*_accepts_*` tests at
  `cli/src/main.rs:13114-17467`). No checked cell maps to a missing
  implementation; no unchecked cell whose absence would let an attack
  through.

## Lane-by-lane verdict

| Lane | Lane file | Findings | Severity | Verdict |
|---|---|---|---|---|
| **CLI** | `audits/swarm-wholerepo/LANE_CLI.md` | 35 | 1 CRITICAL / 6 HIGH / 17 MEDIUM / 10 LOW / 1 INFO | Conditional PASS for the documented fixture-backed graph; do NOT merge for public-testnet rehearsal use |
| **Scripts** | `audits/swarm-wholerepo/LANE_SCRIPTS.md` | 28 | 1 CRITICAL / 2 HIGH / 13 MEDIUM / 5 LOW / 3 INFO | Conditional PASS, three substantive defects that should block merge to a release branch |
| **Primitives** | `audits/swarm-wholerepo/LANE_PRIMITIVES.md` | 38 | 1 CRITICAL / 8 HIGH / 12 MEDIUM / 7 LOW / 10 INFO | Conditional PASS for the celltx/sighash + projection path with one collision-class defect and one ALWAYS-true warning logic |
| **Docs** | `audits/swarm-wholerepo/LANE_DOCS.md` | 31 | 1 CRITICAL / 4 HIGH / 11 MEDIUM / 15 LOW/INFO + 12 open questions | Conditional PASS; fixture orphan is the headline risk |
| **TOTAL** | | **132** | **4 CRITICAL / 20 HIGH / 53 MEDIUM / 37 LOW / 14 INFO** | |

## Cross-lane defects

Three defects surface in more than one lane and are recorded here
once with cross-references.

### XD-01: Final-script fixture is a CLI orphan (CRITICAL)

**Lanes:** Docs (F-DOC-01), CLI (F-CLI-01), Scripts (F-SCRIPT-14)

`cellscript/examples/myelin/da-anchor-final.cell` and
`cellscript/examples/myelin/settlement-final.cell` are wired into
the cellscript v0_18 test (`cellscript/tests/v0_18.rs:898-925`) and
into the CKB devnet smoke (`scripts/myelin_ckb_devnet_smoke.sh:113-176,
700-1500`). They compile under `--target-profile ckb` and
`--target-profile typed-cell`. The fixture metadata sidecar declares
`identity(field(intent_hash))` for `SettlementFinal` and
`identity(field(da_manifest_hash))` for `DaAnchorFinal`.

But:

- `carrier_payload_type_args_hex` (`cli/src/main.rs:4584-4592`) only
  knows two carrier payload kinds: `myelin-session-da-anchor-carrier-v1`
  and `myelin-session-settlement-carrier-v1`. There is no
  `myelin-session-da-anchor-final-v1` or
  `myelin-session-settlement-final-v1` kind.
- `exec/src/celltx/types.rs` has no `TypedCellDecl` entries for
  `SettlementFinal`, `SettlementCarrier`, `DaAnchorCarrier`, or
  `DaAnchorFinal`. The cellscript `identity(field(...))` annotation
  is a typed-cell-only metadata with no on-chain enforcement today.
- `session_carrier_submission` therefore cannot build a final-script
  carrier transaction outside the smoke. The smoke is the only path
  that exercises the final-script fixtures via
  `verify_final_da_publication` and `verify_final_settlement`.
- The runbook's Phase 4 (lines 402-409) and acceptance (line 482)
  reference `--verifier-role final-l1-script` and
  `readiness_evidence_mode is live-ckb-carrier or final-l1-script`,
  but the live script's `role_config`
  (`scripts/myelin_public_testnet_rehearsal_live.sh:94-132`) only
  implements `da-anchor` and `settlement` and forwards
  `--verifier-role` without switching the `.cell` source.
- The production gate's step 12 (`scripts/myelin_production_gate.sh:229-1399`)
  exercises open/commit/court/DA/settlement but only with the
  carrier-path fixtures; final-script fixtures are smoke-only.

**Severity:** CRITICAL. Three lanes converge. The fixture is
documented and tested but cannot be exercised on a real public
testnet through the documented CLI surface. The runbook points
operators at a CLI path that the CLI does not expose.

**Owner:** CLI to add the two missing carrier payload kinds to
`carrier_payload_type_args_hex` with the correct 64-byte
`data_hash || payload[..32]` layout
(`cellscript/examples/myelin/da-anchor-final.cell:13` expects
`[u8; 64]` type args). Live script to add a `final-l1-script` role
mapping. Runbook is already correct; it documents the intended
surface.

### XD-02: Production-gate vs rehearsal-script signed-receipt disagreement (CRITICAL)

**Lanes:** CLI (F-CLI-01), Scripts (F-SCRIPT-14)

The production gate (`scripts/myelin_production_gate.sh:1198-1204`)
asserts that `external_receipt_count == 0`,
`external_receipt_checked == False`, `external_receipt is None`,
and `production_ready is False` on the `availability` block. The
rehearsal scripts (`scripts/myelin_public_testnet_rehearsal_prepare.sh:121-138,
..._live.sh`) produce exactly the opposite shape on the public-
testnet artifact path. Additionally, the gate's dry-run path
asserts `operator-custody-policy-missing` and `operator-runbook-missing`
in `end_to_end_production_blockers` (line 1085-1089) but **does NOT
assert `real-da-availability-guarantee-missing`**, which the CKB
devnet smoke (line 1142, 1147) does assert. The CLI was modified in
commit `3fda2ab` to recompute `da_availability_production_ready` via
`final_l1_da_availability_preflight_ready`
(`cli/src/main.rs:9850-9957`), but the production gate does not
exercise the recompute.

**Severity:** CRITICAL. The two artifacts disagree on whether the
production-evidence-complete posture can be exercised in the gate's
default run. Either the gate must run `session external-da-receipt`
+ `session da-manifest --external-da-receipt` on a real receipt
and assert a positive `production_ready`, or the readiness claim in
`MYELIN_PRODUCTION_REHEARSAL_REPORT.md:11` cannot be exercised in
the gate.

**Owner:** Scripts to add the gate dry-run path assertion for
`real-da-availability-guarantee-missing`, and to add the rehearsal-
receipt positive-path assertion. Alternatively, CLI to make the
rehearsal scripts produce the same `availability` shape as the
gate's dry-run.

### XD-03: Type-cell identity collision on the CLI evidence path (CRITICAL)

**Lanes:** Primitives (F-PRIM-01, F-PRIM-02)

`compute_conflict_hash` and `compute_typed_data_hash`
(`exec/src/celltx/types.rs:299-307, 316-324`) hash `args` and `data`
concatenated without length-prefixing. `(code_hash=H, hash_type=0,
args="X", data="")` collides with `(code_hash=H, hash_type=0,
args="", data="X")`. The contract `Script::hash_v1`
(`exec/src/celltx/types.rs:1458-1467`) DOES length-prefix `args`
(`&(self.args.len() as u32).to_le_bytes()` at line 1464), but the
two typed-cell helpers in the same file do not. The collision is on
the type-cell identity path the CLI signed evidence bundles
traverse (`execution_report.rs:99` and the typed-DAG
`build_from_typed` in `dag.rs:150`).

**Severity:** CRITICAL. This is a foundation-layer defect, not
introduced by the branch — but it sits on the path the CLI's
production-evidence machinery now uses. The production gate
(`scripts/myelin_production_gate.sh:1079-1093`) consumes the CLI's
recomputed flag but does not exercise a CellTx with a non-empty
`type_script.args` and a non-empty `outputs_data[i]`, so the
collision is latent but unreachable in the current rehearsal path.
It would surface as soon as any consumer puts both an args-bearing
type script and a data-bearing output in the same witness bundle.

**Owner:** Primitives (exec) to length-prefix `args` and `data` in
both helpers, matching `Script::hash_v1`.

## Per-lane finding tables

### CLI (F-CLI-01 .. F-CLI-35)

| # | Severity | Component | Finding | File:Line |
|---|----------|-----------|---------|-----------|
| F-CLI-01 | CRITICAL | scripts/myelin_production_gate.sh | Gate asserts no-receipt shape; rehearsal scripts produce positive receipt shape. (See **XD-02**.) | `scripts/myelin_production_gate.sh:1198-1204` vs `scripts/myelin_public_testnet_rehearsal_prepare.sh:121-138, 133-144` |
| F-CLI-02 | HIGH | cli/src/main.rs | `external_da_receipt_provider_message_hash` does not cover `receipt_id` or `availability_window`. Provider can sign once and re-emit with fresh `receipt_id`. | `cli/src/main.rs:3019-3037` |
| F-CLI-03 | HIGH | cli/src/main.rs | `receipt_hash` (raw bytes) and `receipt_commitment` (typed fields) are not jointly bound into a single signature-covered commitment. | `cli/src/main.rs:2955-2975, 3504-3517` |
| F-CLI-04 | HIGH | cli/src/main.rs | All 4 `CliError` variants collapse into exit code 1; "missing / bad / stale evidence" are not distinguishable from `$?`. | `cli/src/main.rs:976-994` |
| F-CLI-05 | HIGH | cli/src/main.rs | Hard-coded `[0x31u8;32]`, `[0x32u8;32]`, `[0x33u8;32]` fixture keys folded into `availability_commitment`. Real DA committee cannot be substituted without editing source. | `cli/src/main.rs:3440-3456, 3525-3549` |
| F-CLI-06 | HIGH | cli/src/main.rs | Hard-coded `[0x11u8;32]`, `[0x22u8;32]` fixture signer set on `settlement_authority_authentication`; `ckb_lock_args_hash` is the fixture hash. | `cli/src/main.rs:3685-3735` |
| F-CLI-07 | HIGH | cli/src/main.rs | `bare_hex_*_arg` strips `0x` but `parse_hex_32` rejects `0x`; same field can pass one and fail the other depending on call site. | `cli/src/main.rs:3374-3396, 3256-3265, 2818` |
| F-CLI-08 | MEDIUM | cli/src/main.rs | `da_availability_evidence` uses fixed `committee_id = "myelin-replicated-da-committee-testnet-beta-v1"` even when mainnet deployment evidence is in use. | `cli/src/main.rs:3440, 3456, 3525` |
| F-CLI-09 | MEDIUM | cli/src/main.rs | `economics_commitment_algorithm` is overwritten mid-flow; parent report and deployment evidence may report different algorithm strings. | `cli/src/main.rs:4147-4166` |
| F-CLI-10 | MEDIUM | cli/src/main.rs | `production_ready` requires `network == "ckb-mainnet"` AND `ckb_enforceable_checked`; the labels leak: testnet rehearsal cannot satisfy the invariant without claiming mainnet. | `cli/src/main.rs:3787-3875` |
| F-CLI-11 | MEDIUM | cli/src/main.rs | `evidence_commitment_algorithm` strings disagree between parent `SessionCourtEconomicsEvidence` and the deployment child. | `cli/src/main.rs:4274-4287, 4338-4340` |
| F-CLI-12 | MEDIUM | cli/src/main.rs | Rehearsal default `MYELIN_LOCAL_DA_PROVIDER_SECRET_KEY="$(hex_repeat 44 32)"` is the same fixture key as unit tests; no warning emitted. | `cli/src/main.rs:12311`, `scripts/myelin_public_testnet_rehearsal_prepare.sh:63` |
| F-CLI-13 | MEDIUM | cli/src/main.rs | `prepare.sh:62-63` produces `0x$(hex_repeat a5 32)` for `MYELIN_DA_AUDIT_LOG_COMMITMENT`, which `bare_hex_32_arg` will reject. | `cli/src/main.rs:3089-3211, 3374-3396` |
| F-CLI-14 | MEDIUM | cli/src/main.rs | `--signing-request` does not verify `--payload-hash` / `--segment-root` length when `--signing-request` is set. | `cli/src/main.rs:3090-3142` |
| F-CLI-15 | MEDIUM | scripts/..._prepare.sh | Default `MYELIN_LOCAL_DA_PROVIDER_SECRET_KEY` is silent; downstream consumers cannot distinguish test-fixture-signed from externally-signed receipts. | `scripts/myelin_public_testnet_rehearsal_prepare.sh:63, 226-261` |
| F-CLI-16 | MEDIUM | scripts/..._live.sh | `role_config` does not expose `final-l1-script`; runbook-documented flag has no helper. | `scripts/myelin_public_testnet_rehearsal_live.sh:101`, `docs/public-testnet-rehearsal-runbook.md:407-409` |
| F-CLI-17 | MEDIUM | cli/src/main.rs | `verify_session_da_manifest` does `==` on the full availability struct; a hand-edited `availability_commitment_algorithm` fails the same check as a tampered signature. | `cli/src/main.rs:3541-3544, 6370-6440` |
| F-CLI-18 | LOW | cli/src/main.rs | `audit_log_commitment` accepts any 32-byte hex including all-zeros; no content check. (See also STATE_DA F-08.) | `cli/src/main.rs:2856-3003` |
| F-CLI-19 | LOW | cli/src/main.rs | `da_availability_evidence` `attestation_signatures` and `availability_commitment` are in fixed declaration order; no `BTreeMap` canonical ordering. | `cli/src/main.rs:3456, 3491-3495, 6398` |
| F-CLI-20 | LOW | cli/src/main.rs | `da_availability_evidence` has `.expect("DA attestation message hash is valid")` in production code. | `cli/src/main.rs:3469-3474, 3520` |
| F-CLI-21 | LOW | cli/src/main.rs | `availability_commitment_algorithm` string is not asserted in any test; typo would not be caught. | `cli/src/main.rs:3541-3543` |
| F-CLI-22 | LOW | scripts/..._live.sh | Does not check that `MYELIN_REHEARSAL_DIR` contains a `summary.json` from `prepare.sh`. | `scripts/myelin_public_testnet_rehearsal_live.sh:264-280` |
| F-CLI-23 | LOW | cli/src/main.rs | `bare_hex_20_arg` rejects `0x`-prefixed hex; rehearsal scripts construct pubkey-hash without `0x` prefix; runbook does not specify. | `cli/src/main.rs:3256-3265`, `scripts/myelin_public_testnet_rehearsal_prepare.sh:64-65` |
| F-CLI-24 | LOW | cli/src/main.rs | `external_da_receipt_provider_message_hash` is called twice per CLI invocation; no assertion the two calls agree. | `cli/src/main.rs:3098-3142, 3188-3211` |
| F-CLI-25 | LOW | cli/src/main.rs | `da-availability-ready` conflates `production_guarantee_checked` failure and `external_receipt_checked` failure into one flag. | `cli/src/main.rs:3497, 6370-6440` |
| F-CLI-26 | LOW | docs/templates | `external-da-receipt.template.json` omits `signature_scheme`; producer omits it; verifier parses it as part of `SessionExternalDaReceiptEvidence`. | `docs/templates/public-testnet-rehearsal/external-da-receipt.template.json` vs `cli/src/main.rs:3188-3210` |
| F-CLI-27 | LOW | docs/templates | Coincidental alignment of `signing_threshold = 2` between template and fixture signer set; not a defect but worth noting. | `docs/templates/public-testnet-rehearsal/operator-custody-policy.json`, `cli/src/main.rs:9981-9987, 3678` |
| F-CLI-28 | LOW | cli/src/main.rs | Recursive `court_economics_evidence` inside `court_economics_deployment_flags_valid` produces `production_ready = false`; the `==` check at line 4382-4383 compares it to the deployment's `production_ready`. | `cli/src/main.rs:4365-4387` |
| F-CLI-29 | LOW | cli/src/main.rs | `verify_session_submission_readiness` does string-equality on `expected_ckb_tx_hash` rather than recompute; a self-consistent tampered context report passes. | `cli/src/main.rs:9814, 10140-10143` |
| F-CLI-30 | INFO | cli/src/main.rs | `serde_json::from_slice(&receipt_bytes)` fails informatively for non-object roots. | `cli/src/main.rs:2848-2855` |
| F-CLI-31 | INFO | cli/src/main.rs | `CliError::Display` and production-gate `require` use different error vocabularies. | `cli/src/main.rs:984-985`, `scripts/myelin_production_gate.sh:1137` |
| F-CLI-32 | INFO | cli/src/main.rs | `audit_log_commitment` exists in both receipt and DA-anchor carrier; never compared. | `cli/src/main.rs:3397-3408, 4629-4646` |
| F-CLI-33 | INFO | scripts/..._prepare.sh | `hex_repeat` has no `count == 0` guard. | `scripts/myelin_public_testnet_rehearsal_prepare.sh:38-46` |
| F-CLI-34 | INFO | cli/src/main.rs | `secp256k1_pubkey_hash20` truncates blake3 to 20 bytes (~80-bit entropy); adversarial collision is 2^80. | `cli/src/main.rs:4032-4037, 3364, 3471, 3688, 11665` |
| F-CLI-35 | INFO | scripts/production_gate | Gate and prepare.sh use `jq` without `require_cmd jq`. Pre-existing. | `scripts/myelin_production_gate.sh:1061-1119`, `scripts/myelin_public_testnet_rehearsal_prepare.sh:53` |

### Scripts (F-SCRIPT-01 .. F-SCRIPT-28)

| # | Severity | Component | Finding | File:Line |
|---|----------|-----------|---------|-----------|
| F-SCRIPT-01 | CRITICAL | scripts/ckb_devnet_smoke | `all_live_checks_passed` computed but not asserted on exit; composite predicate is not gated. | `scripts/myelin_ckb_devnet_smoke.sh:1749-1782, 1784` |
| F-SCRIPT-02 | HIGH | scripts/ckb_devnet_smoke | 32 `cargo run` invocations without `--locked`; the production gate uses `--locked`. | `scripts/myelin_ckb_devnet_smoke.sh:80-1067` vs `scripts/myelin_production_gate.sh:49-74` |
| F-SCRIPT-03 | HIGH | scripts/teeworlds_acceptance | Python heredoc mixes TAB-indented (lines 156-159) with 4-space-indented (lines 153-155, 160-161) dict entries. Pre-existing. | `scripts/myelin_teeworlds_acceptance.sh:152-162` |
| F-SCRIPT-04 | MEDIUM | scripts/ckb_devnet_smoke | Reused `WORKDIR` is not cleared; trap only kills CKB process, not workdir. | `scripts/myelin_ckb_devnet_smoke.sh:10, 51-57, 219` |
| F-SCRIPT-05 | MEDIUM | scripts/ckb_devnet_smoke | Non-canonical `cd "$(dirname "${BASH_SOURCE[0]}")/.."` pattern (no `--`, no quoted `"${BASH_SOURCE[0]}"`). | `scripts/myelin_ckb_devnet_smoke.sh:4` |
| F-SCRIPT-06 | MEDIUM | scripts/ckb_devnet_smoke | Unquoted bash glob pattern in lock-args prefix check (line 447). | `scripts/myelin_ckb_devnet_smoke.sh:447` |
| F-SCRIPT-07 | MEDIUM | scripts/..._live.sh | Unknown role names only caught at first `cargo run` invocation; partial submission attempts wasted. | `scripts/myelin_public_testnet_rehearsal_live.sh:14, 282` |
| F-SCRIPT-08 | MEDIUM | scripts/ckb_devnet_smoke | `require_cmd` only checks `curl`, `jq`, `python3`; also uses `od`, `tr`, `wc`, `awk`, `seq`, `sed`. BusyBox `wc`/`od` differ. | `scripts/myelin_ckb_devnet_smoke.sh:59-61` |
| F-SCRIPT-09 | MEDIUM | scripts/production_gate | No `require_cmd` at all; `rg`, `python3` not pre-checked. | `scripts/myelin_production_gate.sh` (no `require_cmd` defined) |
| F-SCRIPT-10 | MEDIUM | scripts/ckb_devnet_smoke | Hard-coded `ALWAYS_SUCCESS_CODE_HASH` and `GENESIS_ALWAYS_SUCCESS_DEP_INDEX` not re-derived from deployed `always_success` cell. | `scripts/myelin_ckb_devnet_smoke.sh:12, 13` |
| F-SCRIPT-11 | MEDIUM | scripts/ckb_devnet_smoke | `wait_for_rpc` swallows stderr; 60-second sleep before diagnosing CKB bind failure. | `scripts/myelin_ckb_devnet_smoke.sh:34-43` |
| F-SCRIPT-12 | MEDIUM | docs/templates | 4 `.template.json` files never copied by prepare.sh / live.sh; only `operator-custody-policy.json` and `operator-runbook.json` are copied. | `docs/templates/public-testnet-rehearsal/README.md:14-25`, `scripts/myelin_public_testnet_rehearsal_prepare.sh:84-85` |
| F-SCRIPT-13 | MEDIUM | scripts/..._prepare.sh | `assert_valid()` does not emit an error message; relies on `set -e`. | `scripts/myelin_public_testnet_rehearsal_prepare.sh:48-51` |
| F-SCRIPT-14 | MEDIUM | scripts/production_gate | Gate's dry-run path asserts operator-custody/runbook blockers but NOT `real-da-availability-guarantee-missing`. (See **XD-02**.) | `scripts/myelin_production_gate.sh:1079-1093, 1117-1120` |
| F-SCRIPT-15 | LOW | scripts/production_gate | `RUN_TEEWORLDS=0` and `ALLOW_SKIP_TEEWORLDS=1` allow gate to exit 0 without exercising Teeworlds step. | `scripts/myelin_production_gate.sh:30, 1492-1511` |
| F-SCRIPT-16 | LOW | scripts/..._live.sh | Stale `SUMMARY_PATH.tmp` from prior SIGKILL'd run not cleaned up. | `scripts/myelin_public_testnet_rehearsal_live.sh:224-234` |
| F-SCRIPT-17 | LOW | scripts/ckb_devnet_smoke | `compile_carrier_verifiers` does its own ELF hashing; `cellc ckb-hash --json` schema change would silently emit `null`. | `scripts/myelin_ckb_devnet_smoke.sh:113-204, 347` |
| F-SCRIPT-18 | LOW | scripts/ckb_devnet_smoke | "passed" string at line 1785 is unconditional; `&& echo OK` pattern is misleading when JSON says false. | `scripts/myelin_ckb_devnet_smoke.sh:1782-1784` |
| F-SCRIPT-19 | LOW | scripts/ckb_devnet_smoke | `tip_number="$((tip_hex))"` arithmetic on `null` would emit bash error. | `scripts/myelin_ckb_devnet_smoke.sh:486-487` |
| F-SCRIPT-20 | LOW | scripts/ckb_devnet_smoke | `required_reward_capacity` arithmetic uses `${VAR:-default}` defaults; under-funded devnet fails at line 514, not at the arithmetic. | `scripts/myelin_ckb_devnet_smoke.sh:488` |
| F-SCRIPT-21 | LOW | scripts/..._live.sh | `CKB_TESTNET_LOCK_ARGS` defaults to `"0x"` silently; bad configuration surfaces at CKB submission, not at script start. | `scripts/myelin_public_testnet_rehearsal_live.sh:167, 168` |
| F-SCRIPT-22 | LOW | scripts/production_gate | `thread.join(timeout=5)` is the only guarantee the mock server does not deadlock on teardown. | `scripts/myelin_production_gate.sh:529-1017` |
| F-SCRIPT-23 | LOW | scripts/ckb_devnet_smoke | Trap fires before `CKB_PID` is assigned; safe pattern but does not remove workdir on long-lived CI. | `scripts/myelin_ckb_devnet_smoke.sh:51-57, 481` |
| F-SCRIPT-24 | LOW | docs/templates | `operator-custody-policy.json` and `operator-runbook.json` lack `evidence_commitment` fields while the 4 `.template.json` files have them. Intentional but undocumented. | `docs/templates/public-testnet-rehearsal/operator-custody-policy.json:1-13`, `operator-runbook.json:1-16` |
| F-SCRIPT-25 | LOW | scripts/ckb_devnet_smoke | `mine()` does not validate chain advanced; subsequent `reward_capacity < required` is the indirect check. | `scripts/myelin_ckb_devnet_smoke.sh:45-49` |
| F-SCRIPT-26 | INFO | scripts/production_gate | `myelin_protocol_gate.sh` deletion is clean; zero stale references. | `git log c8008e3` |
| F-SCRIPT-27 | INFO | reports/myelin-teeworlds-repro.json | Deletion is intentional; file in `.gitignore:32`; gate regenerates via `build_myelin_teeworlds_repro.py:141-142`. | `.gitignore:32`, `scripts/myelin_production_gate.sh:1507-1508` |
| F-SCRIPT-28 | INFO | scripts/ckb_devnet_smoke | No `ALLOW_SKIP` env var on the CKB devnet smoke; required by design. | `scripts/myelin_ckb_devnet_smoke.sh` |

### Primitives (F-PRIM-01 .. F-PRIM-38)

| # | Severity | Component | Finding | File:Line |
|---|----------|-----------|---------|-----------|
| F-PRIM-01 | CRITICAL | celltx/types | `compute_conflict_hash` / `compute_typed_data_hash` collide on `(args="X", data="")` vs `(args="", data="X")`. (See **XD-03**.) | `exec/src/celltx/types.rs:299-307, 316-324` |
| F-PRIM-02 | HIGH | celltx/types | `Script::hash_v1` length-prefixes `args`; the two typed-cell helpers do not. Inconsistent within the same file. | `exec/src/celltx/types.rs:1458-1467` vs `299-307, 316-324` |
| F-PRIM-03 | HIGH | projection | `project_cell_tx_to_ckb` hard-codes `actual: CELL_TX_VERSION` (the constant) instead of `tx.version` in the warning branch; the warning field is a constant. | `exec/src/projection.rs:116-120` |
| F-PRIM-04 | HIGH | celltx/sighash | `calc_standard_signature_hash` does not cover `cell_deps` / `header_deps`; `compute_rw_bound_sighash` does via wtxid. Two coverage paths coexist. | `exec/src/celltx/sighash.rs:430-455` vs `266-277` |
| F-PRIM-05 | HIGH | serialization | `pack_number` silently truncates `usize` to `u32`; `as u32` casts not affected by `overflow-checks = true`. | `exec/src/serialization/molecule_compat.rs:1063-1065, 1110-1123` |
| F-PRIM-06 | HIGH | serialization | `StreamingDeserializer::deserialize` trusts on-wire `total_len` for `Vec::with_capacity(total_len)` with no bound beyond `total_len < 4`. | `exec/src/serialization/streaming.rs:99-116` |
| F-PRIM-07 | HIGH | scheduler | `OrderedFloat::cmp` collapses NaN to `Equal`; mirrors MEMPOOL_CONSENSUS F-04. | `exec/src/scheduler/conflict.rs:58-62` |
| F-PRIM-08 | HIGH | math/uint | `UintN::div_rem` panics on zero divisor via `assert_ne!`. Release-panic. | `math/src/uint.rs:319` |
| F-PRIM-09 | MEDIUM | math/uint | `UintN::as_f64` overflows f64 exponent field for `BITS > 1023`; produces +inf or NaN. | `math/src/uint.rs:272-306` |
| F-PRIM-10 | MEDIUM | serialization | `SerializationCache::insert` accepts a key when `max_size = 0` and cache is empty. | `exec/src/serialization/cache.rs:150-161` |
| F-PRIM-11 | MEDIUM | serialization | `SecureEnvelope::length` is `u32`; `data.len() as u32` silently truncates for 4 GB+ envelopes. | `exec/src/serialization/security.rs:83-115` |
| F-PRIM-12 | MEDIUM | vm/verifier | `.expect(...)` in production code at `vm/verifier.rs:157, 174`; currently unreachable but latent panic. | `exec/src/vm/verifier.rs:157, 174` |
| F-PRIM-13 | MEDIUM | vm/verifier | CKB-VM cost model version is not pinned in `Cargo.toml`; future CKB-VM cycle-model change breaks reproducibility. | `exec/src/vm/machine.rs:10` (workspace dep `ckb_vm`) |
| F-PRIM-14 | MEDIUM | vm/verifier | `prepare_group_runtime` rejects every `hash_type != 0`; CKB-projected `Type=1` scripts fail verifier-side. | `exec/src/vm/verifier.rs:620-630` |
| F-PRIM-15 | MEDIUM | vm/verifier | `extract_script_groups` orders lock-then-type; CKB convention is type-then-lock. | `exec/src/vm/verifier.rs:342-412` |
| F-PRIM-16 | MEDIUM | serialization | `split_vm_abi_trailer` is heuristic (1/2^64 false-positive per buffer); `run_script` discards `VmAbiFormat`. | `exec/src/serialization/mod.rs:370-406`, `exec/src/vm/machine.rs:139-141` |
| F-PRIM-17 | MEDIUM | crypto/hashes | `SchnorrSigningHash` uses sha256; all other `CellTx*Hash` hashers use blake3. | `crypto/hashes/src/hashers.rs:88-130` |
| F-PRIM-18 | MEDIUM | crypto/hashes | Vendored keccak `.s` files and `keccak` crate are both dead code; never linked. | `crypto/hashes/build.rs:1-16`, `crypto/hashes/Cargo.toml:15-27` |
| F-PRIM-19 | MEDIUM | crypto/muhash | `U3072::inverse` has unconditional `expect` panic on inputs in `[0, prime)` violation. | `crypto/muhash/src/u3072.rs:156-173` |
| F-PRIM-20 | MEDIUM | celltx/sighash | `compute_txid` and `compute_wtxid` are 60+ duplicated lines; cross-check tested only by `assert_ne!`. | `exec/src/celltx/sighash.rs:100-246` |
| F-PRIM-21 | MEDIUM | celltx/sighash | `dep.dep_type.clone() as u8` is misleading; `DepType` is fieldless. | `exec/src/celltx/sighash.rs:124, 194` |
| F-PRIM-22 | MEDIUM | celltx/sighash | `calc_standard_signature_hash` does not include `cell_deps` / `header_deps`. CKB-compatible; risk if production-evidence layer adopts standard lock instead of wtxid-bound. | `exec/src/celltx/sighash.rs:430-455` |
| F-PRIM-23 | MEDIUM | celltx/sighash | `calc_standard_signature_hash` panics via `&tx.inputs[input_index]` if `input_index >= tx.inputs.len()`. | `exec/src/celltx/sighash.rs:437` |
| F-PRIM-24 | MEDIUM | celltx/types | `CellTx::payload` returns `outputs_data.first()` for coinbase tx with multiple outputs; `standard_payload_hash` binds one output only. | `exec/src/celltx/types.rs:1858-1868` |
| F-PRIM-25 | MEDIUM | scheduler | `ParallelExecutor::execute_sequential` `Err(_)` arm at line 76-77 is unreachable; `ExecutionError::TxCountMismatch` also unreachable. | `exec/src/scheduler/executor.rs:68-79` |
| F-PRIM-26 | MEDIUM | scheduler | `CellDAG::build` never populates `conflict_hash_conflicts`; `build_from_typed` does. Caller must know which to use. | `exec/src/scheduler/dag.rs:88-139, 150-203` |
| F-PRIM-27 | LOW | vm/mod | `VmLimits::default()` is CKB-testnet (4 MB / 10M cycles), not mainnet; CLI does not surface which default. | `exec/src/vm/mod.rs:38-94` |
| F-PRIM-28 | LOW | scheduler | `CellDAG::compute_layers` validation is correct but only validates during layer construction. | `exec/src/scheduler/dag.rs:282-308` |
| F-PRIM-29 | LOW | celltx/types | Encode `Vec<u8>` infallible; decode fallible. Asymmetric API. | `exec/src/celltx/types.rs:914-919` |
| F-PRIM-30 | LOW | exec (root) | `exec/src/vm/README_VM_STATUS.md` deleted in commit `c8008e3`; content (VM incompleteness) is still accurate. No replacement. | `git log c8008e3 -- exec/src/vm/README_VM_STATUS.md` |
| F-PRIM-31 | LOW | crypto/muhash | `U3072::mul` short-circuits for `*self == 1`; doc comment wording is ambiguous. | `crypto/muhash/src/u3072.rs:90-100` |
| F-PRIM-32 | LOW | exec (root) | Workspace `[lints.clippy]` not inherited by any in-scope crate. | `Cargo.toml:202-203` |
| F-PRIM-33 | LOW | math | `UintN::compact_target_bits` `as u32` truncates for `UintN` with BITS > 256. | `math/src/lib.rs:64-103` |
| F-PRIM-34 | INFO | crypto/hashes | `unsafe { str::from_utf8_unchecked }` at `crypto/hashes/src/lib.rs:116`; no SAFETY comment. | `crypto/hashes/src/lib.rs:115-118` |
| F-PRIM-35 | INFO | math | 4 `unsafe { str::from_utf8_unchecked }` in `uint.rs`; one at line 850 lacks SAFETY comment. | `math/src/uint.rs:759, 818, 836, 850` |
| F-PRIM-36 | INFO | math (wasm) | `math/src/wasm.rs` not in CLI hot path. | `math/src/wasm.rs:1+` |
| F-PRIM-37 | INFO | workspace | `[profile.bench]` has `overflow-checks = false`; acceptable for benchmark profile. | `Cargo.toml:186-201` |
| F-PRIM-38 | INFO | exec/scripts (out of scope) | 33+ `unsafe { … }` blocks in RISC-V lock-script fixtures. Listed for completeness. | `exec/src/scripts/fixtures/*.rs` |

### Docs (F-DOC-01 .. F-DOC-31)

| # | Severity | Component | Finding | File:Line |
|---|----------|-----------|---------|-----------|
| F-DOC-01 | CRITICAL | cellscript / cli | `da-anchor-final.cell` / `settlement-final.cell` have no CLI consumer outside the smoke. (See **XD-01**.) | `cellscript/examples/myelin/da-anchor-final.cell:1-56`, `cli/src/main.rs:4584-4592, 8282-8320` |
| F-DOC-02 | HIGH | docs | `MYELIN_TEEWORLDS_REPRODUCIBILITY.md:64` claims `court_checks: 16`; implementation has 22 (`cli/src/main.rs:2112-2453`). **One-line doc fix.** | `MYELIN_TEEWORLDS_REPRODUCIBILITY.md:64`, `scripts/myelin_teeworlds_acceptance.sh:159` |
| F-DOC-03 | HIGH | runbook | Runbook documents `--verifier-role final-l1-script`; live script has no role mapping for it. | `docs/public-testnet-rehearsal-runbook.md:402-409,482`, `scripts/myelin_public_testnet_rehearsal_live.sh:94-132` |
| F-DOC-04 | HIGH | docs | `state/README.md:9` still claims "1GB append-only files with mmap"; references non-existent `kv/`. Carry-over from STATE_DA F-06 / F-09. | `state/README.md:9,42-47` |
| F-DOC-05 | HIGH | cellscript / docs | Cellscript fixtures declare `identity(field(...))` but `exec/src/celltx/types.rs` has no `TypedCellDecl` entries; on-chain enforcement absent. | `cellscript/examples/myelin/*.cell:4`, `exec/src/celltx/types.rs:1-3637` |
| F-DOC-06 | MEDIUM | docs | Architecture doc claims smoke "submits final-script transactions"; only the rejection probe is live, not successful submission. | `docs/MYELIN_ARCHITECTURE.md:551-560`, `scripts/myelin_ckb_devnet_smoke.sh:820-960` |
| F-DOC-07 | MEDIUM | runbook | Runbook hardcodes `--current-time-ms 60000 --challenge-window-ms 60000`; no guidance for real sessions. | `docs/public-testnet-rehearsal-runbook.md:264-266,283-286` |
| F-DOC-08 | MEDIUM | docs | `docs/MYELIN_ARCHITECTURE.md` does not adopt the architecture-fit vs production-evidence discipline that positioning.md maintains. | `MYELIN_USE_CASE_POSITIONING.md:10-21, 252-264`, `docs/MYELIN_ARCHITECTURE.md:551-595,612-615` |
| F-DOC-09 | MEDIUM | docs | `MYELIN_PRODUCTION_REHEARSAL_REPORT.md` does not re-state `court_checks: 22`; audit chain ambiguous. | `MYELIN_PRODUCTION_GATE.md:166`, `MYELIN_PRODUCTION_REHEARSAL_REPORT.md:1-123` |
| F-DOC-10 | MEDIUM | runbook / cli | Runbook's `MYELIN_DA_PROVIDER_PUBKEY_HASH` / `MYELIN_DA_PROVIDER_SIGNATURE` env vars diverge from `prepare.sh`'s `MYELIN_LOCAL_DA_PROVIDER_SECRET_KEY`. | `docs/public-testnet-rehearsal-runbook.md:54-59,176-209`, `scripts/myelin_public_testnet_rehearsal_prepare.sh:63,108-133` |
| F-DOC-11 | MEDIUM | docs | Branch deleted 4 top-level audit docs (`MYELIN_ARTEFACT_CLEANUP.md`, `MYELIN_CLI_AUDIT.md`, `MYELIN_SCHEDULER_AUDIT.md`, `MYELIN_STALE_SURFACE_AUDIT.md`) without a replacement rationale doc. | `git log main..HEAD -- MYELIN_*.md` |
| F-DOC-12 | MEDIUM | docs | `MYELIN_SESSION_L2_PLAN.md` reference to `myelin_protocol_gate.sh` is correctly cleaned; no actual bug. | `git diff main..HEAD -- MYELIN_SESSION_L2_PLAN.md MYELIN_PRODUCTION_GATE.md README.md` |
| F-DOC-13 | MEDIUM | cellscript | If a future `myelin-session-da-anchor-final-v1` helper is added, `carrier_payload_type_args_hex` must produce 64 bytes, not the 32-byte default. | `cellscript/examples/myelin/da-anchor-final.cell:13`, `cli/src/main.rs:4584-4592` |
| F-DOC-14 | MEDIUM | docs | Runbook passes `--consensus static-closed-committee`; `prepare.sh` does not. Inconsistency undocumented. | `docs/public-testnet-rehearsal-runbook.md:135`, `cli/src/main.rs:5575-5645` |
| F-DOC-15 | MEDIUM | docs | Plan documents `myelin session open --app-id --participant --escrow-cell` but CLI only exposes `session open-fixture`. | `MYELIN_SESSION_L2_PLAN.md:141`, `cli/src/main.rs:5575-5645` |
| F-DOC-16 | LOW | docs / cli | Rehearsal report's "unit fixtures" label undersells cellscript test + devnet smoke coverage. | `MYELIN_PRODUCTION_REHEARSAL_REPORT.md:43`, `cellscript/tests/v0_18.rs:652-925` |
| F-DOC-17 | LOW | docs | Three docs describe the same submission acceptance path with different emphasis; L2 plan correctly flags `--accepted-tx-hash` as not satisfying strict live readiness. | `MYELIN_SESSION_L2_PLAN.md:222`, `docs/MYELIN_ARCHITECTURE.md:525-535`, `docs/public-testnet-rehearsal-runbook.md:418-426` |
| F-DOC-18 | LOW | docs | Some "Current artefact" rows in `MYELIN_PRODUCTION_REHEARSAL_REPORT.md` are vague. | `MYELIN_PRODUCTION_REHEARSAL_REPORT.md:30-50` |
| F-DOC-19 | LOW | docs | `MYELIN_USE_CASE_POSITIONING.md:284-303` proposes IoT acceptance plan; doc self-labels as proposed. | `MYELIN_USE_CASE_POSITIONING.md:288-303` |
| F-DOC-20 | LOW | cellscript | `settlement-final.cell:39` and `settlement-carrier.cell:22` check `ckb::cell_type_args_empty(output)`; cellscript metadata sidecar lists `fail_closed_runtime_features` per-feature, not globally. | `cellscript/examples/myelin/settlement-final.cell:39`, `cellscript/examples/myelin/settlement-carrier.cell:22`, `settlement-final.s.meta.json:87-89` |
| F-DOC-21 | LOW | docs | Headline descriptions vary by doc: architecture = "benchmark", session L2 plan = "session L2", positioning = "CKB-isomorphic". Stylistic divergence. | `docs/MYELIN_ARCHITECTURE.md:8`, `MYELIN_USE_CASE_POSITIONING.md:1-50`, `MYELIN_SESSION_L2_PLAN.md:1-25` |
| F-DOC-22 | LOW | docs | Plan's description of gate step omits readiness aggregation. | `MYELIN_SESSION_L2_PLAN.md:496`, `MYELIN_PRODUCTION_GATE.md:50` |
| F-DOC-23 | INFO | cellscript / cli | 4 fixtures correctly referenced by cellscript v0_18 test and both shell scripts. | `cellscript/tests/v0_18.rs:225-228`, `scripts/myelin_ckb_devnet_smoke.sh:114-117` |
| F-DOC-24 | INFO | docs / cli | 6 templates match the CLI's `operator_custody_policy_document_evidence` and `operator_runbook_document_evidence` field-by-field. | `docs/templates/public-testnet-rehearsal/operator-custody-policy.json`, `cli/src/main.rs:9955-10083` |
| F-DOC-25 | INFO | docs / cli | `external-da-receipt.template.json` schema matches CLI's `parse_external_da_receipt`. | `docs/templates/public-testnet-rehearsal/external-da-receipt.template.json:1-16`, `cli/src/main.rs:2856-2975` |
| F-DOC-26 | INFO | docs / cli | `authority-signature-evidence.template.json` schema matches CLI's `SessionAuthoritySignatureEvidence` struct. | `docs/templates/public-testnet-rehearsal/authority-signature-evidence.template.json:1-21`, `cli/src/main.rs:3214-3284` |
| F-DOC-27 | INFO | docs | No residual references to deleted `exec/IMPLEMENTATION_SUMMARY.md` (308 lines) or `exec/src/vm/README_VM_STATUS.md` (190 lines). Deletion is clean. | `rg 'IMPLEMENTATION_SUMMARY\.md\|README_VM_STATUS\.md'` (0 matches) |
| F-DOC-28 | INFO | docs | No residual references to deleted `docs/ARCHITECTURE.md` (90 lines). | `rg 'docs/ARCHITECTURE\.md\|MYELIN_ARCHITECTURE Seed'` (0 matches) |
| F-DOC-29 | INFO | docs | `cellscript/examples/myelin/*.cell` fixtures are tracked; compile outputs untracked. | `git status` |
| F-DOC-30 | INFO | docs | `MYELIN_USE_CASE_POSITIONING.md` is internally disciplined on architecture-fit vs production-evidence. | `MYELIN_USE_CASE_POSITIONING.md:10-21,208-264` |
| F-DOC-31 | INFO | cellscript | 4 fixtures compile cleanly under `--target-profile ckb` and `--target-profile typed-cell`. | `target/debug/cellc examples/myelin/*.cell` |

## Specific verifications

### Cross-lane verifications confirmed

- **Determinism of CLI evidence-recompute path** (`cli/src/main.rs:2150-2210`):
  every primitive on the path is deterministic. No `SystemTime::now`,
  `Instant::now`, `thread_rng`, `getrandom`, or atomic counter is
  called. `MuHash::data_to_element` is seeded from a blake3 hash, not
  from OS RNG.
- **Domain separation across the 3 (existing) + 3 (new) hashers:**
  `myelin:block:v1`, `myelin:static-committee-signature:v1`,
  `myelin:tendermint-precommit:v1` (existing, MEMPOOL_CONSENSUS);
  `myelin:external-da-receipt-provider-signature:v2`,
  `myelin:session-da-availability-attestation-message:v1`,
  `myelin:session-settlement-authority-cell-auth:v1` (new,
  whole-repo). All confirmed in their respective `cli/src/main.rs`
  call sites.
- **Adversarial-evidence matrix** (`docs/adversarial-evidence-matrix.md`):
  19 evidence areas, all mapping cleanly to CLI test functions. No
  missing implementations; no unchecked attack paths.
- **Cleanup is complete:** 9 files / 2,156 lines deleted (per
  `git diff --stat main..HEAD` excluding modifications). Zero stale
  references to `MYELIN_ARTEFACT_CLEANUP.md`, `MYELIN_CLI_AUDIT.md`,
  `MYELIN_SCHEDULER_AUDIT.md`, `MYELIN_STALE_SURFACE_AUDIT.md`,
  `MYELIN_PRODUCTION_GATE.md`, `docs/ARCHITECTURE.md`,
  `exec/IMPLEMENTATION_SUMMARY.md`, `exec/src/vm/README_VM_STATUS.md`,
  `scripts/myelin_protocol_gate.sh`, or `reports/myelin-teeworlds-repro.json`
  anywhere in the tree.
- **Workspace compiles clean:** `cargo check --workspace --all-targets`
  finishes in 8.05s with no errors.

### Cross-lane verifications NOT confirmed

- **Production gate does NOT exercise the new evidence commands**:
  the gate asserts the no-receipt shape (`F-CLI-01`) and does not
  assert `real-da-availability-guarantee-missing` (`F-SCRIPT-14`).
  The CLI was modified in commit `3fda2ab` to recompute
  `da_availability_production_ready`, but the gate's dry-run path
  cannot see whether the recompute happened.
- **The 4 final-script fixtures are not on the production-gate
  path**: only the smoke exercises them (`F-DOC-01`).
- **`MYELIN_TEEWORLDS_REPRODUCIBILITY.md:64` says `court_checks: 16`,
  but the actual verifier emits 22** (`F-DOC-02`). The
  `MYELIN_PRODUCTION_GATE.md:166` and the existing swarm audit's
  scope comments both say 22. **One-line doc fix recommended:**
  change `court_checks: 16` → `court_checks: 22`.

## Per-lane hygiene summaries

| Lane | Hygiene score | Key hygiene drift |
|---|---|---|
| CLI | B+ | Hard-coded fixture keys (F-CLI-05, F-CLI-06); error taxonomy collapses to one exit code (F-CLI-04); 4-byte hex `0x` prefix inconsistency (F-CLI-07, F-CLI-13). |
| Scripts | B- | F-SCRIPT-01 (composite not exit-gated); F-SCRIPT-02 (Cargo.lock non-determinism); F-SCRIPT-08/F-SCRIPT-09 (`require_cmd` gaps); F-SCRIPT-04/F-SCRIPT-23 (workdir cleanup). |
| Primitives | B | 5 `unsafe { str::from_utf8_unchecked }` without SAFETY comments (F-PRIM-34, F-PRIM-35); F-PRIM-01 collision; F-PRIM-08 release-panic on div-by-zero; F-PRIM-13 CKB-VM cost-model version un-pinned. |
| Docs | B+ | F-DOC-01 fixture orphan; F-DOC-02 stale doc count; F-DOC-03 runbook references missing helper; F-DOC-04 README mmap claim carry-over; F-DOC-08 architecture doc lacks positioning discipline. |

### Per-crate hygiene summary (Primitives lane)

| Crate | `unsafe` | `random`/`thread_rng` | Random source | Lints inheritance | Notes |
|---|---|---|---|---|---|
| `exec` | 0 in scope Rust files; 33+ in `scripts/fixtures/*.rs` (out of scope) | None | None | None | Celltx/sighash deterministic; verification wraps CKB-VM. |
| `crypto/hashes` | 1 (`lib.rs:116`, no SAFETY comment) | None | None | None | `keccak` crate dead dep; sha256 only in `SchnorrSigningHash`. |
| `crypto/muhash` | 0 | None (only in tests) | None | None | `expect` panic in `inverse`; `mul` carry-chain asserts (release-panic). |
| `math` | 4 in `uint.rs` (3 with SAFETY, 1 without at line 850) | None | None | None | `assert_ne!` panic in `div_rem`; `as_f64` overflows for BITS > 1023. |

Workspace `[workspace.lints.clippy]` (`Cargo.toml:202-203`) sets only
`empty_docs = "allow"`. None of the in-scope crates declare a `[lints]`
table to inherit.

`clippy.toml` sets `too-many-arguments-threshold = 10`. None of the
public functions in any lane exceed this.

Workspace `[profile.release]` (`Cargo.toml:186-189`) sets
`overflow-checks = true`. The `bench` profile
(`Cargo.toml:191-195`) sets `overflow-checks = false`. The release
profile's `as u32` truncations (`molecule_compat::pack_number`,
`u3072::is_overflow`) are not arithmetic ops and are unaffected by
`overflow-checks`.

## Open questions (consolidated)

### XD-01 / F-DOC-01 / F-CLI-01 / F-SCRIPT-14

Should `myelin-session-da-anchor-final-v1` and
`myelin-session-settlement-final-v1` be added to
`carrier_payload_type_args_hex` (with 64-byte `data_hash || payload[..32]`)
so the final-script fixtures can be exercised on a public testnet
through `session carrier-submission --verifier-role final-l1-script`?
If yes, the live script must also gain a `final-l1-script` role
mapping. If no, the runbook's Phase 4 final-l1-script language should
be retracted. The cellscript fixtures, the cellscript v0_18 test, and
the devnet smoke exercise the fixtures today; only the CLI helper is
missing.

### XD-02 / F-CLI-01 / F-SCRIPT-14

Should the production gate either (a) run `session external-da-receipt`
+ `session da-manifest --external-da-receipt` on the rehearsal receipt
and assert a positive `production_ready`, or (b) retract the
`production-evidence-complete prototype / public-testnet rehearsal
candidate` claim in `MYELIN_PRODUCTION_REHEARSAL_REPORT.md:11`? The
gate's current shape rejects the rehearsal receipt shape. The smoke
asserts `real-da-availability-guarantee-missing`; the gate does not.

### XD-03 / F-PRIM-01

Does the production-evidence layer re-derive `typed_data_hash` for
any CellTx with `type_=Some(...)`? If yes, the collision on
`(args="X", data="")` vs `(args="", data="X")` would let an attacker
substitute one cell for another in a witness bundle. The current
production gate does not exercise a CellTx with both a non-empty
`type_script.args` and a non-empty `outputs_data[i]`, so the collision
is latent but unreachable today.

### F-PRIM-03

Is the `NonCkbTransactionVersion` warning ever consumed by a gate?
If `warning.is_empty()` is used as a pass condition, every valid
Myelin tx would fail. If the gate ignores it, the warning is
documentation noise. The warning field is a constant
(`CELL_TX_VERSION`), not `tx.version`, so the diagnostic is currently
broken regardless.

### F-PRIM-04

Is the CKB-compatible `calc_standard_signature_hash` ever used as a
fallback when `compute_rw_bound_sighash` is unavailable? The two
paths cover different fields (`cell_deps` / `header_deps` are only in
the wtxid-bound path); mixing them across sessions would let a
signer bind cell_deps in one session and not in another.

### F-PRIM-05

Is any Molecule table in production larger than 2^32 bytes? The
`as u32` truncation in `pack_number` is benign for sub-4 GB tables
(confirmed via `state/` store's segments up to 1 GB), but a future
4 GB+ table would silently corrupt the offset header.

### F-PRIM-06

Is `StreamingDeserializer` ever used on untrusted input? The 4 GB
allocation is reachable from any 4-byte input. `SecurityGuard::check_size`
exists but is not wired into the deserializer.

### F-PRIM-13

The CKB-VM cost model is imported via
`use ckb_vm::cost_model::estimate_cycles;` in `machine.rs:10` but the
version is not pinned. A CKB-VM major version bump could change the
cycle totals.

### F-PRIM-14

Is the `hash_type != 0` rejection in `prepare_group_runtime`
deliberate scope cut, or a bug? CKB-projected transactions with a
`Type=1` script cannot pass verifier-side.

### F-PRIM-15

Is the lock-before-type ordering intentional? CKB convention is
type-before-lock so the type-script can mutate state visible to the
lock-script. Under the Myelin verifier, the lock-script sees the
un-type-script-verified cell.

### F-PRIM-16

Is the `VmAbiFormat` returned by `split_vm_abi_trailer` actually used
anywhere? The current call in `run_script` discards it.

### F-PRIM-18

Why is the `keccak` crate declared as a dependency? It is never used
in source. The `.s` files are also unused. The Cargo.toml `[features]
no-asm = ["keccak"]` (line 13) suggests the `keccak` crate was supposed
to be the fallback when ASM is disabled, but the source never reaches
for it.

### F-PRIM-19

Is the `expect` at `u3072.rs:166` reachable in production? The defensive
checks in `inverse` (line 158-165) should prevent the panic for
`data_to_element`-derived inputs, but a bug in `full_reduce` that fails
to reduce `prime` to `0` would surface here.

### F-PRIM-30

Is there a replacement status doc for `exec/src/vm/README_VM_STATUS.md`?
The audit couldn't find one. The README's content is still accurate;
deleting it removed a self-admission of incompleteness.

### F-CLI-19 / F-CLI-20

The `da_availability_evidence` fixture keys are hard-coded and the
order of `attestation_signatures` is fixed by declaration order. There
is no CLI flag to substitute the attester set, and no `BTreeMap`
canonical ordering. A future change to the fixture would silently
change every `availability_commitment` produced by the CLI.

### F-SCRIPT-02 / F-SCRIPT-08 / F-SCRIPT-09

Should the CKB devnet smoke gain `--locked` on every `cargo run` to
match the production gate? Should `require_cmd` be added for all
external tools in every script? The current designs are benign while
`Cargo.lock` is in place and while the host has GNU coreutils and `rg`,
but neither is enforced.

### F-SCRIPT-12

The 4 `.template.json` files in `docs/templates/public-testnet-rehearsal/`
are documented as "shape references only". The README claims "the CLI
should reject unreplaced cryptographic templates" (line 28). Is the
CLI's rejection behaviour actually implemented? If yes, the templates
are documentation only. If no, the templates are documentation with no
enforcement path and should be either removed or wired into the
prepare/live scripts.

### F-DOC-04

`state/README.md:9` still claims "1GB append-only files with mmap"
segment storage and references a `kv/` module that does not exist.
The swarm audit flagged this in F-06 / F-09. The branch rewrote the
README framing but did not fix the underlying code/implementation
description.

### F-DOC-05

The 4 cellscript fixtures declare `identity(field(...))` but
`exec/src/celltx/types.rs` does not register `TypedCellDecl` entries
for `SettlementFinal`, `SettlementCarrier`, `DaAnchorCarrier`, or
`DaAnchorFinal`. Is the on-chain enforcement of `identity(field(...))`
deferred to a future PR, or is the typed-cell metadata path the only
enforcement surface?

### F-DOC-11

The branch deleted four top-level audit docs
(`MYELIN_ARTEFACT_CLEANUP.md`, `MYELIN_CLI_AUDIT.md`,
`MYELIN_SCHEDULER_AUDIT.md`, `MYELIN_STALE_SURFACE_AUDIT.md`) without
leaving a replacement doc. A reviewer auditing "what was deleted and
why" must reconstruct from `git log -p`. Should a single replacement
doc be added?

### F-DOC-15

The session L2 plan (line 141) documents `myelin session open
--app-id ... --participant ... --escrow-cell ...` but the CLI only
exposes `session open-fixture`. Is the descriptor-driven `session
open` a planned future surface, or should the plan be updated to
match the current implementation?

### F-DOC-22

The session L2 plan (line 496) describes the production gate step as
"Session fixture open/commit/court/verify/DA/DA-anchor-submit-dry-run/
settlement-intent/package" while the production gate (line 50) lists
additional readiness aggregation. Should the plan be updated to
reflect the full gate content?

## Recommended actions (audit-side only; not fixes)

These are the four actions the audit consumer should take before
treating the branch as a release candidate. They are not proposed
code changes; they are the questions the branch's own audit chain
must answer.

1. **Resolve the orphan-fixture defect (XD-01).** Either add the two
   missing CLI helpers (`myelin-session-da-anchor-final-v1`,
   `myelin-session-settlement-final-v1`) and the live-script
   `final-l1-script` role mapping, or retract the runbook's Phase 4
   `final-l1-script` language until the helpers exist.
2. **Resolve the production-gate ↔ rehearsal-script disagreement
   (XD-02).** Either the gate runs the rehearsal receipt through the
   new evidence commands and asserts positive `production_ready`, or
   the rehearsal scripts produce the no-receipt shape and the
   rehearsal report retracts the "production-evidence-complete
   prototype" claim. The smoke-side `real-da-availability-guarantee-missing`
   assertion should also be added to the gate's dry-run path.
3. **Decide whether the external-DA-receipt signature covers
   `receipt_id` and `availability_window` (F-CLI-02).** Either add
   both fields to the signature domain
   (`external_da_receipt_provider_message_hash` at `cli/src/main.rs:3019-3037`),
   or update `MYELIN_PRODUCTION_REHEARSAL_REPORT.md:36` and
   `MYELIN_PRODUCTION_GATE.md:241-244` to say "the signature covers
   all typed fields except `receipt_id` and `availability_window`".
4. **Fix the type-cell identity collision (XD-03).** Length-prefix
   `args` and `data` in `compute_conflict_hash` and
   `compute_typed_data_hash` (`exec/src/celltx/types.rs:299-307,
   316-324`) to match `Script::hash_v1` at line 1458-1467. The
   production gate does not currently exercise a CellTx that would
   trip the collision, so the fix can land in a follow-up branch.

### One-line doc fix (no code change)

Change `court_checks : 16` to `court_checks : 22` at
`MYELIN_TEEWORLDS_REPRODUCIBILITY.md:64` (F-DOC-02). This matches
the production gate (`MYELIN_PRODUCTION_GATE.md:166`), the existing
swarm audit (`MYELIN_SWARM_AUDIT_MEMPOOL_CONSENSUS.md`), and the
actual `verify_teeworlds_court_bundle` implementation (22 `push_check`
calls at `cli/src/main.rs:2127-2449`).

## Lane deliverable index

| Lane | File | Lines | Findings |
|---|---|---|---|
| CLI | `audits/swarm-wholerepo/LANE_CLI.md` | 383 | F-CLI-01 .. F-CLI-35 (35 findings) |
| Scripts | `audits/swarm-wholerepo/LANE_SCRIPTS.md` | 453 | F-SCRIPT-01 .. F-SCRIPT-28 (28 findings) |
| Primitives | `audits/swarm-wholerepo/LANE_PRIMITIVES.md` | 516 | F-PRIM-01 .. F-PRIM-38 (38 findings) |
| Docs | `audits/swarm-wholerepo/LANE_DOCS.md` | 389 | F-DOC-01 .. F-DOC-31 (31 findings) |
| **Whole-repo** | `MYELIN_SWARM_AUDIT_WHOLEREPO.md` (this file) | — | 132 findings + 3 cross-lane defects |

## Cross-audit index

| Audit file | Lane | Findings |
|---|---|---|
| `MYELIN_SWARM_AUDIT_MEMPOOL_CONSENSUS.md` | mempool + consensus | 19 (1 CRITICAL, 2 HIGH, 13 MEDIUM, 5 LOW, 3 INFO) |
| `MYELIN_SWARM_AUDIT_STATE_DA.md` | state + DA | 25 (3 CRITICAL, 8 HIGH, 7 MEDIUM, 5 LOW, 2 INFO) |
| `MYELIN_SWARM_AUDIT_WHOLEREPO.md` (this) | whole-repo | 132 (4 CRITICAL, 20 HIGH, 53 MEDIUM, 37 LOW, 14 INFO) + 3 cross-lane defects |
| **Branch total** | | **176 findings + 3 cross-lane defects** |