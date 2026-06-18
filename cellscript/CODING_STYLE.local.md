# CellScript Local Coding Style

This file is a local working standard. It is intentionally not tracked by git.

## 1. Release Boundary Discipline

- Do not describe a feature as implemented unless parser, type checking,
  lowering, metadata, tests, examples, and docs agree on the same boundary.
- Use "reserved", "deferred", or "fail-closed" when syntax exists but the
  semantics are intentionally unavailable.
- Never imply that `Address`, parameter names, or witness data create
  authorization. Only verified bindings create authorization semantics.
- Keep beta release notes feature-focused. Put limitations under scope
  boundaries, not as alarm-style evidence warnings.

## 2. CKB Semantics

- Explain features with CKB terms first: input Cell, output Cell, lock script,
  type script, script args, WitnessArgs, lock group, cell deps, since, capacity,
  transaction validation.
- `protected T` means a typed view of one selected input Cell guarded by the
  current lock invocation. It is not an output, not a global scan, and not all
  matching Cells in the transaction.
- `witness T` means typed transaction witness data. It is not signer authority.
- `require` is lock-boundary syntax only. Actions use `assert_invariant`.
- `lock_args` remains fail-closed until explicit typed script-args binding is
  implemented.
- Hidden sighash defaults are forbidden. Future signature verification must
  expose digest mode, script group scope, witness layout, and replay assumptions.

## 3. Rust Code

- Keep changes narrow and local to the feature being modified.
- Prefer existing compiler phases and data structures over adding parallel
  paths.
- Parser support alone is not enough. A new source form needs type checking,
  lowering, metadata visibility, formatter behavior, and tests.
- Do not add stringly typed behavior when an enum or structured field already
  exists.
- Error messages should name the boundary and the next valid action.
- Keep public metadata stable. If metadata changes, add or update tests that
  lock the visible shape.
- Run `cargo fmt --all` before committing Rust changes.
- Run `cargo clippy --locked -p cellscript --all-targets -- -D warnings` for
  broad Rust changes.

## 4. CellScript Source Style

- Use namespace-style modules: `module cellscript::name`.
- Prefer DSL-native capability declarations:
  `resource X has store, transfer, destroy`.
- Use field shorthand only when it is exactly equivalent to `field: field`.
- Empty `[]` is only typed `Vec::new()` sugar when the expected `Vec<T>` type is
  known. Do not use untyped `let x = []`.
- In local variables with empty vectors, prefer an explicit type:
  `let mut data: Vec<u8> = []`.
- Comments should explain Cell lifecycle, CKB boundary, witness scope, or
  builder obligations. Do not comment obvious arithmetic or assignments.
- Do not use names such as `signer`, `authorized`, or `owner` for witness-only
  authority unless the value is cryptographically verified. Prefer
  `claimed_owner`, `provided_admin`, or `claimed_signer`.

## 5. Example Organization

- Top-level `examples/*.cell` is the only checked-in bundled business source.
- `examples/language` is for compiler/tooling language coverage.
- Do not reintroduce `examples/business` or `examples/acceptance` mirrors.
  Acceptance-only profile/effect/scheduler metadata belongs in runner
  configuration or generated files under `target/`.
- Bundled examples should optimize for readability without hiding Cell
  movement or authorization limits.
- Acceptance scripts should compile the canonical top-level examples directly
  and keep report evidence outside checked-in source copies.

## 6. CKB Acceptance Scripts

- Invalid lock-spend evidence must match stable predicate failure paths, not
  broad fragments such as generic `script` or `Invalid`.
- Compile-only acceptance must be visibly weaker than full production
  acceptance.
- Full production acceptance requires builder-backed actions, valid lock spends,
  invalid lock spends, measured cycles, serialized transaction size, occupied
  capacity, and no under-capacity outputs.
- If a validator flag says "compile-only", it must not require on-chain
  production-ready fields.
- Reports should distinguish strict compile coverage from builder-backed
  behavior coverage.

## 7. Documentation And Wiki

- Wiki pages should teach gently and progressively. Avoid making them read like
  internal engineering status dumps.
- Use rendered GitHub Wiki links for wiki navigation, not raw `.md` links.
- Keep wiki pages version-neutral unless a page is explicitly release-specific.
- Put a timestamp on the Wiki Home page when useful.
- Use CKB-native explanations for CKB concepts. Avoid generic smart-contract
  vocabulary when a CKB term is more precise.
- Explicitly state boundaries where users could otherwise infer unsafe
  authorization semantics.

## 8. Roadmaps And Release Notes

- Roadmap overview and release-specific roadmaps must agree.
- A completed item must have implementation, tests, and docs.
- A deferred item must explain why it is deferred and what would make it safe.
- Release notes should separate:
  - highlights;
  - scope boundaries;
  - verification commands;
  - links to detailed docs.
- Do not overclaim production readiness from compile-only evidence.

## 9. Tests

- For syntax changes, add parser, formatter, type-checker, lowering, and
  metadata tests where applicable.
- For CKB-facing changes, add negative tests for unsafe or ambiguous forms.
- For examples, keep tests that prove top-level examples are the only checked-in
  bundled business source and stay free of profile hint noise.
- Prefer targeted tests during development, then run broader gates before
  commit.
- Always run `git diff --check`.

## 10. Git And Publishing

- Keep local-only notes in `*.local.md` files and exclude them with
  `.git/info/exclude`.
- Do not commit local policy notes unless explicitly requested.
- Keep one logical change per commit.
- Tags should point at the intended release commit. Beta tags should use a
  prerelease spelling such as `v0.13.0-beta`.
- GitHub Release edits do not require retagging unless the target commit is
  wrong.

## 11. Review Checklist

Before calling a scope complete:

- Does the implementation match the docs exactly?
- Does the type checker reject misleading or unsafe surfaces?
- Does lowering use the existing trusted path instead of a parallel shortcut?
- Do examples teach the intended canonical style?
- Do acceptance scripts prove the intended predicate, not any rejection?
- Are release notes honest about beta, deferred, and fail-closed boundaries?
