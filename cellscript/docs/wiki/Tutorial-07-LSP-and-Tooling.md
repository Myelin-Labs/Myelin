You can write CellScript with any text editor and the `cellc` CLI. The LSP
server makes that loop shorter. Parse errors, type errors, flow mistakes,
symbols, hovers, formatting, and compiler-backed reports can show up while you
work instead of after a long command sequence.

The useful thing to remember is that editor feedback is not a separate language
implementation. It is tied to the same parser, type checker, state-transition checks,
and lowering metadata used by `cellc`.

## What You Will Learn

- what the LSP server supports;
- where editor tooling helps;
- where release gates still need CLI and CKB evidence.

## LSP Capabilities

The LSP implementation supports the editor features you expect while writing a
contract:

- diagnostics for parse, type, flow, and lowering errors;
- hover information for actions, receipts, fields, local variables, flow
  states, and lowering metadata;
- keyword, type, symbol, field, local, enum variant, and qualified flow
  state completions such as `Ticket::Active`;
- go-to-definition;
- find-references;
- workspace rename with identifier-boundary checks;
- document symbols;
- document highlight;
- signature help;
- folding ranges;
- selection ranges;
- formatting;
- code actions for lowering diagnostics;
- incremental document sync using LSP UTF-16 positions.

Run the server over stdio:

```bash
cellc --lsp
```

In practice you configure your editor to start `cellc --lsp` directly.

## Generated Builder Checks

Generated builder packages are local artifacts. After using
`CellScript: Generate TypeScript Action Builder`, run the generated package's
own checks before treating it as usable transaction-building evidence. This is
the generated package's `npm test` boundary:

```bash
npm --prefix target/cellscript-builder/typescript install --ignore-scripts
npm --prefix target/cellscript-builder/typescript test
```

The generated tests prove the TypeScript package compiles, plans actions,
delegates live-cell resolution/build/dry-run/submit to the runtime adapter, and
fails closed on mismatched lockfile or deployment identity. They do not prove
wallet signing, CKB node acceptance, or committed stateful flows.

## A Comfortable Local Loop

While editing, let the LSP catch small mistakes quickly. Before committing, run
the CLI checks explicitly:

```bash
cellc fmt --check
cellc check --all-targets --json
cellc metadata . --target riscv64-elf --target-profile ckb -o /tmp/metadata.json
cellc build --target riscv64-elf --target-profile ckb --json
cellc verify-artifact build/main.elf --verify-sources --expect-target-profile ckb
cellc package verify --json
cellc registry verify --json
```

For trust metadata review, add the explicit presence gate:

```bash
cellc registry verify --require-publisher-signature --require-audit-report --json
```

Run these from a package directory that contains `Cell.toml`. The `.` argument
refers to the current package; for a single file, pass the file path instead.

For CKB admission, keep the profile visible:

```bash
cellc check --target-profile ckb --json
cellc build --target riscv64-elf --target-profile ckb --json
cellc verify-artifact build/main.elf --expect-target-profile ckb
cellc registry verify --live --rpc-url "$CELLSCRIPT_CKB_RPC_URL" --json
cellc action build . --action mint --target-profile ckb --fabric-intent --json
cellc gen-builder . --target typescript --output target/cellscript-builder/typescript --target-profile ckb --json
npm --prefix target/cellscript-builder/typescript install --ignore-scripts
npm --prefix target/cellscript-builder/typescript test
```

This loop gives fast feedback first, then more formal evidence as the contract
gets closer to review.

## Formatting

Apply formatting:

```bash
cellc fmt
```

Check formatting without changing files:

```bash
cellc fmt --check
```

The formatter is especially useful after applying field shorthand or cleaning up
example code. It keeps the source style consistent without turning style into a
manual review topic.

## Generated Documentation

Generate package docs:

```bash
cellc doc
```

With JSON summary:

```bash
cellc doc --json
```

Documentation output includes the public contract surface and metadata-derived
lowering information.

## Local Package Workflow

The package manager supports:

- `cellc init`
- `cellc build`
- `cellc check`
- `cellc fmt`
- `cellc doc`
- `cellc add --path`
- `cellc remove`
- `cellc info`
- `cellc package verify`
- `cellc registry verify`
- lockfile consistency checks for local dependencies

Use the top-level `cellc path/to/file.cell` form for one-off file compilation.
Use `cellc build` for package builds.

Local `cellc install --path` and `cellc update` are supported as lockfile helpers
for local path dependency workflows. Treat registry package installation,
registry publishing, `login`, and `run` flows as experimental unless your
current build explicitly reports them as completed and supported.

## Next

With the tooling loop in place, continue with
[Bundled Example Contracts](https://github.com/a19q3/CellScript/wiki/Tutorial-08-Bundled-Example-Contracts).
