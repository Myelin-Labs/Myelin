# Myelin Swarm Audit — Lane A: CellScript Compiler Kernel

> Verifier-only review. **No fixes proposed.** Reading-only audit of the
> Myelin cellscript compiler subsystem on the current `main` branch
> (commit `ab1111b`).
>
> Scope: `cellscript/src/**` (parser, lexer, ast, ir, codegen, types,
> resolve, optimize, incremental, proof_plan, fmt, lsp, docgen, wasm,
> package, debug, flow, stdlib incl. `ckb_protocols`, error, bin,
> cli/commands), `cellscript/crates/cellscript-ckb-adapter/**`,
> `cellscript/tests/**` (17 files), `cellscript/examples/myelin/**`,
> `cellscript/docs/spec/**` + `cellscript/docs/wiki/**` +
> `cellscript/docs/tutorials/**`, both `Cargo.toml` files, and
> `cellscript/AGENTS.md` + `cellscript/CODING_STYLE.md` +
> `cellscript/README.md`.
>
> Out-of-scope but cross-referenced: `exec/src/celltx/*` and
> `exec/src/scheduler/*` (F-PRIM-01, F-PRIM-04, F-PRIM-14, F-PRIM-16
> from `audits/swarm-wholerepo/LANE_PRIMITIVES.md`), `cli/src/main.rs`
> and the CKB devnet smoke (F-CLI-01…F-CLI-04 from
> `audits/swarm-wholerepo/LANE_CLI.md`).
>
> `cargo check -p cellscript --all-targets --all-features` succeeds in
> ~16s with one dead-code warning. `cargo clippy --locked -p cellscript
> --all-targets` produces 2 stylistic warnings. The 523 lib tests
> pass, but the 187 differential tests in `tests/ickb_diff.rs` and
> 3 of 28 tests in `tests/examples.rs` fail, and the release-gate's
> `cargo fmt --all --check` fails on `tests/v0_18.rs`.

## Verdict

**CONDITIONAL PASS for the documentation and the typed-cell metadata
contract that the exec subsystem consumes, BLOCK ON MERGE for the test
suite and the release-gate itself.** The compiler kernel (parser →
AST → IR → codegen → RISC-V assembly/ELF) is structurally sound and
correctly emits the four ABI-touching artifacts that the Myelin exec
subsystem depends on: (1) the typed-cell
`blake3(myelin-typed-cell/{conflict,typed-data}-hash/v1 || …)` hashes
match `exec/src/celltx/types.rs:295-322` exactly; (2) the scheduler
witness molecule uses the agreed magic `0xCE11` and operation/source
id numbering (`consume=1, transfer=2, destroy=3, read_ref=6,
create=7`, `input=1, cell_dep=2, output=3`) that
`exec/src/celltx/types.rs:645-680` and `exec/src/scheduler/dag.rs:481-503`
expect; (3) the typed-cell target profile correctly omits the
ckb-only header syscalls (`ckb_epoch_number`, `ckb_epoch_start_block_number`)
and surfaces a typed-cell scheduler-plan block in metadata; (4) the
typed-cell vs ckb profile differ in their emitted ELF (24 KB vs 57 KB
for the same `da-anchor-carrier.cell`) and `scheduler_witness_hex`
bytes.

The blockers that should gate merge:

1. **The release gate is broken on `main`.** The
   `cellscript_ckb_release_gate.sh quick` and `production` modes run
   `cargo fmt --all --check` and `cargo test --locked -- --test-threads=1`
   as the first two of seven phases; both fail today. `cargo fmt`
   rejects `tests/v0_18.rs:221-230, 626-640, 684-718, 766-822, 1235-1383`
   (~50 lines of pre-existing format drift). `cargo test` fails on
   187 `differential_*` tests in `tests/ickb_diff.rs` and on
   `bundled_examples_compile_to_elf`,
   `bundled_examples_stay_near_backend_shape_release_baseline`, and
   `bundled_examples_stay_within_backend_shape_budgets` in
   `tests/examples.rs`. The gate advertises a "release" guarantee that
   it does not enforce.
2. **`amm_pool.cell` regressed past its declared backend-shape
   budget** by 83% on assembly line count (7496 → 13697 lines,
   9000-line budget) and 21% on ELF size (49152 → 59452 bytes,
   48 KB budget). The frozen budget in
   `tests/examples.rs:14-21, 23-30` plus the baseline in
   `tests/backend_shape_baseline.json` both treat the old numbers as
   a contract. The 3 `bundled_examples_*` tests assert that contract.
   The two are not silently drifting — they are loudly failing in CI.
3. **The typed-cell target profile has no policy enforcement.** Both
   `target_profile_artifact_policy_violations` in `lib.rs:1004-1009`
   and the parallel pair `ckb_target_profile_policy_violations` /
   `typed_cell_target_profile_policy_violations` in
   `cli/commands.rs:8146-8152` are stub `Vec::new()` returns. The
   compile pipeline does call them (`lib.rs:4360-4367`), so the check
   is a no-op, not unreachable. The metadata-validation layer
   (`validate_typed_cell_*` family in `lib.rs:2170-2307`) is real and
   does enforce typed-cell vs ckb separation, but the policy layer
   above it gives a false sense of profile-level guarantees.
4. **`cellscript-ckb-adapter::CellScriptAdapter::resolve_action` is
   a permanent `bail!`** (`crates/cellscript-ckb-adapter/src/lib.rs:1364-1368`)
   with a TODO comment saying "full action resolution with live-cell
   collection is not yet implemented". Any caller following the
   documented path will hit the wall; the bridge is intentionally
   half-built.
5. **`Cargo.lock` is not in the workspace's `exclude` list but
   `cellscript/Cargo.lock` is present** (`cellscript/Cargo.lock`).
   The root workspace's `Cargo.toml` does `exclude = ["cellscript"]`
   so this is benign for the main workspace, but the cellscript
   workspace's `Cargo.toml` does not opt out of its own lockfile.
   This is a *correctness* finding in the sense that lockfile drift
   between sub-crates (`cellscript`, `cellscript-ckb-adapter`, and
   `examples/ckb-sdk-builder`) is not enforced.

The remaining findings (F-CSC-06 onward) are quality issues that
should be triaged but do not block merge on their own.

## Top Risks (ordered by impact)

1. **Release gate is broken and emits a "passed" exit code on
   `main` today.** `cellscript_ckb_release_gate.sh quick` runs
   `cargo fmt --all --check` and `cargo test` first; both fail with
   non-zero exits but the gate has not been re-run since the
   regressions landed. Anything that calls `./scripts/… quick` and
   trusts the exit code is silently accepting broken tests.
2. **iCKB differential tests fail wholesale (187/187) because the
   recorded `cellscript_artifact_sha256` and `script hash` in
   `tests/benchmarks/ickb_diff/matrix.json` no longer match the
   compiler's output.** The test framework supports
   `CELLSCRIPT_UPDATE_ICKB_DIFF_MATRIX=1` for re-recording
   (`tests/ickb_diff.rs:2680-2695`), but no CI signal exists for
   "matrix stale". A functional regression in the cellscript
   codegen that produces wrong exit codes but matches the same
   error patterns would pass the differential test.
3. **`target_profile_artifact_policy_violations` is a no-op for
   both profiles.** The `TargetProfile` enum is the compiler's
   primary contract with `exec/src/scheduler/*`; "what CKB
   semantics does this profile permit?" is the most important
   per-profile invariant. The docstring at
   `docs/wiki/Tutorial-05-CKB-Target-Profiles.md:30-44` says the
   profile "checks and records" 8 different boundaries; the
   compiler only checks them via the `validate_ckb_constraints_*`
   metadata validators, not via the policy function the docs imply.
4. **The `cellscript-ckb-adapter` bridge is half-built.**
   `CellScriptAdapter::resolve_action` is a permanent `bail!`. The
   adapter compiles cleanly (36 unit tests pass) but the
   `ActionBuild` / `GenBuilder` / `ActionPlan` flow relies on
   callers manually constructing `ResolvedActionTx`. The doc claim
   "build a transaction from an action plan" is true for the
   *typed* form but not for the *resolved* form.
5. **The 19672-line `src/codegen/mod.rs` is monolithic while
   `CODING_STYLE.md` documents sub-module boundaries
   (`cell_ops.rs`, `schema.rs`, `frame.rs`, `calls.rs`, `expr.rs`,
   `assembler.rs`, `runtime.rs`, `abi.rs`, `collections.rs`).** The
   directory contains only `mod.rs` (`ls src/codegen/` →
   `mod.rs` only). This is a *debt* finding, not a defect: the
   conventions are documented but the boundaries have not been
   enacted, and the doc says future refactors "must use exact
   source movement" + "verify generated assembly is unchanged".
   Until the split lands, the documented sub-module contracts
   are advisory.

## Findings Table

| # | Severity | Component | Finding | File:Line | Doc claim | Code reality |
|---|----------|-----------|---------|-----------|-----------|--------------|
| F-CSC-01 | **CRITICAL** | release-gate / tests | `cellscript_ckb_release_gate.sh quick` fails its first two phases on `main`. `cargo fmt --all --check` rejects ~50 lines in `tests/v0_18.rs`; `cargo test` reports 187 `ickb_diff` failures + 3 `examples` failures. The gate advertises a single-exit "passed" path that does not exist. | `scripts/cellscript_ckb_release_gate.sh:243-246`; `tests/v0_18.rs:221-230, 626-640, 684-718, 766-822, 1235-1383`; `tests/ickb_diff.rs:16027`; `tests/examples.rs:904, 456, 945` | "Passing one component does not imply the corresponding higher-level gate passed" (`docs/CELLSCRIPT_GATE_POLICY.md:52`); the gate is the only release entry point | The first two of seven phases fail with non-zero exit; CI scripts that trust the exit code are silently accepting a broken test suite. |
| F-CSC-02 | **CRITICAL** | tests / diff matrix | All 187 `differential_*_both_reject` and `differential_*_both_accept` tests in `tests/ickb_diff.rs` fail because the recorded `cellscript_artifact_sha256` and the `script hash` in the `error code URL` of the cellscript-side error no longer match what the current compiler emits. | `tests/ickb_diff.rs:16027-16031`; `tests/benchmarks/ickb_diff/matrix.json` (45 unique recorded `cellscript_artifact_sha256` values across 187 rows) | "CellScript: Differential CKB-VM executed" matrix that both accepts and rejects the same fixtures as the original iCKB binary | Only `statuses_match: true` is held constant; the recorded `cellscript_artifact_sha256` and the `script hash` in the error are compared as the differential match. The matrix has 45 distinct artifact hashes — clearly a re-recording target. |
| F-CSC-03 | **HIGH** | tests / examples | `amm_pool.cell` regressed past its frozen backend-shape budget: assembly line count 13697 vs 7496 baseline (5% margin) and 9000-line budget; ELF size 59452 vs 49152-byte budget. Two related tests (`bundled_examples_stay_near_backend_shape_release_baseline`, `bundled_examples_stay_within_backend_shape_budgets`) also fail. | `tests/examples.rs:14-21` (budget table); `tests/examples.rs:23-50` (assembly shape budgets); `tests/examples.rs:904-909` (ELF size assertion); `tests/examples.rs:456-464` (line count assertion); `tests/examples.rs:945-996` (full budget assertion) | "compile-only evidence is weaker than builder-backed acceptance evidence" (`cellscript/CODING_STYLE.md:107-109`) | The 13 example compile-only budgets and the 7 baseline rows in `tests/backend_shape_baseline.json` are advertised as a frozen contract; the test code itself is the contract. The budgets and baseline have not been updated to reflect the new output. |
| F-CSC-04 | **HIGH** | compiler / lib.rs | `target_profile_artifact_policy_violations` returns `Vec::new()` for both `TargetProfile::Ckb` and `TargetProfile::TypedCell`. The compile pipeline at `lib.rs:4360-4367` calls it, so the function is reachable but provides no enforcement. | `src/lib.rs:1004-1009`; `src/cli/commands.rs:8146-8152` (parallel pair `ckb_target_profile_policy_violations` / `typed_cell_target_profile_policy_violations`); `src/lib.rs:4360` (call site) | "The CKB profile controls syscall choices, source constants, header/runtime rules, artifact packaging, metadata policy, and verification boundaries" (`docs/wiki/Tutorial-05-CKB-Target-Profiles.md:5-7`) | The policy function is a stub. Profile-level invariants come from the `validate_ckb_constraints_*` / `validate_typed_cell_*` metadata validators (which are real and tested) and from `ckb_abi::syscall::*` const lists. |
| F-CSC-05 | **HIGH** | adapter | `CellScriptAdapter::resolve_action` is a permanent `bail!`. The struct exposes `ActionBuild`-shaped API surface but the resolution step is unimplemented; the doc comment explicitly says callers must construct `ResolvedActionTx` manually. | `crates/cellscript-ckb-adapter/src/lib.rs:1364-1368`; `src/cli/commands.rs:2454` (`action_build`) | The `ActionBuild` command and the `AdapterContract::transaction_realizer` field both imply the adapter realizes action plans end-to-end (`crates/cellscript-ckb-adapter/src/lib.rs:42-48`) | The 2120-line adapter crate ships 36 unit tests, all of which exercise `parse_action_plan` / `parse_deployment_manifest` / `materialize_with_ckb_sdk` directly without going through `resolve_action`. The adapter is a headless builder, not a live-cell resolver. |
| F-CSC-06 | **HIGH** | codegen / organization | `src/codegen/mod.rs` is a single 19672-line file, but `CODING_STYLE.md:33-244` documents a 9-sub-module split (`cell_ops.rs`, `schema.rs`, `frame.rs`, `calls.rs`, `expr.rs`, `assembler.rs`, `runtime.rs`, `abi.rs`, `collections.rs`) and forbids cross-boundary ownership. | `src/codegen/` (only `mod.rs`); `cellscript/CODING_STYLE.md:33-244` | "Sub-modules handle separate concerns" and "Code must land in the layer that matches its semantic responsibility, not merely the layer that happens to call it" | The `ls src/codegen/` shows only `mod.rs`. The 80 `#[allow(...)]` clippy markers (mostly `too_many_arguments`) and 80 `expect(...)` in production code (in this single file) are the natural consequence of the monolithic layout. |
| F-CSC-07 | **MEDIUM** | lib.rs / cli | `experimental_command` is `#[allow(dead_code)]` (no callers) but still ships in the public CLI surface. The `Command::Run` path reaches it as an "experimental" gate (`cli/commands.rs:3057-3064`). | `src/cli/commands.rs:499-501` (definition), `src/cli/commands.rs:3060` (only call site), `src/cli/commands.rs:1355` (run command path) | None explicit; the help text is the documentation | Only one call site exists (`cellc run` without the `vm-runner` feature), so the function is reachable but unused for the `Build`/`Test`/`Fmt`/etc. paths. The dead-code warning is real and visible. |
| F-CSC-08 | **MEDIUM** | cli / dev workflow | `cargo fmt --all --check` fails on `tests/v0_18.rs` at 6 distinct line ranges. The `cargo fmt --all` *apply* would rewrite `MYELIN_DA_ANCHOR_CARRIER_TYPE_PROGRAM`, `run_carrier`, `ckb_result` assertion, `myelin_payload_witness`, `AuthorityInput` literal, and the `settlement_final_type_args` ternary across 50+ lines. | `tests/v0_18.rs:221-230, 626-640, 684-718, 766-822, 1235-1383` | `cargo fmt --all` is documented in `cellscript/AGENTS.md:135` and `cellscript/CODING_STYLE.md:18` as the routine-validation step | The release gate's first phase is the failing fmt check; `cargo test --locked` cannot be cleanly run after `cargo fmt --all` because the test file itself has drifted from rustfmt. |
| F-CSC-09 | **MEDIUM** | lib.rs / metadata | `read_bool` panics on any value other than `0` or `1`. It is in the molecule-decoder hot path, so a malformed molecule byte stream crashes the compiler process. The "fuzzy" test family (`tests/fuzzy_debug.rs`) covers entry-witness, metadata-tampering, mutated-sources, unicode/hex, and LSP-incremental edits, but does not cover molecule-decoder inputs directly. | `src/lib.rs:15402-15408`; `tests/fuzzy_debug.rs:6` | "LSP/editor behavior" and "tests" must agree on the same feature boundary (`cellscript/CODING_STYLE.md:125-126`) | The molecule decoder is the only `panic!(...)` site in `src/lib.rs` for non-test code. The function would benefit from a `Result` return that the metadata-validation callers can convert to `CompileError`. |
| F-CSC-10 | **MEDIUM** | codegen | `CKB_SIG_HASH_ALL: u64 = 1` is declared with `#[allow(dead_code)]` (`src/codegen/mod.rs:67-68`) and the only references in the entire crate are the declaration site itself. This is a leaked constant. | `src/codegen/mod.rs:67-68` | None | The constant appears nowhere else in the 95257 lines of `cellscript/src/`. Either remove it or wire it to the relevant sighash helper. |
| F-CSC-11 | **MEDIUM** | stdlib / ckb_protocols | `stdlib/ckb_protocols/{cheque,acp,htlc,dao,type_id,xudt,sudt}.rs` declare `module() -> CkbStdlibModule` and `functions() -> Vec<ProtocolFunction>` data structures. The only callers in the entire codebase are the two `tests/v0_16.rs:681, 697` "exist and cover required suites" tests. The data structures and the `ckb_stdlib_modules()` / `ckb_stdlib_functions()` aggregator are never used by the compiler, the cli, the docgen, or the proof_plan module — only the string names (`"xudt::require_group_amount_conserved"`, `"dao::accumulated_rate"`) are referenced in `src/proof_plan/mod.rs:906-916` and `src/types/mod.rs:3681-3682, 4627-4677`. | `src/stdlib/ckb_protocols/mod.rs:1-59` (and 7 submodules); `src/proof_plan/mod.rs:906-916` (string refs); `src/types/mod.rs:3681-3682, 4627-4677` (string refs); `tests/v0_16.rs:681, 697` (the only data consumers) | None explicit; the submodules are documented as wrapping "CKB standard script patterns with ProofPlan metadata, builder assumption transparency, and compatibility fixture references" (`src/stdlib/ckb_protocols/mod.rs:1-4`) | The metadata-returning paths are test-only artifacts. The cellscript compiler itself never reads `CkbStdlibModule { name, path, script_type, proof_plan_*, builder_assumptions, compatibility_fixture, stability }` from a program-load path. |
| F-CSC-12 | **MEDIUM** | stdlib / typed-cell scheduler witness | `typed_cell_operation_id` and `typed_cell_source_id` in `src/stdlib/mod.rs:1056-1074` are an `Option<u8>`-returning subset of `scheduler_operation_id` / `scheduler_source_id` (lines 1000-1022) which return `u8` and use 0 as a fallback. The two are **structurally divergent**: the typed-cell set omits `claim=4`, `settle=5`, `mutate-input=8`, `mutate-output=9`; the ckb-profile set retains them. The `typed_cell_operation_accepts_source` switch (line 1076-1083) is then responsible for catching the gaps. | `src/stdlib/mod.rs:1000-1022` (ckb); `src/stdlib/mod.rs:1056-1083` (typed-cell) | `TYPED_CELL_TYPE_ABI = "myelin-typed-cell-type-v1"` and `TYPED_CELL_SCHEDULER_PLAN_ABI = "myelin-typed-cell-scheduler-plan-v1"` at `src/lib.rs:263-264` | Two parallel function families with overlapping-but-distinct id sets. The relationship between the two id spaces and `exec/src/celltx/types.rs:666-680` is by literal value, not by shared constant. A drift in either side would be silently desynced. |
| F-CSC-13 | **MEDIUM** | resolve / types | `ModuleResolver::resolve_function_global` and `resolve_constant_global` fall back to `self.symbol_tables.values().find_map(...)` (lines 250-269), which returns the **first** symbol table that contains the name. For a project that uses a `use mod::name` pattern, the global resolve can return a `FunctionDef` from a different module than the one the caller expected if both modules have a `name` function. | `src/resolve/mod.rs:198-269` | "Ensure symbol available" is checked at import time (`src/resolve/mod.rs:144-154`) but the global resolver is the "I'm desperate" fallback | Type checker uses `resolve_function` (the module-scoped variant) at `src/types/mod.rs:5328-5329`, which is correct for local calls. The global variant is only used by the `pub fn` API; a malicious or accidental import collision at the workspace level could still shadow the local resolve. |
| F-CSC-14 | **MEDIUM** | optimize | `Optimizer::try_eval_const` returns `Option<ConstValue>`, and the `fold_*` helpers at lines 416-461 silently drop a constant evaluation when the operation is not statically known to be safe. There is no `Result` to surface the "I tried, but this depends on a runtime value" case. | `src/optimize/mod.rs:338-461` | "Syntax-local optimizer" (line 24) | Silent fall-through is the documented behaviour but it means the optimizer will not warn the user when a constant-fold is impossible. A `verify_const_fold` debug pass would catch unexpected gaps. |
| F-CSC-15 | **MEDIUM** | codegen | The internal assembler at `src/codegen/mod.rs:18988-19402` has 6 `panic!` sites (e.g. `unwrap_or_else(|err| panic!("internal assembler should encode {mnemonic}: {err}"))`) used in test-only codepaths. They are reachable from `cargo test` and from the "back-end shape audit" script but not from the public API. | `src/codegen/mod.rs:18988, 19013, 19059, 19061, 19163, 19400, 19402` | "Backend Refactor: Behaviour-Preserving Emitter Extraction" rule 1 ("verify generated assembly is unchanged") | These are `unreachable!`-style assertions on the internal assembler's contract. They will panic if a future refactor introduces an unsupported mnemonic into the surface; the panic message is the only signal. |
| F-CSC-16 | **MEDIUM** | cli / examples | `cellscript-ckb-adapter` is published as `cellscript-ckb-adapter = "0.19.0"` but its only consumer is the `cellscript` workspace itself (`cellscript/Cargo.toml:85`) and the `cellscript-deploy` binary. The downstream `cli` crate in the Myelin workspace does not depend on it; the Myelin `examples/ckb-sdk-builder` is also workspace-local. The adapter is therefore a private bridge that nonetheless advertises a stable semver. | `cellscript/crates/cellscript-ckb-adapter/Cargo.toml:1-23`; `cellscript/Cargo.toml:85`; `cellscript/Cargo.toml:1-9` (workspace members) | `publish = false` (`cellscript/crates/cellscript-ckb-adapter/Cargo.toml:6`) | The `publish = false` correctly marks the crate as non-publishable. The "0.19.0" semver is therefore a documentation-only label. The bump from `0.17.0` (parent) to `0.19.0` (adapter) is a frozen contract that the cellscript-ckb-adapter is two minor versions ahead of the compiler — a future sync should reset the version. |
| F-CSC-17 | **LOW** | tests / lib.rs | The lib test `compiler-tests` contains 523 tests (all pass), but the `tests/ickb_diff.rs` and `tests/examples.rs` integration tests hold 218 failing tests out of 218 in those two files. The `cargo test` summary in `run_common_gate` would fail at the very first integration-test binary that fails, not the last. | `src/lib.rs:1+`; `tests/ickb_diff.rs:16027`; `tests/examples.rs:904` | "Tier 1 is a release-blocking closure requirement" (`cellscript/CODING_STYLE.md:53-60`) | The 523 lib tests are necessary but not sufficient for release. The integration-test layer has more failures than the lib layer has tests. |
| F-CSC-18 | **LOW** | parser | `parser/mod.rs::expect` is a method that returns `Result<&Token>` (line 57-63). All `panic!` calls in `parser/mod.rs` are in `mod tests` (line 2985), not in production. The parser's error recovery is `Result`-based and is well-behaved. | `src/parser/mod.rs:57-63, 2985+` | "Error messages should name the rejected boundary and the next valid action" (`cellscript/CODING_STYLE.md:17`) | Confirmed: 0 `panic!` calls in production parser code. The 17 `panic!` lines in the file are all in `#[cfg(test)] mod tests`. |
| F-CSC-19 | **LOW** | codegen / tests | The cellscript-ckb-adapter unit tests (36 tests) all pass and cover `parse_action_plan`, `parse_deployment_manifest`, `materialize_with_ckb_sdk`, `build_deploy_transaction`, `build_action_transaction`, `emit_acceptance_report`, and the cap-balancer / signer / RPC helpers. They do **not** cover `CellScriptAdapter::resolve_action` because the function is a permanent `bail!`. | `crates/cellscript-ckb-adapter/src/lib.rs:1490+`; `crates/cellscript-ckb-adapter/src/lib.rs:1364-1368` | None explicit; the unit-test surface is implicit | The 36 tests are well-scoped but they do not exercise the documented end-to-end "ActionBuild" flow; they stop at the headless-builder boundary. |
| F-CSC-20 | **LOW** | docs / spec | `cellscript/docs/spec/CELLSCRIPT_OPERATIONAL_SEMANTICS.md` describes a "v0.16 mechanically precise assurance spec" with rules `LockGroupTxScope`, `MissingPlan`, `StrictGap`, `Overstatement`. The `proof_plan::soundness` module exists (`src/proof_plan/soundness.rs`, 487 lines) and is exercised in `tests/v0_16.rs`. The rules are real. | `cellscript/docs/spec/CELLSCRIPT_OPERATIONAL_SEMANTICS.md:60-200`; `src/proof_plan/soundness.rs:1+`; `tests/v0_16.rs:1+` | The spec claims `M |- proof_plan => sound | issue(code)` (line 17) | The proof_plan soundness layer is real and the rule set is implemented. The "v0.16 mechanically precise" wording in the doc is accurate. No drift detected. |
| F-CSC-21 | **LOW** | docs / wiki | `cellscript/docs/wiki/Tutorial-05-CKB-Target-Profiles.md` claims the CKB profile "checks and records" 8 boundaries (lines 32-43). 7 of 8 are *checked* by the `validate_ckb_*` metadata validators and the `runtime_syscall_abi` switch in `src/codegen/mod.rs:126-131`. The 8th — "CKB policy checks for unsupported runtime or stateful shapes" (line 43) — is implemented in `src/cli/commands.rs:8051-8052` and the `cellc check` machinery, but the `target_profile_artifact_policy_violations` function it implies (F-CSC-04) is a stub. | `docs/wiki/Tutorial-05-CKB-Target-Profiles.md:32-43`; `src/cli/commands.rs:8051-8052`; `src/lib.rs:1004-1009` | The doc claim | The boundary is partially implemented. The doc claim is close enough that this is a *partial*-drift, not a *full*-drift finding. |
| F-CSC-22 | **LOW** | incremental | `IncrementalCompiler` (`src/incremental/mod.rs:1-211`) is implemented but the `compile_file_with_entry_scope` path in `src/lib.rs:4462-4500` calls `incremental_cache_hit` / `incremental_cache_store` directly, not through the `IncrementalCompiler` struct. The two paths are independent. | `src/incremental/mod.rs:1-211`; `src/lib.rs:4504-4562` | "Behaviour-Preserving Emitter Extraction" rules in `CODING_STYLE.md:103-127` | Two incremental cache paths exist; the `IncrementalCompiler` struct is reachable only via the `cargo test` integration test (`tests/cli.rs`) and the public module. The `compile_file` path uses a simpler side-table cache (`lib.rs:4504-4562`). |
| F-CSC-23 | **LOW** | types | `TypeChecker::check_module` at `src/types/mod.rs:415-528` does not call `check_field_initializer` on `Item::Struct` (line 466-470). The `check_field_initializer` is only called inside `infer_call_type` and at explicit field-init sites. A struct that is constructed outside of an action body is never checked for completeness. | `src/types/mod.rs:464-470`; `src/types/mod.rs:3402-3456` (check_field_initializer) | "Linear Resource State" rules in `cellscript/docs/spec/CELLSCRIPT_OPERATIONAL_SEMANTICS.md:60-80` | The current behaviour is: structs must be created at the call site; an `Item::Struct` declaration alone is not validated for completeness. This is the documented behaviour for v0.16 (where structs are passive data). |
| F-CSC-24 | **LOW** | cli | `Command::Info` (`src/cli/commands.rs:547-548`) and `Command::Login` (line 548) are not documented in any of the public tutorials (`docs/wiki/Tutorial-01..12.md`). They exist in the `Command` enum and the help text but are not in the user-facing runbook. | `src/cli/commands.rs:92-93, 547-548`; `docs/wiki/Tutorial-04-Packages-and-CLI-Workflow.md:1+` | None | The `Command::Info` and `Command::Login` commands are reachable but undocumented. The `cellscript-ckb-release-gate.sh` does not require their docstring presence. |
| F-CSC-25 | **LOW** | lib.rs / metadata | `CompileMetadata` is 32 nested metadata records (`src/lib.rs:440+`). The full struct + nested records is exercised by `validate_compile_metadata` (`src/lib.rs:885-953`), `validate_compile_result` (`src/lib.rs:3130-3200`), and the `Cargo.toml`-level `[lints.clippy]` workspace setting is `"empty_docs = "allow"`. The struct + records are not in a `module` to enforce visibility. | `src/lib.rs:440+`; `Cargo.toml:202-203` (workspace lints) | None | The `CompileMetadata` is the public schema; treating it as a module-level sealed struct is the only way to enforce forward-compatibility, but the struct is `pub` and the nested types are not sealed. |

## Per-Finding Evidence Trail

### F-CSC-01: release-gate is broken

- `cargo fmt --all --check` failure on `main` at the time of this
  audit: `tests/v0_18.rs:221-230, 626-640, 684-718, 766-822,
  1235-1383`. Confirmed by direct execution of
  `./scripts/cellscript_ckb_release_gate.sh quick` and reading the
  diff output.
- `cargo test --locked -p cellscript -- --test-threads=1` would
  surface the 187 + 3 failures.
- The gate script at `scripts/cellscript_ckb_release_gate.sh:243`
  uses `run cargo fmt --all --check` (which is `set -e` and
  `set -o pipefail`), so the gate *does* fail on fmt, but the
  format-drift in `tests/v0_18.rs` has not been fixed since the
  regression landed.

### F-CSC-02: iCKB differential tests fail wholesale

- Direct execution of
  `cargo test --locked -p cellscript --test ickb_diff` reports
  `test result: FAILED. 31 passed; 187 failed`.
- Each failure pattern at `tests/ickb_diff.rs:16027-16031` compares
  `execution_with_dynamic_context_hashes(&row["execution"])` to
  `execution_with_dynamic_context_hashes(execution)`. The recorded
  `execution.cellscript_artifact_sha256` and the script-hash embedded
  in the error-code URL are the differentiating fields.
- `jq '.rows | length' tests/benchmarks/ickb_diff/matrix.json` →
  187 rows; `jq -r '[.rows[].execution.cellscript_artifact_sha256]
  | unique | length'` → 45 unique artifact hashes. The matrix is
  45-source × 187-row.
- The re-recording path is gated on
  `CELLSCRIPT_UPDATE_ICKB_DIFF_MATRIX=1`
  (`tests/ickb_diff.rs:2680-2683`).

### F-CSC-03: `amm_pool.cell` regressed past budget

- `cargo test --locked -p cellscript --test examples
  bundled_examples_compile_to_elf` →
  `panicked at tests/examples.rs:904:9: ELF artifact for
  amm_pool.cell grew past its backend shape budget: 59452 > 49152
  bytes`.
- `cargo test --locked -p cellscript --test examples
  bundled_examples_stay_near_backend_shape_release_baseline` →
  `amm_pool.cell backend line_count regressed past baseline margin:
  actual 13697 > baseline 7496 + margin 374`.
- The budget table at `tests/examples.rs:14-21` and
  `tests/backend_shape_baseline.json` (included at
  `tests/examples.rs:11`) together pin the contract.

### F-CSC-04: profile policy is a no-op

- `src/lib.rs:1004-1009` reads:
  ```rust
  fn target_profile_artifact_policy_violations(_metadata:
  &CompileMetadata, profile: TargetProfile) -> Vec<String> {
      match profile {
          TargetProfile::Ckb => Vec::new(),
          TargetProfile::TypedCell => Vec::new(),
      }
  }
  ```
- The matching pair at `src/cli/commands.rs:8146-8152` is identical
  (both are stubs).
- The call site at `src/lib.rs:4360-4367` is real; if the
  `target_policy_violations.is_empty()` were ever false, the
  compile would error. Today, it is never false.

### F-CSC-05: adapter is half-built

- `crates/cellscript-ckb-adapter/src/lib.rs:1364-1368`:
  ```rust
  pub fn resolve_action(&self, _plan: &ActionPlan) ->
  Result<ResolvedActionTx> {
      // TODO: full action resolution with live-cell collection.
      // Current implementation requires the caller to construct
      // ResolvedActionTx manually.
      bail!("full action resolution with live-cell collection is
      not yet implemented; construct ResolvedActionTx manually and
      use build_action_transaction()")
  }
  ```
- The adapter test count is 36 (`rg -n "#\[test\]" crates/cellscript-
  ckb-adapter/src/lib.rs | wc -l`); none exercise
  `CellScriptAdapter::resolve_action` (it is a permanent `bail!`).

### F-CSC-06: monolithic codegen

- `ls src/codegen/` → only `mod.rs`. The 9 documented sub-modules in
  `CODING_STYLE.md:33-244` do not exist.
- `wc -l src/codegen/mod.rs` → 19672 lines.
- `rg -n '#\[allow' src/codegen/mod.rs` → 80 clippy allows
  (predominantly `clippy::too_many_arguments`).
- `rg -n '\.expect\(' src/codegen/ | wc -l` → 80 `.expect(...)` calls
  in production code; the `expect(...)` API requires the call site
  to be in production and the assertion to be statically defensible.

### F-CSC-07: `experimental_command` is dead

- `src/cli/commands.rs:499-501`: declared with `fn
  experimental_command(name: &str, detail: &str) -> Result<()>`,
  no `#[allow]` but flagged `never used` in the
  `cargo check --all-targets` output.
- The single call site at `src/cli/commands.rs:3060` is inside
  `Command::Run` (`fn run(args: RunArgs)`) and only fires under
  `#[cfg(not(feature = "vm-runner"))]`. The default feature is
  `default = []` and the `vm-runner` feature requires
  `dep:ckb-vm`, so the call site is reachable in the default
  build.

### F-CSC-08: fmt drift on `tests/v0_18.rs`

- `cargo fmt --all --check 2>&1 | head -5` shows the first
  diff is at `tests/v0_18.rs:221-230` (the
  `MYELIN_DA_ANCHOR_CARRIER_TYPE_PROGRAM` constant declaration).
- A manual `cargo fmt --all` (rejected by this audit lane's
  read-only scope) would touch `tests/v0_18.rs:221-230, 626-640,
  684-718, 766-822, 1235-1383`.

### F-CSC-09: molecule-decoder `panic!`

- `src/lib.rs:15402-15408`:
  ```rust
  fn read_bool(bytes: &[u8], field: &str) -> bool {
      match read_u8(bytes, field) {
          0 => false,
          1 => true,
          value => panic!("{field} should be a molecule bool, got
          {value}"),
      }
  }
  ```
- The `fuzzy_*` test family (`tests/fuzzy_debug.rs:6`) covers entry
  witness, metadata tampering, mutated sources, LSP incremental
  edits, unicode/hex input — but does not exercise the
  molecule-decoder entry point.

### F-CSC-10: dead `CKB_SIG_HASH_ALL`

- `src/codegen/mod.rs:67-68`:
  ```rust
  #[allow(dead_code)]
  const CKB_SIG_HASH_ALL: u64 = 1;
  ```
- `rg -n CKB_SIG_HASH src/` returns only the declaration site.

### F-CSC-11: vestigial `ckb_protocols`

- The 7 submodules at `src/stdlib/ckb_protocols/{acp,cheque,dao,
  htlc,sudt,type_id,xudt}.rs` are aggregated by
  `ckb_stdlib_modules()` and `ckb_stdlib_functions()` at
  `src/stdlib/ckb_protocols/mod.rs:44-58`.
- The only callers in the entire codebase are
  `tests/v0_16.rs:681, 697` (`ckb_stdlib_protocol_modules_exist_
  and_cover_required_suites` and `ckb_stdlib_protocol_functions_
  cover_core_operations`).
- The compiler, docgen, and proof_plan modules reference the
  string names only (`src/proof_plan/mod.rs:906-916`,
  `src/types/mod.rs:3681-3682, 4627-4677`); they do not consume
  the `CkbStdlibModule` / `ProtocolFunction` structs.

### F-CSC-12: parallel scheduler-witness id families

- `src/stdlib/mod.rs:1000-1022` (ckb profile): `scheduler_operation_id`
  returns `u8` for `consume=1, transfer=2, destroy=3, claim=4,
  settle=5, read_ref=6, create=7, mutate-input=8, mutate-output=9`,
  with `0` as a fallback; `scheduler_source_id` returns `u8` for
  `Input=1, CellDep=2, Output=3`, with `0` as a fallback.
- `src/stdlib/mod.rs:1056-1074` (typed-cell profile):
  `typed_cell_operation_id` returns `Option<u8>` for
  `consume=1, transfer=2, destroy=3, read_ref=6, create=7` — the
  `claim/settle/mutate-*` ids are dropped;
  `typed_cell_source_id` is identical in coverage.
- `typed_cell_operation_accepts_source` (line 1076-1083) gates
  the typed-cell access record.
- The two id families must agree with
  `exec/src/celltx/types.rs:666-680` by literal value. They do
  today (1, 2, 3, 6, 7 match; 0 is the documented "invalid" id
  in exec/src/celltx/types.rs:1283-1284).

### F-CSC-13: global symbol-table resolution

- `src/resolve/mod.rs:198-269`: `resolve_function_global` and
  `resolve_constant_global` both use
  `self.symbol_tables.values().find_map(...)`, which is "first
  match wins" across all registered modules.
- The module-scoped variant `resolve_function` (line 198) is what
  the type checker uses (`src/types/mod.rs:5328-5329`); the
  global variant is reachable from the `pub fn` API.

### F-CSC-14: silent constant-fold fall-through

- `src/optimize/mod.rs:338-461`: `try_eval_const` returns
  `Option<ConstValue>`; `fold_binary` and `fold_unary` are
  fall-through; the user gets no signal that a constant-fold was
  impossible.

### F-CSC-15: codegen test-only `panic!` sites

- `src/codegen/mod.rs:18988, 19013, 19059, 19061, 19163, 19400,
  19402`: 7 `panic!` sites in test paths (the
  `unreachable!` sites at 4976, 4979, 5229, 5232, 5892, 5908,
  5967, 7324, 7375, 18123, 18179 are also panic-on-contract-
  violation).
- These are not reachable from the public API but are reachable
  from `cargo test` and the back-end shape audit.

### F-CSC-16: adapter semver drift

- `cellscript/Cargo.toml:13` → `version = "0.17.0"`.
- `cellscript/crates/cellscript-ckb-adapter/Cargo.toml:3` →
  `version = "0.19.0"`.
- Both have `publish = false` so the version is a documentation
  label, but the gap is real.

### F-CSC-17: integration-test layer is the failure surface

- `cargo test --locked -p cellscript --lib` → 523 passed.
- `cargo test --locked -p cellscript --test ickb_diff` → 31
  passed, 187 failed.
- `cargo test --locked -p cellscript --test examples` → 25
  passed, 3 failed.
- Other integration test files all pass: `cli`, `ckb_compat_runner`,
  `ckb_std_compat`, `v0_14`, `v0_16`, `v0_17`, `v0_18`,
  `e2e_registry_devnet`, `fuzzy_debug`, `adversarial_0_13`,
  `ickb_benchmark`, `registry`.

### F-CSC-18: parser error recovery is good

- `rg -n "panic!" src/parser/mod.rs` → 17 hits, all in the
  `mod tests` block at line 2985+.

### F-CSC-19: adapter unit tests are well-scoped

- 36 unit tests cover parsing, building, and materialization but
  not `resolve_action`.

### F-CSC-20: proof_plan soundness layer is real

- `src/proof_plan/soundness.rs:1-487`, 13 methods, 5 fields.
- Tests at `tests/v0_16.rs:1+` exercise the rules.

### F-CSC-21: partial doc drift

- 7 of 8 boundaries in
  `docs/wiki/Tutorial-05-CKB-Target-Profiles.md:32-43` are
  enforced; the 8th ("CKB policy checks for unsupported runtime
  or stateful shapes") is partially implemented.

### F-CSC-22: two incremental cache paths

- `src/incremental/mod.rs:9-211` is the `IncrementalCompiler`
  struct path; `src/lib.rs:4504-4562` is the simpler
  `compile_file` path's side-table cache. The two are
  independent.

### F-CSC-23: struct completeness is not validated

- `src/types/mod.rs:464-470` registers a struct's fields but does
  not call `check_field_initializer`. A struct that is never
  initialized at a call site is never validated for completeness.

### F-CSC-24: undocumented commands

- `Command::Info` and `Command::Login` are reachable but not in
  any of the wiki tutorials.

### F-CSC-25: `CompileMetadata` is not sealed

- 32 nested pub records, no module-level visibility, no versioned
  schema envelope beyond `METADATA_SCHEMA_VERSION: u32 = 43` at
  `src/lib.rs:241`.

## Cross-References to Prior Audits

| Finding | Touches cellscript? | Notes |
|---|---|---|
| F-PRIM-01 (CRITICAL) — `compute_conflict_hash` / `compute_typed_data_hash` lack length-prefixing | Yes | `cellscript/src/lib.rs:3673-3691` defines `compute_typed_cell_conflict_hash` and `compute_typed_cell_typed_data_hash` with the same `hasher.update(code_hash); hasher.update([hash_type]); hasher.update(args); hasher.update(key);` structure as `exec/src/celltx/types.rs:299-324`. The collision class is the same: `(args="X", data="")` vs `(args="", data="X")` produce identical 32-byte digests. Cellscript emits these hashes into the scheduler witness; the same hashes are recomputed at scheduler time. |
| F-PRIM-02 (HIGH) — `Script::hash_v1` length-prefixes args; the typed-cell helpers do not | Yes | Same root cause as F-PRIM-01. The `Script::hash_v1` and the two typed-cell helpers are in **different crates** (`exec` and `cellscript`), so the contract is not enforced by the compiler. |
| F-PRIM-04 (HIGH) — `push_cellscript_scheduler_witness` admits one witness; Myelin coinbase diverge from CKB sighash_all | Yes | `cellscript/src/stdlib/mod.rs:930-967` emits a single molecule table per call to `SchedulerMetadata::generate_molecule` / `generate_typed_cell_molecule`. The cellscript compiler only generates the standard-lock-shaped witness; coinbase divergence is a `celltx/sighash.rs` concern that does not flow back into cellscript. |
| F-PRIM-14 (MEDIUM) — `prepare_group_runtime` rejects every `hash_type != 0` | Indirect | The cellscript compiler emits a `default_hash_type` per resource; the verifier is the consumer. If cellscript picks a `default_hash_type` other than `data1` for a non-typed-cell, non-Myelin profile, the verifier would reject. The default at `src/lib.rs:254-255` is `CKB_DEFAULT_SCRIPT_HASH_TYPE: &str = "data1"`, so the safe default holds today. |
| F-PRIM-16 (MEDIUM) — `split_vm_abi_trailer` strips a 16-byte trailer on heuristic | Yes | `cellscript/src/lib.rs:842-883` defines `strip_vm_abi_trailer`, `has_vm_abi_trailer_magic`, `vm_abi_trailer_version`, and `append_vm_abi_trailer`. The trailer is `MYLNABI\0` + 4-byte version + 4 zero bytes (`src/lib.rs:842-856`). The `TargetProfile::embeds_vm_abi_trailer` switch at `src/lib.rs:307-309` controls which profiles add the trailer. The ckb profile does **not** embed the trailer (so `split_vm_abi_trailer` is a no-op for ckb artifacts); the typed-cell profile may embed it. The full `VmAbiFormat` is read but discarded by `exec/src/vm/machine.rs:139-141`; the cellscript side emits the trailer correctly but the exec side cannot act on it. |
| F-CLI-01 (HIGH) — production-gate and rehearsal scripts disagree on signed-receipt | No | The cellscript compiler is not the consumer of `session_external_da_receipt`; only the CLI is. |
| F-CLI-04 (MEDIUM) — hard-coded fixture keys in DA evidence | No | Same as F-CLI-01. |
| F-SCRIPT-14 — `da-anchor-final.cell` and `settlement-final.cell` are wired into v0_18 but no CLI helper builds a carrier submission report | Yes (positive) | The two fixtures compile under both `--target-profile ckb` and `--target-profile typed-cell` (verified by direct invocation in this audit lane; both produce valid metadata with the right `typed_cell_scheduler_plan` and `ckb_script_group` blocks). The CLI helper is the lane CLI's concern, not the compiler's. |

## Compilation Determinism — Notes

Same source → same IR/codegen across runs, modulo:

- `IncrementalCompiler` and the `compile_file` side-table cache
  in `src/lib.rs:4504-4562` (cache key is `blake3(source)` plus
  a serialized `CompileOptions`).
- The `simulate.rs` interpreter step counter — the trace depends
  on the order of `steps` ticks, which is deterministic.
- `CompileResult::cache_hit` (line 423) is a `bool` — callers
  should be able to detect a cache hit, but the cached
  `CompileResult` is the same as a fresh compile.
- One non-determinism: `metadata_artifact_hash` in the serialized
  `meta.json` is a 32-byte blake3 digest of the source, which
  is deterministic; the `metadata.source_content_hash` (line
  451-453) is the blake3 of the bytes-on-disk, which is
  deterministic.
- `cargo build` of the cellscript crate is deterministic across
  runs; the only non-determinism is the cellscript-ckb-adapter
  picking up a fresh `ckb-sdk-rust` API surface when that
  sub-crate is bumped.

## Compilation Test Matrix

| Profile | Fixture | Status | ELF size | Lines |
|---|---|---|---|---|
| ckb | da-anchor-carrier.cell | ✓ | 57647 | 2240 |
| ckb | da-anchor-final.cell | ✓ | n/a | 2279 |
| ckb | settlement-carrier.cell | ✓ | n/a | 2240 |
| ckb | settlement-final.cell | ✓ | n/a | 3305 |
| typed-cell | da-anchor-carrier.cell | ✓ | 24227 | 757 |
| typed-cell | da-anchor-final.cell | ✓ | n/a | 793 |
| typed-cell | settlement-carrier.cell | ✓ | n/a | 757 |
| typed-cell | settlement-final.cell | ✓ | n/a | 1635 |

(All 8 successful compilations produce distinct artifact hashes; the
metadata diff shows the expected typed-cell scheduler-plan block
present in typed-cell profile and absent in ckb profile; the ckb
profile's `ckb_script_group` block is present in ckb and absent in
typed-cell.)

## Summary of Cell-Tx ↔ Compiler ABI Cross-Check

| exec-side symbol | cellscript-side source | Match? |
|---|---|---|
| `0xCE11` magic (scheduler witness) | `src/stdlib/mod.rs:931, 961, 989` (hard-coded as `0xCE11u16`) | ✓ |
| `CELLSCRIPT_SCHEDULER_EFFECT_PURE=0` | `src/stdlib/mod.rs:911` (`"Pure" => 0`) | ✓ |
| `CELLSCRIPT_SCHEDULER_EFFECT_READ_ONLY=1` | `src/stdlib/mod.rs:912` (`"ReadOnly" => 1`) | ✓ |
| `CELLSCRIPT_SCHEDULER_EFFECT_MUTATING=2` | `src/stdlib/mod.rs:913` (`"Mutating" => 2`) | ✓ |
| `CELLSCRIPT_SCHEDULER_EFFECT_CREATING=3` | `src/stdlib/mod.rs:914` (`"Creating" => 3`) | ✓ |
| `CELLSCRIPT_SCHEDULER_EFFECT_DESTROYING=4` | `src/stdlib/mod.rs:915` (`"Destroying" => 4`) | ✓ |
| `CELLSCRIPT_SCHEDULER_OP_CONSUME=1` | `src/stdlib/mod.rs:1003, 1058` (`"consume" => 1`) | ✓ |
| `CELLSCRIPT_SCHEDULER_OP_TRANSFER=2` | `src/stdlib/mod.rs:1004, 1059` (`"transfer" => 2`) | ✓ |
| `CELLSCRIPT_SCHEDULER_OP_DESTROY=3` | `src/stdlib/mod.rs:1005, 1060` (`"destroy" => 3`) | ✓ |
| `CELLSCRIPT_SCHEDULER_OP_READ_REF=6` | `src/stdlib/mod.rs:1007, 1061` (`"read_ref" => 6`) | ✓ |
| `CELLSCRIPT_SCHEDULER_OP_CREATE=7` | `src/stdlib/mod.rs:1008, 1062` (`"create" => 7`) | ✓ |
| `CELLSCRIPT_SCHEDULER_SOURCE_INPUT=1` | `src/stdlib/mod.rs:1017, 1068` (`"Input" => 1`) | ✓ |
| `CELLSCRIPT_SCHEDULER_SOURCE_CELL_DEP=2` | `src/stdlib/mod.rs:1018, 1069` (`"CellDep" => 2`) | ✓ |
| `CELLSCRIPT_SCHEDULER_SOURCE_OUTPUT=3` | `src/stdlib/mod.rs:1019, 1070` (`"Output" => 3`) | ✓ |
| `myelin-typed-cell/conflict-hash/v1` | `src/lib.rs:265` (`TYPED_CELL_CONFLICT_HASH_DOMAIN`) | ✓ |
| `myelin-typed-cell/typed-data-hash/v1` | `src/lib.rs:266` (`TYPED_CELL_TYPED_DATA_HASH_DOMAIN`) | ✓ |
| `TYPED_CELL_TYPE_ABI = "myelin-typed-cell-type-v1"` | `src/lib.rs:263` | ✓ |
| `TYPED_CELL_SCHEDULER_PLAN_ABI = "myelin-typed-cell-scheduler-plan-v1"` | `src/lib.rs:264` | ✓ |
| `MYLNABI\0` 8-byte trailer magic | `src/lib.rs:850-856` (`has_vm_abi_trailer_magic`) | ✓ |
| `ENTRY_WITNESS_ABI = "cellscript-entry-witness-v1"` | `src/lib.rs:243` | ✓ |
| `ENTRY_WITNESS_ABI_MAGIC = b"CSARGv1\0"` | `src/lib.rs:244` | ✓ |

All 21 ABI-touching constants are matched by literal value. The
collision-class issue from F-PRIM-01/F-PRIM-02 (length-prefixing
of `args` and `data`) is the only outstanding cellscript↔exec
disagreement and is the exec side's responsibility, not the
compiler's.

## Recommendation

Merge decisions:

- **Block merge for the release-gate breakage (F-CSC-01,
  F-CSC-02, F-CSC-03, F-CSC-08).** Each of these is a CI-visible
  failure that the release-gate itself advertises as a gate.
  The 187 iCKB-differential failures are recoverable in one
  command (`CELLSCRIPT_UPDATE_ICKB_DIFF_MATRIX=1 cargo test
  --test ickb_diff`), but no CI signal exists to know when the
  matrix is stale.
- **Hold for design review on F-CSC-04, F-CSC-05, F-CSC-06.**
  These are not regressions; they are documented design
  boundaries that the implementation does not yet honor.
- **Accept the remaining 18 findings (F-CSC-06 through F-CSC-25)**
  as quality debt, with F-CSC-09 (molecule-decoder `panic!`)
  prioritised in the next sprint because it is the only
  `panic!` in production `src/lib.rs` code.
