# Agent Workflow

## Rust Skills

Use the installed `actionbook/rust-skills` skill set for Rust work in this
project. Start Rust questions, code changes, reviews, and compiler-error work
with `rust-router`, then follow the routed skill such as `m01-ownership`,
`m06-error-handling`, `m07-concurrency`, `unsafe-checker`, or
`coding-guidelines` as applicable.

Treat the CellScript-specific navigation, `CODING_STYLE.md`, and validation
requirements in this file as project overrides when they conflict with generic
Rust Skills guidance. Do not replace this file with upstream Rust Skills
`AGENTS.md`; merge only the parts that fit this repository.

## Code Exploration

For unfamiliar Rust files, inspect structure before reading full source.

Recommended workflow:

1. Use `ast-outline digest src` to build a repo-level structural map. Treat it
   as a local cache / retrieval index, not prompt material to paste into every
   session.
2. Use `ast-outline <file>` to inspect one file's declarations, signatures,
   and line ranges without method bodies.
3. Use `ast-outline show <file> <symbol>` only after identifying the target
   function, method, type, impl, or section.
4. Fall back to direct file reads only when the outline and selected symbol
   body do not provide enough context.

Bad:

```bash
# Pasting the whole digest into every coding session.
ast-outline digest src
```

Good:

```bash
# Keep the digest cached, then inspect only the relevant path and symbol.
ast-outline src/parser/mod.rs
ast-outline show src/parser/mod.rs parse_module
```

Symbol lookup caveat:

For Rust methods, do not assume `Type.method` will resolve. Prefer this order:

1. Run `ast-outline <file>`.
2. Copy the symbol name exactly as shown in the outline.
3. For simple functions, the bare function name often works:

   ```bash
   ast-outline show src/parser/mod.rs parse_module
   ```

4. For methods or ambiguous names, use the full outline symbol:

   ```bash
   ast-outline show src/parser/mod.rs "impl_Parser<'a>.parse_module"
   ```

If `ast-outline` is unavailable locally, install it with:

```bash
cargo install ast-outline
```

Use `rg` for text search. Use `ast-grep` for structural search when matching
syntax shape matters more than text. Do not treat `ast-outline` as a semantic
dependency graph, type resolver, macro expander, or compiler substitute.

## CellScript-Specific Navigation

`ast-outline` applies to the Rust implementation, not to `.cell` DSL source.
For `.cell` files, inspect declarations and behavior with the compiler, tests,
and targeted source reads.

Prefer these entry points:

- Parser and AST: `src/parser/mod.rs`, `src/ast/mod.rs`
- Type and name handling: `src/types/mod.rs`, `src/resolve/mod.rs`
- Lowering, metadata, and compile pipeline: `src/lib.rs`, `src/ir/mod.rs`
- Code generation and target behavior: `src/codegen/mod.rs`
- CLI and user-facing commands: `src/main.rs`, `src/cli/commands.rs`
- LSP/editor behavior: `src/lsp/mod.rs`, `src/lsp/server.rs`
- Formatting: `src/fmt/mod.rs`

Before changing a compiler feature, confirm that parser behavior, type
checking, lowering, metadata, formatter behavior, docs, and tests still agree
on the same feature boundary.

## Coding Style

Follow the tracked project rules in `CODING_STYLE.md`. For backend work, treat
emitted assembly as a compiler contract: any new mnemonic or pseudo-op emitted
by codegen, generated stdlib assembly, or collection helpers must be supported
by the internal assembler and covered by regression tests in the same change.

### Backend Refactor: Behaviour-Preserving Emitter Extraction

When extracting `&mut self` emitter methods from `codegen/mod.rs` into a
sub-module (e.g. `assembler.rs`, `runtime.rs`, `abi.rs`):

1. **Use exact source movement.** Extract the original code verbatim with
   `git show` or equivalent. Never manually reconstruct emitter logic from
   memory. A single wrong register, label, or branch in a reconstructed
   method will silently change generated assembly and break on-chain contracts.

2. **Verify generated assembly is unchanged.** Run the full test suite after
   each extraction. The codegen tests include end-to-end assembly assertions
   that catch transcription errors.

3. **Prefer `pub(crate)` temporarily.** Cross-module `impl` blocks on the same
   struct need method visibility to match call sites. Use `pub(crate)` for
   methods called from other modules within the crate. Fields of types shared
   across module boundaries (e.g. `MachineLayoutPlan.metrics`) also need
   `pub(crate)`.

4. **Delete from back to front.** When removing code by line number with `sed`,
   delete later ranges first to keep earlier line numbers stable.

5. **Brace-count after every deletion.** Use `python3 -c` to verify brace
   balance before attempting compilation. Off-by-one `sed` ranges can leave
   orphaned lines or eat closing braces.

## Validation

Use focused checks while developing, then broaden validation before completion.

```bash
cargo fmt --all
cargo check --locked -p cellscript --all-targets
cargo test --locked -p cellscript
git diff --check
```

For broad Rust changes, also run:

```bash
cargo clippy --locked -p cellscript --all-targets -- -D warnings
```

Keep CKB-facing claims precise. Compile-only evidence is weaker than
builder-backed acceptance, valid and invalid lock-spend evidence, cycle
measurement, transaction size, occupied capacity, and under-capacity checks.
