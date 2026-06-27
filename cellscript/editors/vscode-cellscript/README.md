# CellScript for VS Code

CellScript for VS Code gives `.cell` contracts the editor support you expect from a serious language toolchain: diagnostics, completion, hover, navigation, formatting, snippets, and compiler-backed commands.

The extension talks to the CellScript compiler through `cellc --lsp`, so editor feedback comes from the same parser, type checker, and lowering pipeline used by the command line.

## How It Fits Into The Toolchain

The extension is a compiler-backed editor surface. Syntax highlighting, snippets, and VS Code integration live in this repository, but language semantics come from `cellc`. That split keeps the editor useful without creating a second, weaker implementation of CellScript.

There are two feedback loops:

- the LSP loop, where `cellc --lsp` reports diagnostics, hover, completion, symbols, formatting, and navigation while you edit;
- the command loop, where one-shot VS Code commands run compiler package, registry, builder, ABI, metadata, and production-report checks.

Use the extension to shorten local authoring. Keep release evidence in the compiler, package, builder, registry, and CKB acceptance gates.

## What You Get

### Everyday Editing

- Syntax highlighting for `.cell` files.
- Snippets for resources, actions, locks, lifecycle helpers, proof blocks, source-qualified parameters, and CKB-oriented patterns.
- Live diagnostics as files open, change, and save.
- Completion for keywords, types, fields, symbols, and locals.
- Hover, go-to-definition, references, workspace rename, document symbols, document highlights, folding, selection ranges, and formatting.

### Compiler-Backed Commands

The command palette includes:

- `CellScript: Compile Current File`
- `CellScript: Show Metadata`
- `CellScript: Show Constraints`
- `CellScript: Show Entry Witness ABI`
- `CellScript: Show Action Build Plan`
- `CellScript: Generate TypeScript Action Builder`
- `CellScript: Verify Package`
- `CellScript: Verify Registry`
- `CellScript: Verify Live Registry`
- `CellScript: Show Builder Assumptions`
- `CellScript: Show Transaction Template`
- `CellScript: Show Deploy Plan`
- `CellScript: Show Profile`
- `CellScript: Generate Audit Bundle`
- `CellScript: Show Production Report`

These commands spawn `cellc` for one-shot evidence and build tasks. They do not replace release gates, deployment checks, transaction signing, or CKB node acceptance.

## Requirements

Install `cellc` and make sure it is available on `PATH`:

```bash
cellc --version
```

If `cellc` is not on `PATH`, set:

```text
cellscript.compilerPath
```

to the full compiler binary path.

When developing inside the CellScript Rust workspace, the extension can also use:

```bash
cargo run -q -p cellscript --
```

That fallback only runs in trusted workspaces and can be disabled with:

```text
cellscript.useCargoRunFallback = false
```

## First Use

1. Open a folder containing `.cell` files.
2. Confirm the CellScript status bar item appears.
3. Open a `.cell` file and check that diagnostics and highlighting are active.
4. Run `CellScript: Show Metadata` or `CellScript: Compile Current File` from the command palette.

For package-based projects, run package commands from a file under the nearest `Cell.toml`.

## Useful Settings

| Setting | Default | What it controls |
| --- | --- | --- |
| `cellscript.compilerPath` | `cellc` | Compiler binary used for the LSP server and CLI-backed commands. |
| `cellscript.useCargoRunFallback` | `true` | Whether trusted CellScript workspaces may fall back to `cargo run -q -p cellscript --`. |
| `cellscript.commandTimeoutMs` | `15000` | Timeout for compiler-backed commands. |
| `cellscript.maxOutputBytes` | `4194304` | Captured output limit. |
| `cellscript.target` | `riscv64-asm` | Default compiler target for compile, metadata, and constraints commands. |
| `cellscript.builderOutputDir` | `target/cellscript-builder/typescript` | Output directory for generated TypeScript action builders. |
| `cellscript.ckbRpcUrl` | empty | Optional CKB RPC URL for live registry verification. |
| `cellscript.deploymentNetwork` | empty | Optional deployment network filter for live registry checks and builder identity binding. |
| `cellscript.registryRequirePublisherSignature` | `false` | Adds the publisher-signature metadata gate to registry verification. |
| `cellscript.registryRequireAuditReport` | `false` | Adds the audit-report metadata gate to registry verification. |

## Authoring Model

This README documents the current 0.20 authoring surface. The extension follows the current CellScript action surface:

```cellscript
action fill_offer(input: Offer) -> output: Offer {
    transition input.state: Live -> output.state: Filled

    verification
        require output.price == input.price
        require output.seller == input.seller
}
```

Use `verification` sections for action and lock proof logic. At action and lock boundaries, source qualifiers are written before the parameter name:

```cellscript
action grant(read config: Config, tokens: Token) -> grant: Grant {
    verification
        create grant = Grant { admin: config.admin }
}

lock owner_only(protected cell: Wallet, witness owner: Address) -> bool {
    verification
        require owner == cell.owner
}
```

`create output = T { ... }` constrains a named proposed output Cell. It is not runtime allocation.

## Local Development

Install dependencies and run the extension checks:

```bash
npm ci
npm run validate
```

`npm run validate` builds the bundled extension and checks the manifest, grammar, snippets, language configuration, commands, settings, and runtime wiring.

## Packaging

```bash
npm run package
npm run publish:dry-run
```

`npm run publish:dry-run` writes a disposable VSIX to:

```text
/tmp/cellscript-vscode-dry-run.vsix
```

It does not contact the Visual Studio Marketplace.

## Evidence Boundary

This extension displays compiler evidence. It does not:

- create audit signatures;
- publish packages;
- deploy code Cells;
- sign transactions;
- submit transactions;
- replace `cargo test`, `cargo clippy`, `cellc check --production`, or chain acceptance scripts.

Use `CellScript: Show Production Report` during release review, then keep final acceptance in the compiler, package, and CKB deployment gates.

For a fuller tutorial, see:

```text
docs/Tutorial-07-LSP-and-Tooling.md
```
