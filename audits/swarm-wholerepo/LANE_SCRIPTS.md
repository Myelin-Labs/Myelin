# Myelin Swarm Audit — Scripts + Fixture Builder

> Verifier-only review. No fixes proposed. Scope: `scripts/myelin_ckb_devnet_smoke.sh`
> (1,785 lines, new), `scripts/myelin_public_testnet_rehearsal_live.sh` (286 lines,
> new), `scripts/myelin_public_testnet_rehearsal_prepare.sh` (264 lines, new),
> `scripts/myelin_production_gate.sh` (1,515 lines, heavily modified),
> `scripts/myelin_protocol_gate.sh` (compatibility wrapper, deleted),
> `scripts/myelin_teeworlds_acceptance.sh` (167 lines, small modification),
> `scripts/build_myelin_teeworlds_repro.py` (147 lines, small modification),
> `docs/templates/public-testnet-rehearsal/*` (6 JSON templates), and the deletion
> of `reports/myelin-teeworlds-repro.json`.

## Verdict

**Conditional PASS, with three substantive defects that should block merge to a
release branch.** The two new public-testnet rehearsal scripts
(`myelin_public_testnet_rehearsal_live.sh` and `..._prepare.sh`) are small,
disciplined, and follow the project's existing patterns. The
`myelin_teeworlds_acceptance.sh` change is correctly minimal — only the
`TEEWORLDS_ROOT` default was generalised (and the matching
`myelin_production_gate.sh` change is consistent with it). The deletion of
`reports/myelin-teeworlds-repro.json` is intentional and is now in `.gitignore`
(line 32); the production gate regenerates it via
`build_myelin_teeworlds_repro.py` (line 1508) on every Teeworlds run. The
deletion of `myelin_protocol_gate.sh` (a 9-line `exec`-wrapper) is also clean
and there are no stale references anywhere in the tree (verified by
`grep -rn 'myelin_protocol_gate'`).

The three blockers are:

1. **The CKB devnet smoke (`myelin_ckb_devnet_smoke.sh`) writes its summary
   JSON (lines 1567-1782) and `cat`s it (line 1784) without ever asserting
   that the computed `all_live_checks_passed` field is `true`.** A failure
   of the CKB script-verification rejection probe, a tampered-carrier
   acceptance, or a settlement-uniqueness check would still be encoded in
   the JSON, but the script would exit 0 because the report-writing path is
   unguarded by `set -e` (the only failure modes that exit 1 are the
   sub-assertions at lines 1108-1153, 1320, 1426, 1431). A negative result
   encoded as `all_live_checks_passed: false` would silently pass the
   wrapper exit-code check.
2. **`myelin_teeworlds_acceptance.sh` has a Python heredoc at lines 152-162
   that mixes TAB-indented dict entries (lines 156-159) with 4-space-indented
   dict entries (lines 153-155, 160-161).** Python's parser tolerates the
   mix because the dict is a continuation, not a logical line start, so this
   is a style defect rather than a runtime defect, but it would fail
   `python3 -tt` and it would fail any linter (`ruff`, `black`,
   `flake8`). It also would fail the project's own "reproducible-artefact"
   stance if the script is later pinned to a strict-mode Python.
3. **The CKB devnet smoke uses 32 `cargo run` invocations without
   `--locked`.** The production gate consistently uses `--locked` (lines
   49, 52, 56, 67, 68, 71, 74). The smoke is the only script that does not.
   The behaviour is benign while `Cargo.lock` is in place, but if the
   `Cargo.lock` is regenerated (e.g. by an `cargo update` on a developer's
   machine), the CKB smoke would resolve to a different cellc version and
   produce different verifier code hashes, silently invalidating the
   `da_verifier_code_hash` / `settlement_verifier_code_hash` strings
   embedded in the report. This is a determinism regression.

Several lower-severity hygiene issues are also listed below. None are
correctness bugs in the happy path; all are discipline drifts that a
reviewer should be aware of.

## Findings

| # | Severity | Component | Finding | File:Line | Doc claim | Code reality |
|---|----------|-----------|---------|-----------|-----------|--------------|
| F-SCRIPT-01 | **CRITICAL** | scripts/ckb_devnet_smoke | `all_live_checks_passed` is computed in the report (line 1749-1775) and emitted to `$REPORT` (line 1782) but **never asserted before exit 0**. The script `cat`s the report at line 1784 and exits without `set -e` triggering, because the writing jq never fails when the inputs are present. | `scripts/myelin_ckb_devnet_smoke.sh:1749-1782, 1784` | "Myelin CKB devnet smoke passed" (line 1785) | If `all_live_checks_passed == false` (e.g. tampered carrier accepted, replay not rejected, settlement uniqueness check failed, DA-anchor final readiness regressed), the script still exits 0. The internal sub-assertions at lines 1108-1153, 1320, 1426, 1431 protect *individual* failures, but the composite `all_live_checks_passed` predicate is not gated on exit. |
| F-SCRIPT-02 | **HIGH** | scripts/ckb_devnet_smoke | 32 `cargo run` invocations without `--locked` (line 80, 100, 118, 125, 132, 139, 146, 153, 160, 167, 175, 182, 189, 196, 208, 213, 217, 221, 230, 237, 242, 247, 793, 949, 1015, 1019, 1053, 1058, 1062, 1067). | `scripts/myelin_ckb_devnet_smoke.sh:80-1067` | None explicit; the production gate is consistent with `--locked` (`scripts/myelin_production_gate.sh:49,52,56,67,68,71,74`). | If `Cargo.lock` is regenerated on a developer's machine, `cargo run` would resolve to a different cellc version and produce a different `da_verifier_code_hash` / `settlement_verifier_code_hash` / `da_final_verifier_code_hash` / `settlement_final_verifier_code_hash` (lines 175-201). The smoke's report would then carry different code hashes than the ones the local-rehearsal would re-import, silently invalidating the cross-script fixture contract. |
| F-SCRIPT-03 | **HIGH** | scripts/teeworlds_acceptance | Python heredoc mixes TAB-indented dict entries (lines 156, 157, 158, 159) with 4-space-indented dict entries (lines 153, 154, 155, 160, 161). | `scripts/myelin_teeworlds_acceptance.sh:152-162` | Implicit "valid Python" requirement (the script's `set -euo pipefail` causes `python3 -` to fail-fast on `SyntaxError`/`IndentationError`). | Python's CPython parser accepts this because the dict is a statement continuation (not at the start of a logical line) — but `python3 -tt` (tab-nanny in strict mode) rejects it. Any linter (ruff, black, flake8 with E/W101) will flag it. The diff did not introduce this; it is a pre-existing style defect. |
| F-SCRIPT-04 | **MEDIUM** | scripts/ckb_devnet_smoke | `WORKDIR="${WORKDIR:-$(mktemp -d /tmp/myelin-ckb-devnet.XXXXXX)}"` (line 10) creates a fresh temp dir on first run, but if `WORKDIR` is exported and reused, no prior `session-da-store` / `*.dat` / `*.idx` / `*.log` is cleared. The trap (line 57) only kills the CKB process — it does not `rm -rf` the workdir. | `scripts/myelin_ckb_devnet_smoke.sh:10, 51-57, 219` | None explicit. | Re-running the smoke with `WORKDIR=/tmp/x` set produces a non-idempotent run: prior-run `da-miner-*.log` files and prior-run segment files accumulate; the new `da-manifest` step (line 217) writes to `session-da-store` inside the same WORKDIR, but prior-store contents are not removed, so two successive `da-manifest` invocations on the same `--storage-dir` would produce inconsistent segment roots across runs. The production gate handles this correctly (line 285: `rm -rf "${SESSION_DA_STORE_STATIC}" "${SESSION_DA_STORE_TENDERMINT}"`). |
| F-SCRIPT-05 | **MEDIUM** | scripts/ckb_devnet_smoke | ROOT derivation at line 4 uses the non-canonical `cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd` pattern (no `--`, no quoted `"${BASH_SOURCE[0]}"`). All other scripts in scope use the canonical `cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd` pattern (`myelin_production_gate.sh:21-22`, `myelin_public_testnet_rehearsal_live.sh:10-11`, `myelin_public_testnet_rehearsal_prepare.sh:11-12`, `myelin_teeworlds_acceptance.sh:4-5`). | `scripts/myelin_ckb_devnet_smoke.sh:4` | "ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" | The bare pattern still works because the script's `BASH_SOURCE[0]` is a controlled literal (`./myelin_ckb_devnet_smoke.sh` or its absolute path), and `set -u` is satisfied. But it is the only script in the lane that doesn't use the canonical form, which is a hygiene drift. |
| F-SCRIPT-06 | **MEDIUM** | scripts/ckb_devnet_smoke | `[[ "$settlement_authority_lock_args" != 0x6d79656c696e2d617574682d7631* ]]` (line 447) is an unquoted bash glob pattern. The unquoted right-hand side means bash performs pattern matching against the literal prefix `0x6d79656c696e2d617574682d7631` followed by `*` (any suffix). | `scripts/myelin_ckb_devnet_smoke.sh:447` | None explicit. | With `set -e` and no `failglob`, the comparison works. But the lock-args value could in principle contain `*` or `[` characters, which would be interpreted as part of the pattern. The production gate does the equivalent check in Python (line 1335: `bytes.fromhex(auth["ckb_lock_args"][2:]).startswith(b"myelin-auth-v1")`) which is more robust. The smoke's approach is not strictly wrong, but it relies on bash's pattern matching semantics that a future maintainer may break with `failglob` or `extglob`. |
| F-SCRIPT-07 | **MEDIUM** | scripts/public_testnet_rehearsal_live | `MYELIN_REHEARSAL_LIVE_SUBMIT` is the only opt-in gate (line 242). The script refuses to run unless this env var is `"1"`. But `MYELIN_REHEARSAL_ROLES` (line 14, default `"da-anchor"`) is word-split unquoted at line 282, so a user passing `MYELIN_REHEARSAL_ROLES="da-anchor settlement extra"` will attempt to dispatch the role `extra` and fail with the line 128 message — but only **after** `MYELIN_REHEARSAL_LIVE_SUBMIT=1` has been confirmed, so a typo in `ROLES` wastes a partial submission attempt. | `scripts/myelin_public_testnet_rehearsal_live.sh:14, 282` | "MYELIN_REHEARSAL_ROLES=\"da-anchor settlement\"" (line 31) | The script does not validate role names up front; an unknown role is only caught at the first `cargo run` invocation (line 158), which may have side effects on the running CKB testnet. The check would be safer if it ran before the first CLI invocation. |
| F-SCRIPT-08 | **MEDIUM** | scripts/ckb_devnet_smoke | `set -e` is enabled, but the script's external tool checks (lines 59-61) only cover `curl`, `jq`, `python3`. The script also uses `od`, `tr`, `wc`, `awk`, `seq`, `sed`, `cargo`, `mkdir`, `cp`, `file` (line 75's `file_hex` uses `od -An -tx1 -v`). | `scripts/myelin_ckb_devnet_smoke.sh:59-61` | Implicit "tool present or fail" (via `set -e`). | On a minimal Alpine/musl environment without GNU coreutils, `wc -c` or `od -An` may not exist (BusyBox has them, but with different output formats). The script's `da_verifier_elf_size="$(wc -c <"$WORKDIR/myelin/da-anchor-carrier.elf" | tr -d ' ')"` (line 178) on a BusyBox `wc` would emit `<bytes> <file>` (two fields) instead of just `<bytes>`, and the `tr -d ' '` would still leave the filename, producing a wrong `da_verifier_elf_size` and cascading into a wrong `da_verifier_code_capacity` and a wrong deployment transaction. |
| F-SCRIPT-09 | **MEDIUM** | scripts/production_gate | The production gate does not `require_cmd` any external tool (`cargo`, `python3`, `jq`, `rg` are used heavily but only `set -e` and the call site are protecting against missing tools). | `scripts/myelin_production_gate.sh` (no `require_cmd` function defined) | Implicit via `set -e`. | A first `cargo run -p myelin-cli` at line 97 would fail with a clear `cargo: command not found` and the script would exit 1. But `rg` (line 1405 inside the embedded `bash -c` and line 1437 in the stale-surface grep) and `python3` (lines 127, 170, 468, 572, 668, 751, 866, 1020, 1123, 1408, 1456, 1507) are not pre-checked. On a host that has `cargo` but not `rg` (Debian: `ripgrep` is a separate package), the script would fail deep inside the embedded bash, with an opaque error message. |
| F-SCRIPT-10 | **MEDIUM** | scripts/ckb_devnet_smoke | Hardcoded `ALWAYS_SUCCESS_CODE_HASH="0x28e83a1277d48add8e72fadaa9248559e1b632bab2bd60b27955ebc4c03800a5"` (line 12) and `GENESIS_ALWAYS_SUCCESS_DEP_INDEX="${GENESIS_ALWAYS_SUCCESS_DEP_INDEX:-0x5}"` (line 13). The code hash is not parameterised and not validated against the actual CKB version's `always_success` lock. | `scripts/myelin_ckb_devnet_smoke.sh:12, 13` | Implicit "match the CKB binary at `$CKB_BIN`". | If the developer has a different CKB version whose `always_success` lock has a different code hash (or whose dev chain places the lock at a different genesis dep index), the script will silently deploy 4 verifier code cells with mismatched `da_verifier_code_hash` etc., and the `commit_final_da_publication` script will reject the carrier output. The CKB binary is checked for presence (line 63) and the integration spec is copied (line 477-478), but the actual `always_success` lock's code hash is not re-hashed from the deployed cell. The error would only surface at the very end of the script, with an opaque `commit_final_da_publication` rejection. |
| F-SCRIPT-11 | **MEDIUM** | scripts/ckb_devnet_smoke | `wait_for_rpc` at line 34-43 uses `rpc '...get_tip_header...'` with `>/dev/null 2>&1`. The `rpc` function at line 30-32 uses `curl -fsS`, where `-f` makes curl exit non-zero on HTTP error, and `-S` makes it show errors. The `>/dev/null 2>&1` redirection silences both, so a `127.0.0.1:18314` connection-refused error is invisible. | `scripts/myelin_ckb_devnet_smoke.sh:34-43` | None explicit. | If CKB fails to bind to `RPC_PORT` (e.g. port already in use from a prior run, or `CKB_BIN` crashes immediately after `init`), the script will sleep for 60 seconds before failing — the only signal to the user is `CKB RPC did not become ready at $RPC_URL` (line 41). This is correct behaviour but the diagnostic cost is 60 seconds. |
| F-SCRIPT-12 | **MEDIUM** | docs/templates/public-testnet-rehearsal | Four `.template.json` files (`authority-signature-evidence`, `court-economics-deployment`, `external-da-receipt`, `threshold-lock-deployment`) are documented in `README.md:30-35` as "shape references only" that the operator should fill in. | `docs/templates/public-testnet-rehearsal/README.md:14-25, 26-36`, `docs/public-testnet-rehearsal-runbook.md:101-109` | "The CLI should reject unreplaced cryptographic templates." (README line 28) | Neither `myelin_public_testnet_rehearsal_prepare.sh` nor `..._live.sh` ever copies the `.template.json` files. The prepare script only copies `operator-custody-policy.json` and `operator-runbook.json` (lines 84-85). The `.template.json` files are therefore not actually wired into the data flow; the README claim that the CLI should reject unreplaced templates is not exercised by any lane script. If the CLI does not in fact reject unreplaced templates, the templates are documentation only; if the CLI does reject, the templates cannot be used at all. The doc-vs-code contract is unclear. |
| F-SCRIPT-13 | **MEDIUM** | scripts/public_testnet_rehearsal_prepare | `assert_valid()` at line 48-51 does `jq -e '.valid == true' "$path" >/dev/null` and returns the exit code. It does not emit any error message. The function relies on `set -e` (line 9) to make the script exit 1 if the jq call fails. | `scripts/myelin_public_testnet_rehearsal_prepare.sh:48-51, 101, 144, 155, 188, 224` | Implicit via `set -e`. | If `set -e` is ever relaxed (e.g. for debugging), the `assert_valid` calls at lines 101, 144, 155, 188, 224 would silently pass. A "false" `valid` in any of the five `*verify.json` reports would propagate to the `summary.json` (line 226-261) as `true` despite the verification failure, and the rehearsal's `phases_completed_locally` would be wrongly stamped as clean. A simple `if ! jq -e ... ; then echo "assert_valid: $path failed" ; exit 1 ; fi` would close the gap. |
| F-SCRIPT-14 | **MEDIUM** | scripts/production_gate | The production gate's "recomputed production DA readiness evidence" check at line 1079-1093 only checks that `readiness_evidence_mode == "coherent-offline-or-mock"` and that several markers are `False`. It does **not** check that the recompute is actually performed for the dry-run path — the `real-da-availability-guarantee-missing` blocker is not in the asserted blocker list at line 1085-1089. | `scripts/myelin_production_gate.sh:1079-1093, 1117-1120` | "recompute production DA readiness evidence" (commit `3fda2ab`, `cli/src/main.rs:9850-9957`) | The CLI was modified in commit `3fda2ab` to **recompute** the `da_availability_production_ready` flag by re-deriving the DA availability commitment from the manifest and comparing it to the manifest's stored commitment (`final_l1_da_availability_preflight_ready`, `cli/src/main.rs:9900-9957`). The production gate consumes the recomputed flag via `report.get("end_to_end_production_blockers", [])` (line 1085) and asserts that `operator-custody-policy-missing` and `operator-runbook-missing` are present, but does **not** assert that `real-da-availability-guarantee-missing` is present. The CKB devnet smoke (line 1142, 1147) does assert that blocker. So the production gate's "dry-run" path passes even if the CLI's `final_l1_da_availability_preflight_ready` were to silently no-op, because no assertion depends on the recomputed flag. The recompute is not actually exercised by the production gate's dry-run path. |
| F-SCRIPT-15 | **LOW** | scripts/production_gate | `RUN_TEEWORLDS=0` (line 30) and `ALLOW_SKIP_TEEWORLDS=1` (line 1495) are explicit opt-out flags. The skip path is documented and gated, but it allows the production gate to pass without exercising the `myelin_teeworlds_acceptance.sh` end-to-end check, which is the only path that verifies the CKB-VM compatibility of the ckb-strict profile. | `scripts/myelin_production_gate.sh:30, 1492-1511` | "Teeworlds acceptance, required by default" (line 1491) | `RUN_TEEWORLDS=0` is a hard opt-out and `ALLOW_SKIP_TEEWORLDS=1` is a soft opt-out. The doc line 13 says "Exits non-zero on any failure" but the gate can be set to exit 0 without running the Teeworlds step. The doc-vs-code gap is the explicit and documented escape hatches, but those escape hatches are not signposted in the docstring at the top of the file (lines 1-17). |
| F-SCRIPT-16 | **LOW** | scripts/public_testnet_rehearsal_live | The summary jq at line 225-234 reads `"$SUMMARY_PATH"`, mutates it, and writes to `"$tmp_summary"`, then `mv` to the final path. The `local tmp_summary="${SUMMARY_PATH}.tmp"` is declared inside the function, so a stale `SUMMARY_PATH.tmp` from a prior failed run is overwritten on every iteration. The atomic-rename pattern is correct, but a `.tmp` file from a prior SIGKILL'd run is never cleaned up if the script starts fresh. | `scripts/myelin_public_testnet_rehearsal_live.sh:224-234` | None explicit. | A prior-run leftover `public-testnet-live-summary.json.tmp` will be silently overwritten on the next run's first iteration. This is correct behaviour but not idempotent on the disk side. The main summary file (`$SUMMARY_PATH`) is created at line 266-280, which atomically truncates the prior summary if any. |
| F-SCRIPT-17 | **LOW** | scripts/ckb_devnet_smoke | `compile_carrier_verifiers` is called at line 347, but it is the *only* function in the script that runs 8 `cargo run --bin cellc` invocations (lines 118-201). The function has no error trap. | `scripts/myelin_ckb_devnet_smoke.sh:113-204, 347` | Implicit via `set -e`. | A compile failure inside `compile_carrier_verifiers` would propagate via `set -e`. But the function does its own ELF hashing (lines 175-201) with no `ckb_hash_hex` indirection — meaning the hash format is whatever `cellc ckb-hash --file ... --json | jq -r '.hash'` produces. If a future cellc version changes the JSON shape (e.g. `{ "hash": "0x..." }` vs `{ "code_hash": "0x..." }`), the jq filter would silently emit `null` and the smoke would deploy an all-zero code hash. The 8 invocations are not DRY: a single helper would centralise the schema contract. |
| F-SCRIPT-18 | **LOW** | scripts/ckb_devnet_smoke | The `cat "$REPORT"` at line 1784 emits the full JSON to stdout. With `set -e`, the script has not exited 1 at this point, so a `myelin_ckb_devnet_smoke.sh | jq '.all_live_checks_passed'` pipeline would receive valid JSON, but the `myelin_ckb_devnet_smoke.sh && echo OK` pattern would be misleading: the wrapper exited 0 even if `all_live_checks_passed == false`. | `scripts/myelin_ckb_devnet_smoke.sh:1782-1784` | "Myelin CKB devnet smoke passed. Report: $REPORT" (line 1785) | See F-SCRIPT-01. The exit-0 wrapping is the same defect. The "passed" string in line 1785 is unconditional, not gated on `all_live_checks_passed`. |
| F-SCRIPT-19 | **LOW** | scripts/ckb_devnet_smoke | The `tip_number="$((tip_hex))"` at line 487 is a hex-to-decimal conversion. `tip_hex` comes from `rpc '...get_tip_header...'` at line 486. The RPC response field is `.result.number`, which CKB returns as a hex string `"0x..."`. If the field is missing, `tip_hex` would be `null` (string) and `$((null))` would emit a bash error. With `set -e`, this would exit the script. | `scripts/myelin_ckb_devnet_smoke.sh:486-487` | None explicit. | The `jq -r '.result.number'` filter at line 486 is unguarded — if `.result` is missing, jq emits `null`, which then fails the arithmetic. The CKB RPC mock at `myelin_production_gate.sh:693-695` always returns `.result.number`, so the production gate is safe, but the CKB devnet smoke depends on the real CKB binary, whose response format is not validated. |
| F-SCRIPT-20 | **LOW** | scripts/ckb_devnet_smoke | `required_reward_capacity="$((total_verifier_code_capacity + settlement_authority_capacity + DEPLOY_FEE_SHANNONS + MIN_FUNDING_CELL_CAPACITY_SHANNONS + (4 * CARRIER_CELL_CAPACITY_SHANNONS) + (4 * FEE_SHANNONS)))"` at line 488. | `scripts/myelin_ckb_devnet_smoke.sh:488` | None explicit. | The arithmetic uses `(( ... ))` semantics; if any of the inputs is unset (e.g. `MIN_FUNDING_CELL_CAPACITY_SHANNONS` is exported to empty), the variable defaults to 0 via the `${...:-default}` pattern at lines 18-20, so this is not a runtime crash. But the `MIN_FUNDING_CELL_CAPACITY_SHANNONS` at line 18 has default `10000000000` (10 billion shannons = 100 CKB), and the smoke's `MIN_FUNDING_CELL_CAPACITY_SHANNONS` and `CARRIER_CELL_CAPACITY_SHANNONS` (line 19: `42000000000` = 420 CKB) and `SETTLEMENT_AUTHORITY_CELL_CAPACITY_SHANNONS` (line 20: 300 CKB) are large. The user can override these but there is no documentation in the script's header comment (lines 1-2) about what these mean or what their cumulative cost is. A casual re-run with an under-funded CKB devnet would fail at line 514, not at the arithmetic. |
| F-SCRIPT-21 | **LOW** | scripts/public_testnet_rehearsal_live | `CKB_TESTNET_LOCK_ARGS` (line 168) is read with `${CKB_TESTNET_LOCK_ARGS:-0x}` — there is no `require_env` for it, and the default `"0x"` is silently used. The same is true for `CKB_TESTNET_LOCK_HASH_TYPE` (line 167, default `"type"`) and `CKB_TESTNET_FUNDING_TX_HASH` (referenced only via `env_or` for the da-anchor role, not for settlement). | `scripts/myelin_public_testnet_rehearsal_live.sh:167, 168` | Implicit (operator should provide all env vars per runbook section "Inputs"). | A misconfigured operator who forgets `CKB_TESTNET_LOCK_ARGS` would submit carriers with `args: "0x"` and get a `LockScript` error from CKB at submission time, not at script start. The script could check the bare minimum of `CKB_TESTNET_LOCK_ARGS` non-empty before the first CLI invocation. |
| F-SCRIPT-22 | **LOW** | scripts/production_gate | Lines 532, 628, 710, 831, 959 bind a local HTTP server to `127.0.0.1:0` (random port). The `try`/`finally` at lines 533-568, 629-664, 707-748, 828-863, 956-1017 calls `server.shutdown()` and `thread.join(timeout=5)`. | `scripts/myelin_production_gate.sh:529-568, 625-664, 707-748, 828-863, 956-1017` | Implicit via `set -e` and `try`/`finally`. | The pattern is correct. The `thread.join(timeout=5)` is generous, but if the thread is stuck on a `BaseHTTPRequestHandler.handle()` call, the `server.shutdown()` may not unblock the thread (Python `socketserver` shutdown is cooperative but `handle()` may be blocked on `rfile.read()`). The pattern works in practice because the mock server only reads request bodies and immediately writes a response, but the `thread.join(timeout=5)` is the only guarantee the script does not deadlock on teardown. |
| F-SCRIPT-23 | **LOW** | scripts/ckb_devnet_smoke | `cleanup()` at line 51-56 kills the CKB process and `wait`s. The `trap cleanup EXIT` (line 57) is set **before** `CKB_PID` is assigned (line 481). The trap dereferences `CKB_PID` via `${CKB_PID:-}` (line 52), which is the correct pattern. | `scripts/myelin_ckb_devnet_smoke.sh:51-57, 481` | Implicit. | If the script aborts before line 481 (e.g. `CKB_BIN` not found at line 63), the trap fires with `CKB_PID=""`, the `kill -0` returns 1, and the trap returns cleanly. Good. But: the trap does **not** remove the temp workdir (F-SCRIPT-04). On a long-lived CI machine, `/tmp/myelin-ckb-devnet.*` directories accumulate. |
| F-SCRIPT-24 | **LOW** | docs/templates/public-testnet-rehearsal | `operator-custody-policy.json` (line 2: `schema: myelin-operator-custody-policy-v1`) and `operator-runbook.json` (line 2: `schema: myelin-operator-runbook-v1`) have no `evidence_commitment` or `evidence_commitment_algorithm` fields, while the other 4 `.template.json` files have both. | `docs/templates/public-testnet-rehearsal/operator-custody-policy.json:1-13`, `docs/templates/public-testnet-rehearsal/operator-runbook.json:1-16` | "shape references and fallback review aids" (README line 11) | The CLI's `verify-submission-readiness` (referenced at `myelin_public_testnet_rehearsal_prepare.sh:213` and `myelin_ckb_devnet_smoke.sh:1067`) embeds `operator_custody_policy_hash` and `operator_runbook_hash` (per `myelin_production_gate.sh:1104-1108`). The two operator documents have no `evidence_commitment` field to bind, while the 4 deployment/receipt templates do. The asymmetric schema is intentional (the operator documents are starter fixtures, not cryptographic evidence), but it would help to add a comment in each operator file explaining the contract. |
| F-SCRIPT-25 | **LOW** | scripts/ckb_devnet_smoke | `mine()` at line 45-49 calls `"$CKB_BIN" -C "$WORKDIR" miner --limit "$limit"` and discards stdout via redirection. The CKB miner's `miner` subcommand does not need to communicate with the RPC — it operates on the local chain. | `scripts/myelin_ckb_devnet_smoke.sh:45-49` | Implicit. | The `> "$WORKDIR/ckb-miner-$label.log" 2>&1` redirection is correct. But the `mine` function does not validate that the chain actually advanced. A miner failure (e.g. insufficient `INITIAL_MINING_BLOCKS`) would be silent, and the next `rpc 'get_tip_header'` would still return a low block number. The smoke does check at line 514-517 (`reward_capacity < required_reward_capacity`), which indirectly catches a stuck miner, but the diagnostic is `could not find enough spendable always-success reward capacity` rather than `miner did not advance chain`. |
| F-SCRIPT-26 | **INFO** | scripts/production_gate | The production gate is consistent with the protocol gate's removal. The compatibility wrapper (`myelin_protocol_gate.sh`) was a 9-line `exec` wrapper. Its deletion removes a no-value indirection. | (file deleted at commit `c8008e3`) | None. | `grep -rn 'myelin_protocol_gate' --include='*.sh' --include='*.md' --include='*.toml' --include='*.yml' --include='*.yaml'` returns zero matches. The deletion is clean. The `exclude` list at line 1419 of the production gate was updated to remove `myelin_protocol_gate.sh`, consistent with the deletion. |
| F-SCRIPT-27 | **INFO** | reports/myelin-teeworlds-repro.json | The file deletion at commit `c8008e3` is intentional. The file is now in `.gitignore` (line 32: `/reports/myelin-teeworlds-repro.json`). The production gate regenerates it via `build_myelin_teeworlds_repro.py:141-142` (line 1508 of the gate). | `.gitignore:32`, `scripts/build_myelin_teeworlds_repro.py:24, 141-142`, `scripts/myelin_production_gate.sh:1507-1508` | None. | The `reports/` directory is otherwise empty (`ls reports/` returns no entries), confirming that the deleted file was the only committed artefact. The deletion is not a regression. |
| F-SCRIPT-28 | **INFO** | scripts/ckb_devnet_smoke | `ALLOW_SKIP_TEEWORLDS`-style escape hatches are absent from the CKB devnet smoke. The smoke is fully required (no skip flag). | `scripts/myelin_ckb_devnet_smoke.sh` (no `ALLOW_SKIP` env var) | None. | This is correct: the CKB devnet smoke is the substantive check for live CKB script verification. Allowing it to be skipped would defeat its purpose. The opt-in is **whether to run the smoke at all** (the production gate's `RUN_TEEWORLDS=0` is a different axis — Teeworlds acceptance, not CKB devnet). |

## Exit code discipline

The 5 scripts in scope all use `set -euo pipefail` and a single global exit
code of 1. This is **internally consistent** but lacks the granularity that a
larger project (e.g. the CKB devnet smoke with 75 `exit 1` sites) would benefit
from. Specific concerns:

- **All exits are 1.** `myelin_ckb_devnet_smoke.sh:26, 65, 69, 257, 262, 279,
  283, 287, 291, 295, 317, 321, 325, 329, 333, 337, 341, 345, 381, 385, 389,
  393, 397, 401, 405, 409, 413, 417, 421, 425, 429, 433, 437, 441, 445, 449,
  453, 474, 517, 620, 637, 643, 648, 653, 658, 663, 668, 725, 746, 760, 774,
  828, 843, 848, 853, 872, 908, 913, 1004, 1012, 1043, 1050, 1115, 1126, 1130,
  1134, 1139, 1144, 1149, 1153, 1320, 1426, 1431` all `exit 1` with no
  per-site error code differentiation. CI can distinguish failures by reading
  the final `$REPORT` JSON, but the exit code itself is uninformative.
- **Subshell `set -e` propagation.** The production gate has one embedded
  `bash -c "set -e; ..."` (line 1405) that re-enables `set -e` inside the
  subshell. The CKB devnet smoke's `run_step "Copy rehearsal sources and
  operator starter documents" bash -c 'set -euo pipefail; ...'` (in
  `myelin_public_testnet_rehearsal_prepare.sh:78-86`) also re-enables. The
  practice is correct, but the discipline is local — there is no project-wide
  `bash -e` template.
- **Python heredocs in bash.** All five scripts use `python3 - <<'PY' ... PY`
  with `<<'PY'` (single-quoted heredoc) to prevent shell expansion inside the
  Python. This is correct. The teeworlds acceptance at lines 100-164 also
  uses `<<'PY'`.

## Command availability

| Script | `require_cmd` defined? | Commands used but not pre-checked |
|--------|------------------------|-----------------------------------|
| `myelin_production_gate.sh` | No | `cargo`, `python3`, `jq` (none, jq is not used directly), `rg`, `cat`, `sed` |
| `myelin_ckb_devnet_smoke.sh` | Yes (line 23) | `cargo`, `od`, `tr`, `wc`, `awk`, `seq`, `sed`, `mkdir`, `cp`, `file`, `bash`, `printf` |
| `myelin_public_testnet_rehearsal_live.sh` | Yes (line 38) | `cargo` (via `myelin()` wrapper, which would `set -e` fail), `bash` |
| `myelin_public_testnet_rehearsal_prepare.sh` | Yes (line 17) | `cargo`, `bash` |
| `myelin_teeworlds_acceptance.sh` | Yes (line 33) | `cargo`, `python3` |

**F-SCRIPT-08** and **F-SCRIPT-09** are the high-severity versions of this
gap. The CKB devnet smoke's `od -An -tx1` (line 75) is GNU-specific (BusyBox
`od` accepts the same flags but emits differently formatted output). The
production gate's `rg` (line 1405, 1437) is a separate Debian package from
`ripgrep` and may not be installed on a minimal CI image.

## Fixture ↔ template consistency

The 6 JSON templates under `docs/templates/public-testnet-rehearsal/` are:

| File | Schema | Used by script | Required fields enforced by script |
|------|--------|----------------|-------------------------------------|
| `operator-custody-policy.json` | `myelin-operator-custody-policy-v1` | `myelin_public_testnet_rehearsal_prepare.sh:84` (copies) and `myelin_public_testnet_rehearsal_live.sh:220-221` (passes to CLI) | None — script only checks file presence. |
| `operator-runbook.json` | `myelin-operator-runbook-v1` | Same as above | None. |
| `external-da-receipt.template.json` | `myelin-external-da-receipt-v2` | Not copied by prepare.sh; only referenced in `README.md:30-35` | None — the CLI is expected to reject unreplaced templates, but this is not exercised by any lane script. **F-SCRIPT-12**. |
| `authority-signature-evidence.template.json` | `myelin-session-authority-signature-evidence-v1` | Same | None. |
| `court-economics-deployment.template.json` | `myelin-session-court-economics-deployment-v1` | Same | None. |
| `threshold-lock-deployment.template.json` | `myelin-session-threshold-lock-deployment-v1` | Same | None. |

The two operator starter files (`operator-custody-policy.json` and
`operator-runbook.json`) are the **only** templates that the prepare script
copies into the rehearsal artefact directory (lines 84-85). The other 4
`.template.json` files are **never copied** by any lane script, so the
README's "shape reference" framing is consistent with the code, but the
"the CLI should reject unreplaced cryptographic templates" claim at README
line 28 is **not exercised by the lane scripts**. If the CLI does reject,
the templates are documentation only. If the CLI does not reject, the
templates are documentation only with no enforcement path.

The prepare.sh substitutes placeholders via the CLI (e.g. `external-da-receipt
--signing-request` at line 108-120, then `external-da-receipt --provider-secret-key`
at line 121-133), so the production-quality evidence is generated by the CLI,
not by manual template-filling. The `.template.json` files therefore have no
runtime data-flow role.

## Production gate behaviour

Walking the 15 numbered steps in `myelin_production_gate.sh`:

1. **cargo fmt** (line 43): `cargo fmt --all --check`. Standard. No script
   logic.
2. **git diff --check** (line 46): Standard. No script logic.
3. **cargo check --locked** (line 49): Standard.
4. **cargo clippy** (line 52): `cargo clippy --locked --workspace --all-targets -- -D warnings`. `set -e` propagates the clippy exit code.
5. **cargo test** (lines 55-64): `-p myelin-hashes -p myelin-math -p myelin-exec -p myelin-consensus -p myelin-state -p myelin-mempool -p myelin-utils -p myelin-cli`. **Note:** `cargo test --locked --workspace` would already run all of these; the explicit `-p` list is redundant and may diverge from the actual workspace if a crate is renamed. If the intent is "focused test" (i.e. faster), the redundant list is correct. If the intent is "all tests", the `-p` list is brittle.
6. **5b. myelin-state and myelin-mempool full tests** (lines 67-68): `cargo test --locked -p myelin-state` and `cargo test --locked -p myelin-mempool`. Redundant with step 5's `-p` list. If a future developer adds `-p myelin-x` to step 5 and removes `myelin-state` from the explicit list at step 5, step 5b still runs `myelin-state` standalone. The duplication is defensive, not buggy.
7. **myelin-consensus tests** (line 71): Redundant with step 5.
8. **cellscript** (line 74): `bash -c "cd cellscript && cargo check --locked -p cellscript --all-targets"`. The `bash -c` is a subshell, so the parent shell remains in `$MYELIN_ROOT` after the step. Correct.
9. **CLI smoke** (lines 77-152): Two `committee finalise-demo` invocations + a Python contract check. The Python check at lines 127-152 asserts `len(committee["signer_ids"]) >= 2` and the equivalent for tendermint. The committee configs are TOML with 2 validators each. Standard.
10. **Runtime smoke** (lines 154-227): `runtime smoke --consensus {static-closed-committee,tendermint}` + Python check that asserts cross-engine agreement on `cell_tx_id`, `cell_wtxid`, `state_root_before`, `state_root_after`, and disagreement on `certificate_hash`. The "CellTx + state mutation is consensus-independent" claim is the contract. Standard.
11. **Session L2 fixture** (lines 229-1399): 2× 14-step sessions (static + tendermint), each invoking `open-fixture`, `commit-fixture`, `court-bundle`, `verify-court-bundle`, `da-manifest`, `verify-da-manifest`, `da-anchor-package`, `verify-da-anchor-package`, `submit-da-anchor-package --dry-run`, `settlement-intent`, `verify-settlement-intent`, `settlement-package`, `verify-settlement-package`, `submit-settlement-package --dry-run`. Each step is gated by `set -e`; the contract is enforced by 4 Python check blocks (lines 467-569, 571-665, 667-749, 751-864, 866-1018, 1020-1121, 1123-1401) that run 5 mock CKB RPC servers and validate the response shape, schema, marker values, hash bindings, and end-to-end production blocker presence. **F-SCRIPT-14** notes that the `real-da-availability-guarantee-missing` blocker is asserted only for the dry-run path, not for the recompute path.
12. **Dependency tree forbidden crates** (line 1404-1405): `cargo tree -p myelin-cli -e normal | rg -q 'workflow-node|workflow-perf-monitor'`. The check is wrapped in `bash -c "set -e; ..."` which re-enables `set -e` inside the subshell. **F-SCRIPT-09** notes `rg` is not pre-checked.
13. **Stale-surface grep** (line 1408-1453): Python loop over 10 patterns, excluding `myelin_production_gate.sh` itself. Standard.
14. **Forbidden parent path audit** (line 1456-1488): 3 patterns for `Spora` references. Standard.
15. **Teeworlds acceptance** (line 1492-1511): Conditional on `RUN_TEEWORLDS=1` and on the replayer + rust-tools manifest being present. The opt-out is documented and gated.

**TODO/skip findings:** None. The production gate has no `TODO`, no
`# FIXME`, no `exit 0` inside an error path, no `|| true` that masks a
failure.

**`# 15. Teeworlds acceptance, required by default`** (line 1491): the
opt-out via `ALLOW_SKIP_TEEWORLDS=1` (line 1495) is gated, but the
production gate's exit code discipline allows the script to exit 0
without exercising the Teeworlds step. **F-SCRIPT-15** flags this.

## Public-testnet rehearsal data flow

```
docs/templates/public-testnet-rehearsal/*.json
        │
        │  (prepare.sh:78-86)
        ▼
REHEARSAL_DIR/session-open.json
REHEARSAL_DIR/session-commit.json
REHEARSAL_DIR/session-court.json
REHEARSAL_DIR/session-court-verify.json        (assert_valid, line 101)
REHEARSAL_DIR/session-da-in-memory.json         (in-memory DA manifest)
REHEARSAL_DIR/external-da-receipt.signing-request.json  (provider signature request)
REHEARSAL_DIR/external-da-receipt.json          (signed receipt)
REHEARSAL_DIR/session-da.json                   (manifest + external receipt)
REHEARSAL_DIR/session-da-verify.json            (assert_valid, line 144)
REHEARSAL_DIR/session-da-anchor-package.json
REHEARSAL_DIR/session-da-anchor-package-verify.json  (assert_valid, line 155)
REHEARSAL_DIR/court-economics-deployment.json
REHEARSAL_DIR/session-settlement-intent.json
REHEARSAL_DIR/session-settlement-intent-verify.json  (assert_valid, line 188)
REHEARSAL_DIR/session-settlement-package.json
REHEARSAL_DIR/session-settlement-package-verify.json  (assert_valid, line 224)
REHEARSAL_DIR/operator-custody-policy.json     (copied from template)
REHEARSAL_DIR/operator-runbook.json            (copied from template)
REHEARSAL_DIR/da-anchor-carrier.cell           (copied from cellscript/examples)
REHEARSAL_DIR/settlement-carrier.cell          (copied from cellscript/examples)
REHEARSAL_DIR/da-anchor-final.cell             (copied from cellscript/examples)
REHEARSAL_DIR/settlement-final.cell            (copied from cellscript/examples)
REHEARSAL_DIR/rehearsal-prepare-summary.json
        │
        │  (live.sh:282-284)
        ▼
[ per role ]
live.sh:role_config(role)               sets global vars (no subshell)
live.sh:run_step ...                    myelin session carrier-submission --submit --require-accepted
live.sh:run_step ...                    myelin session verify-submission-context
live.sh:run_step ...                    myelin session verify-submission-economics
live.sh:run_step ...                    myelin session verify-submission-inclusion
live.sh:run_step ...                    myelin session verify-submission-stability
live.sh:run_step ...                    myelin session verify-submission-finality
live.sh:run_step ...                    myelin session verify-submission-readiness
        │  (jq slurpfile + .roles += [...])
        ▼
REHEARSAL_DIR/da-anchor-{carrier-submission,context,economics,inclusion,stability,finality,readiness}.json
REHEARSAL_DIR/settlement-{carrier-submission,context,economics,inclusion,stability,finality,readiness}.json
REHEARSAL_DIR/public-testnet-live-summary.json  (per-role roles[])
```

**Chain breaks found:**

- **`assert_valid` silent pass if `set -e` is relaxed** (F-SCRIPT-13). The
  function at `myelin_public_testnet_rehearsal_prepare.sh:48-51` does not
  emit a diagnostic message; if `set -e` is ever disabled (e.g. in a
  debugging session), the rehearsal's `summary.json` would record
  `valid: true` for a verify report that was actually `valid: false`.
- **No validation that `MYELIN_REHEARSAL_ROLES` values are well-formed
  before the first CLI invocation** (F-SCRIPT-07). A typo in `ROLES`
  causes a `cargo run` invocation that may have side effects on the live
  CKB testnet.
- **`.template.json` files are not wired into the data flow** (F-SCRIPT-12).
  The README's claim that the CLI should reject unreplaced cryptographic
  templates is not exercised. The 4 deployment/receipt templates are
  documentation only.

## CKB devnet smoke (1,785 lines)

The script has 4 macro-phases:

1. **Setup** (lines 1-72): `require_cmd`, `trap cleanup EXIT`, `mkdir -p
   $WORKDIR/myelin $WORKDIR/specs/cells`.
2. **Pre-flight evidence generation** (lines 73-455): 8 `cargo run -p
   myelin-cli -- session ...` invocations to produce `session-open.json`,
   `session-commit.json`, `session-court.json`, `session-da.json`,
   `session-da-anchor.json`, `session-settlement-package.json`, plus 5
   `verify-*` reports. The pre-flight then asserts 100+ invariants via
   shell `[[ ... ]]` checks (lines 254-454). The shell checks are
   **defensive but rigid** — every field is read with `jq -r
   '...field_name...' "$path"` and then compared as a string. There is
   no JSON-schema validation; a future schema change in the CLI would
   break the smoke at the next field that is renamed.
3. **Carrier verifiers** (lines 113-204): 8 `cargo run --bin cellc` to
   compile 4 verifiers × 2 profiles (`typed-cell` + `ckb`), then 4
   `cellc ckb-hash --file ... --json | jq -r '.hash'` to compute the
   code hashes. The function is called at line 347.
4. **CKB devnet** (lines 457-1782): `init` (line 463), `run` background
   (line 480), `wait_for_rpc` (line 482), `mine 48` initial blocks (line
   484), collect reward cells (lines 491-512), deploy 4 verifier code
   cells + 1 settlement authority cell (lines 540-621), mine 8 blocks
   (lines 624-632), verify `outputs_data[1..5]` (lines 640-669), then 4
   `submit_and_verify_carrier` invocations (lines 1479-1565) and 2
   `assert_tampered_carrier_rejected` invocations (lines 1495-1501,
   1519-1525). Each carrier submission includes a competing-settlement
   probe (lines 855-947) that mutates a single byte and asserts CKB
   rejects the mutation.
5. **Final report** (lines 1567-1784): a single `jq -n ...` builds the
   composite report and writes it to `$REPORT`. The script then `cat`s
   the report and exits 0 (line 1784).

**False-positive risks found:**

- **F-SCRIPT-01** (composite `all_live_checks_passed` is not asserted on
  exit).
- **F-SCRIPT-02** (no `--locked` on `cargo run`).
- **F-SCRIPT-04** (workdir not cleared on re-run).
- **F-SCRIPT-08** (`od`, `tr`, `wc`, `awk` not pre-checked).
- **F-SCRIPT-10** (`ALWAYS_SUCCESS_CODE_HASH` hardcoded; not re-hashed
  from the actual deployed cell).
- **F-SCRIPT-11** (60-second blind `wait_for_rpc` if CKB fails to bind).
- **F-SCRIPT-19** (`tip_number` arithmetic on a potentially-null jq
  output).
- **F-SCRIPT-20** (capacity arithmetic is correct but undocumented in
  the header).

**Network availability:** None. The smoke uses `127.0.0.1` exclusively
(line 9). The CKB binary is launched by the script (line 480). No
external network.

## Teeworlds acceptance and reports deletion

The `myelin_teeworlds_acceptance.sh` diff is minimal (8 lines: TEEWORLDS_ROOT
defaulting). The `build_myelin_teeworlds_repro.py` diff is also minimal
(docstring update). The `reports/myelin-teeworlds-repro.json` deletion is
intentional and is now in `.gitignore:32`. The `myelin_protocol_gate.sh`
deletion is clean and there are no stale references anywhere in the tree.

The teeworlds acceptance has a **pre-existing style defect** in the Python
heredoc at lines 152-162: the dict mixes TAB-indented entries (lines 156-159)
with 4-space-indented entries (lines 153-155, 160-161). The Python parser
accepts this because the dict is a continuation, not a logical line start,
but the resulting code fails `python3 -tt`, `ruff`, `black`, and
`flake8 E/W101`. **F-SCRIPT-03**.

## Determinism

The 5 scripts in scope produce the following evidence files:

| Script | Outputs | Determinism risk |
|--------|---------|------------------|
| `myelin_production_gate.sh` | `OUTPUT_DIR/{static-committee.toml,json,tendermint.toml,json,runtime-smoke-{static,tendermint}.json,session-{open,commit,court,verify,da,da-verify,da-anchor,da-anchor-verify,da-anchor-submit,da-anchor-context,da-anchor-economics,da-anchor-inclusion,da-anchor-stability,da-anchor-finality,da-anchor-readiness}-{static,tendermint}.json,session-{settlement,settlement-verify,package,package-verify,package-submit}-{static,tendermint}.json,cli-tree.txt,exec-tree.txt,teeworlds/*}` | Path-of-Myelin is `MYELIN_ROOT`; OUTPUT_DIR defaults to `/tmp/myelin-production-gate`. `MYELIN_ROOT` is derived from the script location, so the path is stable per checkout. `TEEWORLDS_ROOT` depends on `${HOME}` (line 23) and is not deterministic across users — but the gate does not embed it in evidence, only in `TEEWORLDS_OUTPUT_DIR` and the Teeworlds acceptance's `OUTPUT_DIR`. The Teeworlds acceptance's `BUILD_FIXTURE_REPORT` etc. embed `TEEWORLDS_ROOT` indirectly via the `cargo run --teeworlds-root "${TEEWORLDS_ROOT}"` argument; the JSON report does not embed `TEEWORLDS_ROOT`. |
| `myelin_ckb_devnet_smoke.sh` | `$WORKDIR/myelin-ckb-devnet-smoke.json` + ~200 intermediate files in `$WORKDIR` | `WORKDIR` defaults to `/tmp/myelin-ckb-devnet.XXXXXX` (line 10). The `XXXXXX` is a random suffix; re-runs of the script produce different workdirs. The final `$REPORT` is a single file with deterministic contents (assuming CKB and the CLI are deterministic). The `CKB_PID` is the only embedded runtime value; it is not in the report. The `ckb_version` is the CKB binary's `--version` output (line 1570) — this is a string and is deterministic per CKB build. |
| `myelin_public_testnet_rehearsal_live.sh` | `$REHEARSAL_DIR/public-testnet-live-summary.json` + per-role `*-{carrier-submission,context,economics,inclusion,stability,finality,readiness}.json` | `REHEARSAL_DIR` defaults to `$MYELIN_REHEARSAL_DIR` or `/tmp/myelin-public-testnet-rehearsal-live-XXXXXX` (live.sh does not set this; it relies on the operator or the prepare.sh). The summary is deterministic given a fixed CKB testnet state and fixed env vars. |
| `myelin_public_testnet_rehearsal_prepare.sh` | `$REHEARSAL_DIR/rehearsal-prepare-summary.json` + 15+ intermediate files | `REHEARSAL_DIR` defaults to `$MYELIN_REHEARSAL_DIR` or `/tmp/myelin-public-testnet-rehearsal-prepare.XXXXXX` (line 13). The summary embeds the `REHEARSAL_DIR` path (line 228), so two different runs produce two different summaries. The contents are otherwise deterministic. |
| `myelin_teeworlds_acceptance.sh` | `$OUTPUT_DIR/{scripted-tape.bin,teeworlds-mock-tx.json,build-fixture.json,vm-probe.json,court-bundle.json,court-bundle-verify.json}` + `reports/myelin-teeworlds-repro.json` (via `build_myelin_teeworlds_repro.py`) | `OUTPUT_DIR` defaults to `/tmp/myelin-teeworlds-acceptance`. Determinism depends on the Teeworlds tape, the CKB replayer, and the CLI. The Teeworlds tape is built from a fixed seed (`SEED=1`, line 17), so the tape is deterministic. The CLI's `build-fixture` runs 3 times (`RUNS=3`, line 18) and averages. The CLI's outputs are deterministic for fixed inputs. |

**Determinism defects found:**

- **F-SCRIPT-04** (CKB devnet smoke: workdir not cleared on re-run).
- **F-SCRIPT-02** (CKB devnet smoke: no `--locked` on `cargo run`).
- **F-SCRIPT-10** (CKB devnet smoke: hardcoded `ALWAYS_SUCCESS_CODE_HASH`).
- **F-SCRIPT-20** (CKB devnet smoke: large capacity numbers are
  underdocumented; a re-run with different env vars would produce a
  different report).

## Shellcheck-level hygiene

| Check | Production gate | CKB devnet smoke | Public-testnet live | Public-testnet prepare | Teeworlds acceptance |
|-------|-----------------|-------------------|---------------------|------------------------|---------------------|
| `set -euo pipefail` at top | ✓ (line 19) | ✓ (line 2) | ✓ (line 8) | ✓ (line 9) | ✓ (line 2) |
| Variables quoted | Mostly | Mostly | Mostly | Mostly | Mostly |
| `cd --` pattern | ✓ | ✗ (line 4, no `--`) | ✓ | ✓ | ✓ |
| `trap` for cleanup | None | ✓ (line 57) | None | None | None |
| Idempotency on re-run | ✓ (`rm -rf` at line 285) | ✗ (F-SCRIPT-04) | ✗ (no cleanup of `$REHEARSAL_DIR`) | ✗ (no cleanup of `$REHEARSAL_DIR`) | N/A (uses `/tmp/...`) |
| `require_cmd` for every external tool | ✗ (F-SCRIPT-09) | ✗ (F-SCRIPT-08) | Partial | Partial | Partial |
| Atomic file writes | N/A | N/A | ✓ (line 224-234) | N/A | N/A |
| `set +e` / `set -e` toggling | None | None | None | None | None |
| Output to stderr vs stdout | Mostly | Mostly | Mostly | Mostly | Mostly |

The hygiene score is **B-** across the lane. The most pressing issues are
F-SCRIPT-01 (false-positive risk), F-SCRIPT-02 (Cargo.lock non-determinism),
and F-SCRIPT-03 (mixed-tab/space Python heredoc).

## Open questions

1. **F-SCRIPT-01 (CRITICAL):** Should the CKB devnet smoke's exit code
   track the composite `all_live_checks_passed` field, or is the
   per-step `[[ ... ]]` + `exit 1` discipline considered sufficient?
   The current design encodes the composite in JSON but does not gate
   the wrapper exit on it. If a CI step pipes `myelin_ckb_devnet_smoke.sh
   && echo OK`, the `OK` is misleading when the JSON says false.

2. **F-SCRIPT-02 (HIGH):** The CKB devnet smoke's `cargo run` does not
   pass `--locked`. Is this intentional (because the smoke is a
   development script, not a release gate), or is it a gap that should
   be closed? The production gate is consistent with `--locked`; the
   smoke is not.

3. **F-SCRIPT-12 (MEDIUM):** The 4 `.template.json` files in
   `docs/templates/public-testnet-rehearsal/` are documented as "shape
   references only". The README claims "the CLI should reject
   unreplaced cryptographic templates" (line 28). Is the CLI's
   rejection behaviour actually implemented? If yes, the templates
   are documentation only. If no, the templates are documentation
   with no enforcement path and should be either removed or wired
   into the prepare/live scripts with explicit operator instructions.

4. **F-SCRIPT-14 (MEDIUM):** The production gate's "recomputed
   production DA readiness evidence" claim (commit `3fda2ab`,
   `cli/src/main.rs:9850-9957`) modifies the CLI but the production
   gate does not assert the recompute's output for the dry-run path
   (it only asserts `real-da-availability-guarantee-missing` is
   present in `end_to_end_production_blockers`). The CKB devnet
   smoke (line 1142, 1147) does assert this blocker. Should the
   production gate's dry-run path also assert this blocker? If the
   CLI's `final_l1_da_availability_preflight_ready` were to silently
   no-op, the production gate would not catch it.

5. **F-SCRIPT-04 (MEDIUM):** The CKB devnet smoke's `WORKDIR` is
   reused if the env var is set. Should the script `rm -rf "$WORKDIR"`
   at start (or at least `$WORKDIR/session-da-store`) to guarantee
   idempotency? The production gate handles this correctly (line
   285). The public-testnet scripts do not handle this at all.

6. **F-SCRIPT-08 (MEDIUM):** Should the CKB devnet smoke's
   `require_cmd` list be expanded to include `cargo`, `od`, `tr`,
   `wc`, `awk`, `seq`, `sed`, `mkdir`, `cp`, `file`, `bash`,
   `printf`? Or is `set -e` propagation sufficient? On a minimal
   Alpine/BusyBox environment, several of these have different
   output formats.

7. **F-SCRIPT-10 (MEDIUM):** The CKB devnet smoke's
   `ALWAYS_SUCCESS_CODE_HASH` is hardcoded. Should the script
   re-derive the code hash from the deployed `always_success` cell
   after `ckb init` rather than trusting a literal? The risk is
   silent failure on a CKB version mismatch.

8. **F-SCRIPT-15 (LOW):** The production gate's `RUN_TEEWORLDS=0` and
   `ALLOW_SKIP_TEEWORLDS=1` allow the Teeworlds acceptance to be
   skipped. Should the gate's exit code reflect this (e.g. emit a
   non-zero "skipped" code)? Currently, `RUN_TEEWORLDS=0
   ALLOW_SKIP_TEEWORLDS=1` produces the same exit 0 as a clean run.

9. **F-SCRIPT-19 (LOW):** Should the CKB devnet smoke validate the
   RPC response shape before the `(( ... ))` arithmetic? A `null`
   tip number from a misconfigured CKB would currently fail with a
   bash arithmetic error, not a clean diagnostic.

10. **F-SCRIPT-25 (LOW):** Should the CKB devnet smoke's `mine()`
    function validate that the chain actually advanced (e.g. by
    comparing `tip_number` before and after)? The current design
    relies on the subsequent `reward_capacity < required_reward_capacity`
    check to catch a stuck miner, but the diagnostic is opaque.

## Per-script hygiene summary

| Script | Lines | `set -e` | `set -u` | `set -o pipefail` | `require_cmd` | `trap` | `rm -rf` cleanup | Idempotent | All `exit 1` only |
|--------|-------|---------|---------|-------------------|---------------|--------|------------------|------------|---------------------|
| `myelin_production_gate.sh` | 1,515 | ✓ | ✓ | ✓ | ✗ | ✗ | ✓ (line 285) | ✓ | n/a (uses `set -e` for most failures) |
| `myelin_ckb_devnet_smoke.sh` | 1,785 | ✓ | ✓ | ✓ | Partial (curl, jq, python3) | ✓ (line 57) | ✗ (workdir only) | ✗ | ✓ (75 sites) |
| `myelin_public_testnet_rehearsal_live.sh` | 286 | ✓ | ✓ | ✓ | Partial (jq) | ✗ | ✗ | ✗ | ✓ (9 sites) |
| `myelin_public_testnet_rehearsal_prepare.sh` | 264 | ✓ | ✓ | ✓ | Partial (jq, seq) | ✗ | ✗ | ✗ | ✓ (1 site) |
| `myelin_teeworlds_acceptance.sh` | 167 | ✓ | ✓ | ✓ | Partial (TEEWORLDS_ROOT files) | ✗ | ✗ | N/A (uses `/tmp/...`) | ✓ (1 site) |
| `build_myelin_teeworlds_repro.py` | 147 | n/a (Python) | n/a | n/a | n/a | n/a | n/a | ✓ (idempotent write) | n/a |

**Findings count by severity:** 1 CRITICAL, 2 HIGH, 13 MEDIUM, 5 LOW, 3 INFO
(F-SCRIPT-26, -27, -28). Total 28 findings (F-SCRIPT-01 through F-SCRIPT-28).
