# CellScript Gate Policy

CellScript uses one release gate entry point:

```bash
./scripts/cellscript_ckb_release_gate.sh <quick|full>
```

The lower-level audit scripts remain available for focused debugging, but they
are implementation details of the gate policy. Prefer the release gate when
making merge-readiness or production claims.

## Gate Modes

| Mode | When to run | Evidence boundary |
|---|---|---|
| `quick` | Pull requests, pushes, and routine merge readiness | Rust formatting/check/test/clippy, syntax-combination quick audit, builder tooling, and compile-only CKB production acceptance |
| `full` | Nightly/stable release candidates and any production CKB claim | `quick` plus deep syntax-combination audit, builder-backed CKB production acceptance, and stateful scenario/action coverage |

For local iteration, run the focused component that matches the change and then
run `quick` before making a merge-readiness claim.

## Command Cheatsheet

```bash
# Local fast path
cargo check --locked -p cellscript --all-targets

# Default CI/PR gate
./scripts/cellscript_ckb_release_gate.sh quick

# Strict compiler-contract gate for backend work
./scripts/cellscript_strict_backend_audit.sh full

# Release-facing CKB production gate
./scripts/cellscript_ckb_release_gate.sh full
```

## Lower-Level Components

Use these only when you need a focused failure:

```bash
./scripts/cellscript_syntax_combo_audit.sh quick
./scripts/cellscript_syntax_combo_audit.sh ci
./scripts/cellscript_strict_backend_audit.sh quick
./scripts/cellscript_strict_backend_audit.sh ci
./scripts/cellscript_strict_backend_audit.sh full
./scripts/ckb_cellscript_acceptance.sh --production --stateful-scenarios
```

Passing one component does not imply the corresponding higher-level gate passed.
For example, CKB acceptance proves selected transaction behavior, while the
syntax-combination and strict backend audits prove compiler-layer edge cases and
structural invariants.

## Artifact Reports

The gates write machine-readable reports under `target/`:

- `target/syntax-combo-audit/`
- `target/cellscript-strict-backend-audit/`
- `target/ckb-cellscript-acceptance/`
- `target/cellscript-backend-shape/`
- `target/cellscript-schema-manifest/`

For release evidence, keep the JSON report paths in the release checklist rather
than copying long logs into review threads.
