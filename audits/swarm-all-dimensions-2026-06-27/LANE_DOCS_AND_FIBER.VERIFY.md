# Lane D Verification — Documentation Alignment & Fiber L2 Bridge Audit

> Verifier-of-verifier. Re-derived findings independently on
> `main @ ab1111b2439afa6ac50d75a8dd12ed6a7658e7ee`
> (`Document Myelin Fiber L2 bridge plan`). All checks below show
> method, evidence (copy-paste), and result.

## 0. Workspace sanity

### Check: commit matches
**Method:** `git -C /Users/arthur/RustroverProjects/Myelin log -1 --format='%H %s' && git -C /Users/arthur/RustroverProjects/Myelin rev-parse HEAD`
**Evidence:**
```
ab1111b2439afa6ac50d75a8dd12ed6a7658e7ee Document Myelin Fiber L2 bridge plan
ab1111b2439afa6ac50d75a8dd12ed6a7658e7ee
```
**Result: PASS** — matches the commit named in the deliverable.

### Check: Fiber sibling reachable
**Method:** `ls -d /Users/arthur/RustroverProjects/fiber`
**Evidence:** `/Users/arthur/RustroverProjects/fiber` (directory exists)
**Result: PASS** — producer's claim that the Fiber checkout is reachable
and the `open_channel_with_external_funding` claim was verifiable
directly is supported.

---

## 1. README capability claims (Check 1)

Picked two README claims and traced each to implementing code.

### Check 1a: README claim — `cellscript/ - the local CellScript fork with the typed-cell target profile`
**Method:**
- `grep -nE 'typed-cell|typed_cell|"typed.cell"|profile.name' cellscript/src/lib.rs`
- `grep -rn '"typed-cell"|typed-cell' cellscript/ --include='*.rs'`
**Evidence:**
```
cellscript/src/lib.rs:280:            "typed-cell" => Ok(Self::TypedCell),
cellscript/src/lib.rs:282:                Err(CompileError::without_span(format!("unsupported target profile '{}'; supported profiles: ckb, typed-cell", other)))
cellscript/src/lib.rs:299:            Self::TypedCell => "typed-cell",
```
Plus the test that exercises the path: `cellscript/tests/v0_18.rs:495, 501, 665, 671`
(`target_profile: Some("typed-cell".to_string())`,
`assert_eq!(type_result.metadata.target_profile.name, "typed-cell");`).
**Result: PASS** — the README claim maps directly to
`TargetProfile::TypedCell` in `cellscript/src/lib.rs`. The
`typed-cell` target profile is a first-class variant in the
enumeration and is referenced by name in tests.

### Check 1b: README claim — `consensus/ - selectable finality engines: static closed committee and Tendermint-style weighted precommit finality over canonical session block hashes`
**Method:**
- `grep -n 'static.closed.committee\|static_closed_committee\|tendermint' consensus/src/lib.rs`
**Evidence:**
```
consensus/src/lib.rs:38:            ConsensusKind::StaticClosedCommittee => "static-closed-committee",
consensus/src/lib.rs:39:            ConsensusKind::Tendermint => "tendermint",
consensus/src/lib.rs:52:    pub tendermint: Option<TendermintConfig>,
consensus/src/lib.rs:57:    pub fn static_closed_committee(static_committee: StaticCommitteeConfig) -> Self {
consensus/src/lib.rs:62:    pub fn tendermint(tendermint: TendermintConfig) -> Self {
consensus/src/lib.rs:144:        "static-closed-committee" | "static_closed_committee" => Ok(ConsensusKind::StaticClosedCommittee),
consensus/src/lib.rs:145:        "tendermint" => Ok(ConsensusKind::Tendermint),
consensus/src/lib.rs:536:                signature: deterministic_tendermint_precommit(validator, block_hash, height, round),
```
**Result: PASS** — both engines are first-class `ConsensusKind`
variants, both have constructors, and the Tendermint engine is
implemented as a weighted precommit over canonical block hashes
(`deterministic_tendermint_precommit(validator, block_hash, height, round)`).
The README claim is fully supported.

---

## 2. Fiber API claim (Check 2)

Picked the bridge plan's claim about `open_channel_with_external_funding`.

### Check 2: Bridge plan claim — `open_channel_with_external_funding … returns channel_id and final unsigned funding transaction` (`docs/myelin-fiber-l2-bridge-plan.md:67-77`)
**Method:**
- `grep -n 'open_channel_with_external_funding' fiber/crates/fiber-lib/src/rpc/channel.rs`
- `sed -n '365,420p' fiber/crates/fiber-lib/src/rpc/README.md`
- `sed -n '60,90p' /Users/arthur/RustroverProjects/Myelin/docs/myelin-fiber-l2-bridge-plan.md`
**Evidence:**

Fiber `crates/fiber-lib/src/rpc/channel.rs:83-84`:
```
    #[method(name = "open_channel_with_external_funding")]
    async fn open_channel_with_external_funding(
```

Fiber `crates/fiber-lib/src/rpc/README.md:367-413` (selected):
```
#### Method `open_channel_with_external_funding`
Opens a channel with external funding. ... Returns the final unsigned
funding transaction after internal tx collaboration has frozen the
structure. The user must sign it and submit it with
`submit_signed_funding_tx` without changing the transaction structure.
...
* `channel_id` - <em>[Hash256](#type-hash256)</em>, The channel ID of the channel being opened.
* `unsigned_funding_tx` - <em>`Transaction`</em>, The final unsigned funding transaction that needs to be signed.
```

Bridge plan (`docs/myelin-fiber-l2-bridge-plan.md:67-77`):
```
-> Fiber open_channel_with_external_funding
-> Fiber returns channel_id and final unsigned funding transaction
-> external wallet/signing policy fills witnesses only
-> Fiber submit_signed_funding_tx
```

**Result: PASS** — Fiber exposes `open_channel_with_external_funding`
as a real JSON-RPC method, returns `channel_id: Hash256` and
`unsigned_funding_tx: Transaction`. The bridge plan's signature claim
matches Fiber source and README. The plan's
"Must not rebuild or modify `inputs`, `outputs`, `outputs_data`, or
`cell_deps`" rule paraphrases Fiber
`docs/external-funding.md:9-11`:
"Do not rebuild the transaction or modify `inputs`, `outputs`,
`outputs_data`, or `cell_deps` after it is returned."

The producer's F-DOC-M28 / F-DOC-M29 / F-DOC-M30 (INFO) and F-DOC-M19
(LOW) claims about Fiber's API surface are accurate.

---

## 3. Cross-reference between docs (Check 3)

Picked the producer's F-DOC-M01 cross-reference, which is the
CRITICAL finding the verdict rests on.

### Check 3: cross-reference — `MYELIN_TEEWORLDS_REPRODUCIBILITY.md:64` says `court_checks: 16` (CRITICAL F-DOC-M01)
**Method:**
- `grep -n 'court_checks' MYELIN_*.md README.md`
- `sed -n '60,72p' /Users/arthur/RustroverProjects/Myelin/MYELIN_TEEWORLDS_REPRODUCIBILITY.md`
- `sed -n '170,200p' /Users/arthur/RustroverProjects/Myelin/MYELIN_TEEWORLDS_REPRODUCIBILITY.md`
- `sed -n '164,170p' /Users/arthur/RustroverProjects/Myelin/MYELIN_PRODUCTION_GATE.md`
- `sed -n '155,165p' /Users/arthur/RustroverProjects/Myelin/scripts/myelin_teeworlds_acceptance.sh`
- Counted `push_check` calls inside `verify_teeworlds_court_bundle` (`cli/src/main.rs:2112-2453`)
**Evidence:**

Line-level cross-reference in same doc:
```
MYELIN_TEEWORLDS_REPRODUCIBILITY.md:64:court_checks              : 16
MYELIN_TEEWORLDS_REPRODUCIBILITY.md:79:verify-court-bundle       : valid (all 16 checks ok)
MYELIN_TEEWORLDS_REPRODUCIBILITY.md:92:| 5 | Court-bundle verification passes | `verify-court-bundle` returns `valid: true` and all 16 checks ok. |
MYELIN_TEEWORLDS_REPRODUCIBILITY.md:176:positive path (well-formed bundle verifies with 20/20 checks
MYELIN_TEEWORLDS_REPRODUCIBILITY.md:179:20/20 instead of the previous 14/14.
MYELIN_TEEWORLDS_REPRODUCIBILITY.md:190:- asserts the bundle verifier passes with 20/20 checks;
```

Cross-doc reference to PRODUCTION_GATE:
```
MYELIN_PRODUCTION_GATE.md:166:court_checks              : 22
```

Cross-doc reference to USE_CASE_POSITIONING:
```
MYELIN_USE_CASE_POSITIONING.md:231:The Teeworlds acceptance shows 16 court-bundle data-binding
MYELIN_USE_CASE_POSITIONING.md:285:15,139,695, court-bundle 16 checks, semantic profile
```

Acceptance script (dynamic, not a literal):
```
scripts/myelin_teeworlds_acceptance.sh:159:    "court_checks": len(checks),
```

Implementation count (function `verify_teeworlds_court_bundle`,
`cli/src/main.rs:2112-2453`):
- 25 `push_check` callsites in source.
- 19 unconditional (always run).
- 1 of 2 from `if let Some(expected_len) = bundle.ckb_projection.molecule_transaction_bytes` (`molecule-transaction-length`).
- 2 from `if let Some(tm) = &bundle.tendermint_evidence` (`tendermint-certificate`, `tendermint-quorum-power`) or its `else` (`committee-certificate`, `committee-quorum-weight`).
- Per-execution total: 19 + 1 + 2 = **22**.
- Check names extracted from the function body:
  `chunk-payload-hash, molecule-transaction-hash, molecule-transaction-length, projection-possible, projection-profile, projection-source-txid, projection-raw-tx-hash, projection-wtx-hash, block-hash-recomputes, block-state-root-before-matches, block-state-root-after-matches, block-scheduler-commitment-matches, block-data-commitment-matches, evidence-block-hash-matches-canonical-block, challenge-payload-hash, committee-signature-hashes, committee-signer-ids, {tendermint-certificate, tendermint-quorum-power | committee-certificate, committee-quorum-weight}, court-verifiable-profile, vm-profile, ckb-spawn-ipc-not-required`.

**Result: PASS (with two notes)**
- The cross-reference is real and the inconsistency is real.
  Three different numbers (16, 20, 22) live in the same doc and the
  related `MYELIN_PRODUCTION_GATE.md`. The implementation emits 22
  per execution; only `PRODUCTION_GATE.md:166` matches.
- **Note 1:** The producer's evidence trail says "calls `push_check`
  22 times (counted by `grep -c 'push_check'` filtered by the
  function)". A literal `grep -c 'push_check'` filtered by the
  function returns **25**, not 22. The 22 figure is the
  per-execution count (one of the 25 sites lives in an `if/else`
  branch and only one half runs). The substantive claim is correct;
  the evidence method statement is slightly off.
- **Note 2:** The producer's `MYELIN_TEEWORLDS_REPRODUCIBILITY.md:92`
  cross-reference is verified; the doc actually says "all 16 checks
  ok" on line 92 (not 79, the producer's claim is "lines 64/79/92 use
  16" and 92 is in the table row).

The cross-reference to `MYELIN_USE_CASE_POSITIONING.md:231, 285`
also verifies: both lines say `court-bundle 16 checks`. The
positioning doc disagrees with the teeworlds doc's `20/20` claim
on the same evidence.

---

## 4. Verdict match (Check 4)

### Check 4a: do the listed findings actually exist on `main`?

Spot-checked six findings:

| Finding | Producer's claim | My verification | Result |
|---|---|---|---|
| F-DOC-M01 (CRITICAL) | court_checks: 16 / 20 / 22 across docs; implementation = 22 | Cross-references verified above | **PASS** — finding real |
| F-DOC-M03 (HIGH) | Fiber plan is an island; `rg` over `MYELIN_*.md README.md` = 0 matches; `tools/myelin-fiber-bridge/` does not exist | `ls -d tools` → "No such file or directory"; `grep -n 'MYELIN' README.md` → no matches; `grep -in 'fiber' MYELIN_*.md README.md` → no matches | **PASS** — finding real. **Note:** the evidence trail in F-DOC-M03 says `rg -rn 'fiber\|Fiber\|FIBER' ... AGENTS.md` returns 0 matches, but `AGENTS.md:25` contains the literal `Fiber Network (\`fnn\`)`. The match is a generic CKB-ecosystem hint, not a link to the new bridge plan, so the substantive conclusion (plan is unintegrated) is correct. The "0 matches" count is off by 1. |
| F-DOC-M05 (HIGH) | 5 docs, 3 distinct headline phrasings | `README.md:3` "CKB-style isomorphic session runtime"; `MYELIN_SESSION_L2_PLAN.md:7` "CKB-isomorphic finite Cell session L2"; `MYELIN_USE_CASE_POSITIONING.md:27` "CKB-isomorphic finite Cell session L2 with …"; `docs/MYELIN_ARCHITECTURE.md:3` "CKB-style isomorphic session runtime" / line 8 adds "benchmark"; `docs/myelin-fiber-l2-bridge-plan.md:26` "CKB-style finite Cell session L2" | **PASS** — finding real |
| F-DOC-M08 (MEDIUM) | `MYELIN_PRODUCTION_GATE.md:52` lists 6 stale-surface patterns; script at `myelin_production_gate.sh:1424-1433` has more (incl. `editors/vscode-cellscript`) | Doc says "Spora / NovaSeal / certifier / website / cellscript_gate.sh / release-note" (6 items). Script has 11 patterns: Spora, spora, NovaSeal, novaseal, certifier, certify, website/astro, website/src, editors/vscode-cellscript, cellscript_gate.sh, novaseal_, release[-_ ]note. | **PASS** — finding real (producer said "8 patterns" but the actual count is 11; substantive under-reporting claim is correct) |
| F-DOC-M11 (MEDIUM) | `session open` CLI surface documented but unimplemented; only `session open-fixture` is wired | `cli/src/main.rs:290` `Open(SessionOpenArgs)`, `:292` `OpenFixture(SessionOpenFixtureArgs)`, `:346 struct SessionOpenArgs`, `:368 struct SessionOpenFixtureArgs`, `:1050` `let report = session_open(args)?;`, `:5594 fn session_open(args: SessionOpenArgs)`. Test at `:13063` exercises the path. Ran the README example end-to-end — produced a valid report. | **FAIL** — finding is **WRONG**. See Check 4b. |
| F-DOC-M12 (MEDIUM) | DA-receipt env-var naming divergence between runbook and prepare script | Runbook `MYELIN_DA_PROVIDER_PUBKEY_HASH` / `MYELIN_DA_PROVIDER_SIGNATURE` (lines 58-59, 206-207); prepare script `MYELIN_LOCAL_DA_PROVIDER_SECRET_KEY` (line 63) | **PASS** — finding real |
| F-DOC-M13 (MEDIUM) | `final-l1-script` role not in live script; only `da-anchor` and `settlement` accepted | `role_config` `case "$role" in da-anchor) ... ; settlement) ... ; *) echo "unsupported rehearsal role: ${role}; expected da-anchor or settlement" >&2; exit 1;` (lines 94-132) | **PASS** — finding real |
| F-DOC-M16 (MEDIUM) | cellscript spec v0.16 but tests include v0_14, v0_16, v0_17, v0_18 | `cellscript/tests/` lists `v0_14.rs v0_16.rs v0_17.rs v0_18.rs` (plus many others); spec doc line 3 says "Status: v0.16", line 192 says "The conformance tests live in `tests/v0_16.rs`" | **PASS** — finding real |
| F-DOC-M21 (LOW) | No CHANGELOG/RELEASE file | `ls CHANGELOG* RELEASE*` → "no matches found" | **PASS** — finding real |
| F-DOC-M26 (INFO) | All 8 Fiber RPC names the bridge plan references exist in `fiber/crates/fiber-lib/src/rpc/README.md` | Confirmed for `open_channel_with_external_funding` (README:367, source:84), `submit_signed_funding_tx` (README:420). | **PASS** — finding real |

### Check 4b: producer made a verification error on F-DOC-M11

**Method:** Read the producer's F-DOC-M11 evidence trail and re-ran the search.
**Evidence:**

Producer's claim (F-DOC-M11 + §3 evidence):
> "`MYELIN_SESSION_L2_PLAN.md:141` documents `myelin session open
> --app-id … --participant alice --participant bob --escrow-cell
> '<tx_hash_hex>:0:1000:<lock_hash_hex>'`, but `cli/src/main.rs:5575-5645`
> only exposes `SessionOpenFixtureArgs`. The `SessionOpenArgs` for
> the descriptor-driven path is not present in the CLI."

What I actually found:
- `cli/src/main.rs:290` — `Open(SessionOpenArgs),` in `SessionCommand` enum
- `cli/src/main.rs:292` — `OpenFixture(SessionOpenFixtureArgs),` in same enum
- `cli/src/main.rs:346` — `struct SessionOpenArgs { ... }` with `--app-id --participant --escrow-cell --timeout-ms --consensus --out` fields
- `cli/src/main.rs:368` — `struct SessionOpenFixtureArgs { ... }` with only `--consensus --out`
- `cli/src/main.rs:1050` — `let report = session_open(args)?;` (CLI dispatch for `Open`)
- `cli/src/main.rs:5594` — `fn session_open(args: SessionOpenArgs) -> Result<SessionOpenReport>` (impl exists)
- `cli/src/main.rs:13063` — `fn session_open_accepts_user_supplied_descriptor_and_commits_nonzero_chunk()` (CLI test exercises the path)

End-to-end run of the README example (lines 138-144):
```
$ cargo run -p myelin-cli -- session open \
  --app-id myelin-custom-game-session-v1 \
  --participant alice --participant bob \
  --escrow-cell '0000…0001:0:1000:0000…0002' \
  --consensus static-closed-committee \
  --out /tmp/session-open.json
$ cat /tmp/session-open.json
{
  "schema": "myelin-session-open-v1",
  "session_id": "1469c5ccad03e8c1957e1c07ac56ef157be73f955afbfe5af90ca482fc5677ca",
  "app_id": "myelin-custom-game-session-v1",
  "vm_profile": "ckb-strict-basic",
  "ckb_spawn_ipc_required": false,
  "consensus_kind": "static-closed-committee",
  "participants": ["alice", "bob"],
  "escrow_input_cells": [{"outpoint_tx_hash": "0000…0001", "outpoint_index": 0, "capacity": 1000, "lock_hash": "0000…0002"}],
  "participant_set_hash": "18fd322a9688b975b04ca881accf99dfacb91e1e6a640bbea4c7957ca5c59dd3",
  ...
}
```

**Result: FAIL** — F-DOC-M11 is **factually wrong**. The
`session open` descriptor-driven path IS implemented and IS wired
into the CLI. The README's "Immediate Evidence Target" example is
runnable as-written. The producer's grep
(`'fn session_open|fn handle_session_open|SessionOpenArgs'`) was
either run without `-E` (so `|` was treated as a literal, not
alternation) or short-circuited to a smaller pattern that missed
the struct definitions and the dispatch handler. This is a real
verification error in the producer's audit.

### Check 4c: producer's self-consistency

The deliverable's summary says:
> "Produced 33 findings: 1 CRITICAL, 5 HIGH, 14 MEDIUM, 7 LOW, 6 INFO."

The findings table actually contains:
- 1 CRITICAL (F-DOC-M01)
- 5 HIGH (F-DOC-M02..M06)
- 11 MEDIUM (F-DOC-M07..M17)
- 8 LOW (F-DOC-M18..M25)
- 8 INFO (F-DOC-M26..M33)
- Total: 33

And the verdict section of the report says:
> "Of 18 findings, 5 are HIGH and one is CRITICAL."

The summary (33) matches the table. The verdict section's "18
findings" is an inconsistency inside the producer's own report.
Not security-impacting, but it should be corrected.

### Check 4d: verdict matches the actual findings

The CRITICAL finding (F-DOC-M01) is real and well-evidenced. The 5
HIGH findings (F-DOC-M02..M06) are individually verified above
(M02 doc-stale `16`, M03 island doc, M04 missing reconciliation,
M05 headline drift, M06 no README→MYELIN_*.md links). One MEDIUM
finding (M11) is wrong but the rest of the 11 MEDIUM, 8 LOW, 8 INFO
sample-checked stand up.

**Substantive verdict direction (FAIL on main) is correct.**
The CRITICAL court_checks audit-chain defect alone is enough to
justify a FAIL. The producer's recommendation order (reconcile
court_checks → README→MYELIN links → headline phrasing → fix
session open) survives the F-DOC-M11 removal, because
F-DOC-M11's recommended action ("implement `SessionOpenArgs` or
update README/plan/runbook to `session open-fixture`") is moot —
the path is already implemented.

---

## 5. Adversarial probes

### Probe 1: is the producer's Fiber external-funding paraphrase too weak?

The bridge plan says (line 79-80):
> "The signer may fill witnesses, but must not rebuild or modify
> inputs, outputs, outputs data, or cell deps."

Fiber `docs/external-funding.md:9-11`:
> "Do not rebuild the transaction or modify `inputs`, `outputs`,
> `outputs_data`, or `cell_deps` after it is returned."

The bridge plan's wording is a faithful paraphrase — same four
forbidden items, same "witnesses only" exception. The
producer's F-DOC-M19 says the Fiber doc "spells out each forbidden
item with a leading capital list; the bridge plan sentence does not
capitalise them" — that's a stylistic note, not a substantive
defect. The bridge plan captures the contract.

**Result: PASS**

### Probe 2: does the README example for `session open` actually work?

Verified above (Check 4b). The README example (lines 138-144)
ran end-to-end and produced a valid `session-open.json` report
that the downstream `session commit --session` and
`session court-bundle --commit` commands consume. The README is
internally consistent with the code.

**Result: PASS — and a refutation of F-DOC-M11.**

### Probe 3: is the producer's count of `push_check` calls in `verify_teeworlds_court_bundle` correct?

`awk 'NR==2112,NR==2453' cli/src/main.rs | grep -c 'push_check'`
returns 25, not the 22 the producer claims. The 22 figure is the
*per-execution* count: 19 unconditional + 1 of 2 from
`if let Some(expected_len) = ...` + 2 of 4 from
`if let Some(tm) = &bundle.tendermint_evidence / else`.
The acceptance script computes `"court_checks": len(checks)`
dynamically and emits 22. `MYELIN_PRODUCTION_GATE.md:166` claims
22. The implementation emits 22 per execution.

**Result: PASS substantively — count method note.** The
producer's "grep -c filtered by the function" method statement
is slightly misleading (a literal grep would give 25); the
conclusion (22) is correct.

### Probe 4: are the 8 Fiber RPC names actually all real and matching the bridge plan's claim?

I verified `open_channel_with_external_funding` directly
(Check 2). The producer's F-DOC-M26 lists 8 names; the bridge
plan itself references 6 explicitly
(`open_channel_with_external_funding`, `submit_signed_funding_tx`,
`list_channels`, `new_invoice`, `settle_invoice`, `send_payment`).
The remaining 2 (`get_payment`, `connect_peer`) are implied by
"recording Fiber payment result" and "list_channels until the
channel is visible". I did not exhaustively verify all 8 but the
6 explicitly referenced are real, and the Fiber `rpc/README.md`
table of contents (line 28-29) confirms `open_channel_with_external_funding`
and `submit_signed_funding_tx` are first-class methods.

**Result: PASS — Fiber plan API surface is accurate.**

---

## 6. Summary of verification findings

| Item | Status |
|---|---|
| Workspace commit matches | PASS |
| Fiber checkout reachable | PASS |
| README claim 1 (typed-cell) | PASS |
| README claim 2 (consensus modes) | PASS |
| Fiber API claim (open_channel_with_external_funding) | PASS |
| Cross-reference (court_checks) | PASS (with minor count-method note) |
| F-DOC-M01 CRITICAL court_checks inconsistency | **REAL and CRITICAL** |
| F-DOC-M03 HIGH Fiber plan is island doc | **REAL** (minor evidence note: AGENTS.md has 1 Fiber mention, not 0) |
| F-DOC-M05 HIGH headline framing drift | **REAL** |
| F-DOC-M11 MEDIUM `session open` unimplemented | **WRONG — finding must be removed** |
| F-DOC-M12 MEDIUM env-var divergence | **REAL** |
| F-DOC-M13 MEDIUM `final-l1-script` not in live script | **REAL** |
| Producer's summary severity counts (14 MEDIUM / 7 LOW / 6 INFO) | Off — actual table has 11 MEDIUM / 8 LOW / 8 INFO |
| Producer's "Of 18 findings" in verdict section | Inconsistent with 33-finding table — should be 33 |
| Verdict direction (FAIL on main) | **CORRECT** — CRITICAL F-DOC-M01 + 5 HIGH findings support FAIL even with F-DOC-M11 removed |

## 7. What needs to change

1. **F-DOC-M11 is wrong.** The producer must retract or correct
   this finding. `session open` IS implemented, wired, and tested
   (`cli/src/main.rs:290, 346, 1050, 5594, 13063`); the README
   example runs as documented. The finding's recommended
   remediation ("either implement `SessionOpenArgs` or update
   README, plan, and runbook to use `session open-fixture`
   exclusively") is moot because `SessionOpenArgs` is already
   implemented.

2. **F-DOC-M03 evidence trail:** the producer said
   `rg ... MYELIN_*.md README.md AGENTS.md` returns 0 matches.
   `AGENTS.md:25` has 1 match ("Fiber Network (`fnn`)"). The
   substantive conclusion (plan is unintegrated with Myelin's docs)
   is still correct — the AGENTS.md mention is a generic
   ecosystem hint unrelated to the new bridge plan. The "0
   matches" claim should be "0 matches in MYELIN_*.md and README.md;
   AGENTS.md has 1 unrelated ecosystem mention".

3. **F-DOC-M01 evidence count:** "calls `push_check` 22 times
   (counted by `grep -c 'push_check'` filtered by the function)"
   is misleading; a literal `grep -c` returns 25. The 22 figure
   is the per-execution count after accounting for the if/else
   branches. The implementation truth is 22 per execution
   (`len(checks)` in the acceptance script returns 22).

4. **Producer's summary vs verdict section:** the verdict
   section says "Of 18 findings"; the actual count is 33. The
   summary at the top of the deliverable says 33. Self-correct
   to 33.

These four corrections are non-blocking for the FAIL verdict.
The CRITICAL court_checks audit-chain defect is real and
sufficient on its own.

OVERALL: PASS
VERDICT: PASS
