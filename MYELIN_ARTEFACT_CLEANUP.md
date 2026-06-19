# Myelin Artefact Cleanup

> This document records what non-Rust artefact files were deleted in
> the previous preparation pass, what remains in the working tree, and
> the justification for each remaining non-Rust artefact.
>
> It does not propose new artefacts. The hardening pass deliberately
> keeps the existing surface lean.

## 1. Files deleted in the previous preparation pass

The full set of paths that were already deleted (and now show as
`deleted:` in `git status`) is listed in section 2 of
`MYELIN_STALE_SURFACE_AUDIT.md`. The non-Rust portion of that
deletion list is summarized here for traceability:

```text
Markdown (history / roadmap / archive / release notes):
  cellscript/BRANCHES.md
  cellscript/CHANGELOG.md
  cellscript/CODING_STYLE.local.md
  cellscript/README_REVIEW.md
  cellscript/docs/0.17/CELLSCRIPT_0_17_ROADMAP.md
  cellscript/docs/0.17/ickb_final_report.md
  cellscript/docs/0.17/ickb_production_equivalence_gate.md
  cellscript/docs/0.17/review_findings_closure.md
  cellscript/docs/CELLSCRIPT_0_18_ROADMAP.md
  cellscript/docs/CELLSCRIPT_0_19_ROADMAP.md
  cellscript/docs/CELLSCRIPT_0_20_ROADMAP.md
  cellscript/docs/archive/0.13/CELLSCRIPT_0_13_1_PLAN.md
  cellscript/docs/archive/0.13/CELLSCRIPT_SIGNATURE_DIRECTION_EXECUTION_PLAN.md
  cellscript/docs/archive/0.15/CELLSCRIPT_0_15_ROADMAP_SUMMARY.md
  cellscript/docs/releases/CELLSCRIPT_0_13_2_ACCEPTANCE_COMMUNITY_POST.md
  cellscript/docs/releases/CELLSCRIPT_0_13_2_RELEASE_NOTES.md
  cellscript/docs/releases/CELLSCRIPT_0_13_RELEASE_SCOPE.md
  cellscript/docs/releases/CELLSCRIPT_0_14_RELEASE_NOTES.md
  cellscript/docs/releases/CELLSCRIPT_0_15_RELEASE_NOTES.md
  cellscript/docs/releases/CELLSCRIPT_0_16_1_RELEASE_NOTES.md
  cellscript/docs/releases/CELLSCRIPT_0_16_RELEASE_NOTES.md
  cellscript/docs/releases/CELLSCRIPT_0_19_CLOSURE_NOTES.md
  cellscript/roadmap/CELLSCRIPT_0_13_ROADMAP.md
  cellscript/roadmap/CELLSCRIPT_0_13_TODOLIST.md
  cellscript/roadmap/CELLSCRIPT_0_14_ROADMAP.md
  cellscript/roadmap/CELLSCRIPT_0_15_ROADMAP.md
  cellscript/roadmap/CELLSCRIPT_0_16_ROADMAP.md
  cellscript/roadmap/CELLSCRIPT_ROADMAP.md
  cellscript/roadmap/CELLSCRIPT_ROADMAP_OVERVIEW.md

JavaScript / VS Code extension:
  cellscript/editors/vscode-cellscript/.gitignore
  cellscript/editors/vscode-cellscript/.vscodeignore
  cellscript/editors/vscode-cellscript/CHANGELOG.md
  cellscript/editors/vscode-cellscript/LICENSE.md
  cellscript/editors/vscode-cellscript/README.md
  cellscript/editors/vscode-cellscript/extension.js
  cellscript/editors/vscode-cellscript/language-configuration.json
  cellscript/editors/vscode-cellscript/package-lock.json
  cellscript/editors/vscode-cellscript/package.json
  cellscript/editors/vscode-cellscript/scripts/validate.mjs
  cellscript/editors/vscode-cellscript/snippets/cellscript.json
  cellscript/editors/vscode-cellscript/syntaxes/cellscript.tmLanguage.json

Website (Astro / TS / CSS / images):
  cellscript/website/.gitignore
  cellscript/website/astro.config.mjs
  cellscript/website/design/cellscript-website-concept.png
  cellscript/website/package-lock.json
  cellscript/website/package.json
  cellscript/website/public/cellscript-logo.png
  cellscript/website/src/data/site.ts
  cellscript/website/src/i18n/translations.ts
  cellscript/website/src/pages/index.astro
  cellscript/website/src/styles/global.css
  cellscript/website/tsconfig.json

Shell / Python (NovaSeal / proposal / tooling release wrappers):
  cellscript/scripts/cellscript_0_14_scope_audit.sh
  cellscript/scripts/cellscript_gate.sh
  cellscript/scripts/novaseal_agreement_devnet_stateful_live.py
  cellscript/scripts/novaseal_bip340_tcb_review.py
  cellscript/scripts/novaseal_btc_anchor_contract.py
  cellscript/scripts/novaseal_btc_spv_evidence_adapter.py
  cellscript/scripts/novaseal_devnet_stateful_acceptance.sh
  cellscript/scripts/novaseal_devnet_stateful_live.py
  cellscript/scripts/novaseal_external_attestation_adapter.py
  cellscript/scripts/novaseal_external_evidence_handoff_bundle.py
  cellscript/scripts/novaseal_fiber_node_experiments.py
  cellscript/scripts/novaseal_planned_profiles_devnet_stateful_live.py
  cellscript/scripts/novaseal_profile_operator_fixtures.py
  cellscript/scripts/novaseal_service_builder_fixtures.py
  cellscript/scripts/novaseal_wallet_signing_vectors.py
  cellscript/scripts/validate_cellscript_tooling_release.py

Nix:
  shell.nix
```

These deletions are correct. They are not reintroduced by this
hardening pass.

## 2. Files kept, with a written reason for each non-Rust artefact

Every non-Rust artefact that survives in the working tree falls into
exactly one of the following categories. The list below is exhaustive.

### 2.1 Build, formatting, and tooling configuration

```text
.cargo/config.toml                         cargo env: llvm paths, INSTA_UPDATE
.rustfmt.toml                              formatting policy for the main workspace
clippy.toml                                clippy lints for the main workspace
.gitattributes                             * text=auto eol=lf
.gitignore                                 build / target / IDE exclusion rules
LICENSE                                    MIT licence
Cargo.toml                                 main workspace
Cargo.lock                                 main workspace lockfile
cellscript/.rustfmt.toml                   formatting policy for cellscript
cellscript/Cargo.toml                      cellscript workspace
cellscript/Cargo.lock                      cellscript workspace lockfile
cellscript/crates/cellscript-ckb-adapter/Cargo.toml
                                           headless CKB adapter crate manifest
cellscript/examples/*/Cell.toml + Cargo.toml
                                           example CellScript package manifests
cellscript/tools/ckb-tx-measure/Cargo.toml + README.md
                                           release-evidence helper, links against
                                           a parent CKB checkout (see §2.5)
cli/Cargo.toml                             myelin-cli manifest
clippy.toml                                clippy lints
consensus/Cargo.toml                       myelin-consensus manifest
crypto/hashes/Cargo.toml                   myelin-hashes manifest
crypto/muhash/Cargo.toml                   myelin-muhash manifest
crypto/muhash/fuzz/Cargo.toml              fuzz harness manifest
crypto/muhash/fuzz/rust-toolchain.toml     pinned toolchain for the fuzz harness
crypto/muhash/fuzz/fuzz.sh                 fuzz runner script
exec/Cargo.toml                            myelin-exec manifest
math/Cargo.toml                            myelin-math manifest
mempool/Cargo.toml                         myelin-mempool manifest
state/Cargo.toml                           myelin-state manifest
utils/Cargo.toml                           myelin-utils manifest
```

The `vendor/` directory is also retained, but it is **not used** by
any active Cargo manifest. It contains two stale source-chain
dependencies that survived the cut:

```text
vendor/workflow-node/                      empty stub directory
vendor/workflow-perf-monitor/              stale perf-monitor fork, not referenced
```

These are kept for now because removing them is a separate change
that is outside the audit scope of this hardening pass. They are
explicitly tagged as stale in the kept-files table.

### 2.2 Acceptance and protocol gate scripts

```text
scripts/myelin_protocol_gate.sh            full Myelin protocol gate
scripts/myelin_teeworlds_acceptance.sh     Teeworlds acceptance gate
cellscript/scripts/cellscript_ckb_release_gate.sh
                                           cellscript release gate (quick/full)
cellscript/scripts/cellscript_ckb_adapter_acceptance.sh
                                           cellscript CKB adapter acceptance
cellscript/scripts/cellscript_ckb_ecosystem_reuse_gate.sh
                                           CKB ecosystem reuse gate
cellscript/scripts/cellscript_ckb_stateful_scenarios.sh
                                           stateful CKB scenarios
cellscript/scripts/cellscript_strict_backend_audit.sh
                                           strict backend audit runner
cellscript/scripts/cellscript_strict_backend_audit.py
                                           strict backend audit driver
cellscript/scripts/cellscript_syntax_combo_audit.sh
                                           syntax-combination audit runner
cellscript/scripts/cellscript_syntax_combo_audit.py
                                           syntax-combination audit driver
cellscript/scripts/cellscript_cellfabric_bridge_smoke.sh
                                           CellFabric bridge smoke check
cellscript/scripts/ckb_cellscript_acceptance.sh
                                           CKB acceptance driver
cellscript/scripts/validate_ckb_cellscript_production_evidence.py
                                           production evidence validator
cellscript/scripts/install.sh              cellscript build/install helper
exec/src/scripts/fixtures/build_fixtures.sh
                                           builds standard-script fixtures for tests
```

Each of these is invoked by a concrete test, gate, or fixture builder.
The Python scripts in particular are called by both the bash wrappers
and the validator. They are not redundant.

### 2.3 Top-level and crate-level docs

```text
README.md                                  public framing of the standalone Myelin
docs/ARCHITECTURE.md                       high-level architecture seed
docs/MYELIN_ARCHITECTURE.md                full architecture narrative
docs/TEEWORLDS_FIXTURE.md                  Teeworlds fixture path document
exec/README.md                             myelin-exec overview
exec/API_GUIDE.md                          myelin-exec serialization API guide
exec/IMPLEMENTATION_SUMMARY.md             myelin-exec serialization summary
exec/src/scripts/README.md                 standard-script fixtures index
exec/src/serialization/README.md           serialization framework overview
exec/src/vm/README_VM_STATUS.md            VM status
mempool/README.md                          myelin-mempool overview
state/README.md                            myelin-state overview
cellscript/AGENTS.md                       agent workflow for cellscript
cellscript/CODING_STYLE.md                 cellscript maintainer style guide
cellscript/README.md                       cellscript overview
cellscript/docs/README.md                  cellscript documentation map
cellscript/docs/CELLSCRIPT_*.md            active compiler reference docs
cellscript/docs/examples/*.md              active example walkthroughs
cellscript/docs/spec/CELLSCRIPT_OPERATIONAL_SEMANTICS.md
                                           v0.16 mechanically precise spec
cellscript/docs/tutorials/phase1-end-to-end.md
                                           active end-to-end tutorial
cellscript/docs/wiki/*.md                  active wiki tutorials
cellscript/examples/ckb-sdk-builder/README.md
                                           CKB SDK builder overview
cellscript/tests/benchmarks/ickb_specs/README.md
                                           iCKB benchmark specs
cellscript/tools/ckb-tx-measure/README.md  helper tool overview
```

All of these are read paths for active code or active gates. None
are release-note history or roadmap promises.

### 2.4 Test fixtures, manifests, and benchmark data

```text
cellscript/tests/backend_shape_baseline.json
                                           backend shape baseline
cellscript/tests/benchmarks/ickb_diff/claim_manifest.json
                                           iCKB claim manifest
cellscript/tests/benchmarks/ickb_diff/matrix.json
                                           iCKB differential matrix
cellscript/tests/benchmarks/ickb_diff/ckb_vm_fixtures/*.json
                                           CKB VM regression fixtures
cellscript/tests/benchmarks/ickb_negative/*.json
                                           iCKB negative test fixtures
cellscript/tests/benchmarks/ickb_positive/*.json
                                           iCKB positive test fixtures
cellscript/tests/benchmarks/ickb_specs/README.md
                                           iCKB benchmark specs
cellscript/tests/compat/ckb_standard/manifest.json
                                           CKB standard compat manifest
cellscript/tests/compat/ckb_standard/*.json
                                           CKB standard compat fixtures
cellscript/tests/syntax_combo/matrix.toml  syntax-combo audit matrix
exec/src/scripts/fixtures/*.elf            standard-script binaries
exec/src/scripts/fixtures/*.rs            standard-script sources
exec/src/scripts/fixtures/CODE_HASHES.blake3
                                           pinned code-hash commitments
```

Every JSON / TOML here is a deterministic regression fixture. The
ELF files are prebuilt standard scripts that the VM-probe and
script tests load. Removing any of them would either break
specific tests or force a re-pin of the runtime code-hash
commitments.

### 2.5 CKB-relative tooling and per-repo toolchains

```text
cellscript/tools/ckb-tx-measure/            links against a parent ckb/ checkout
                                           via path = "../../../ckb/util/..."
                                           kept because it is part of the
                                           ckb_cellscript_acceptance.sh
                                           production-evidence flow.
crypto/muhash/fuzz/rust-toolchain.toml     pinned nightly toolchain for the
                                           fuzz harness
```

`cellscript/tools/ckb-tx-measure` is the only non-Rust artefact
that depends on the parent `Spora/ckb/` checkout. The
`ckb_cellscript_acceptance.sh` and
`validate_ckb_cellscript_production_evidence.py` scripts
*expect* a parent CKB source tree to be present. In a fully
standalone Myelin tree this helper is unused; in the current
cellscript production-evidence path it is still called. We do not
delete it in this hardening pass because doing so would break
the production-evidence acceptance script.

### 2.6 Cargo-registry-mirrored vendored crates

```text
vendor/workflow-node/                      empty stub, not referenced anywhere
vendor/workflow-perf-monitor/              stale fork of perf-monitor, not
                                           referenced anywhere
```

Both are kept for now. They are unused by the active Cargo
manifests (`rg "workflow-(node|perf-monitor)" Myelin/` returns no
matches except their own internal files). They were inherited
from the source-chain prototype and are candidates for removal in
a follow-up sweep, but removing them is outside the audit scope
of this hardening pass.

## 3. Broken references fixed in this hardening pass

The previous preparation pass already fixed the broken references
in:

```text
cellscript/CODING_STYLE.md            (cellscript_gate.sh -> ckb_release_gate)
cellscript/README.md                 (VS Code extension references removed)
cellscript/docs/CELLSCRIPT_GATE_POLICY.md
                                     (gate modes rewritten to use the
                                      current release gate)
cellscript/docs/CELLSCRIPT_GRAMMAR_GOVERNANCE_RFC.md
                                     (VS Code validate/dry-run removed)
cellscript/docs/CELLSCRIPT_SURFACE_ELEGANCE_RFC.md
                                     (LSP/VS Code -> LSP)
cellscript/docs/CELLSCRIPT_SYNTAX_COMBO_AUDIT_METHODOLOGY.md
                                     (gate references updated)
cellscript/docs/wiki/*.md            (gate references and stale release
                                      notes removed)
cellscript/scripts/cellscript_ckb_release_gate.sh
                                     (deleted-file references removed)
cellscript/scripts/ckb_cellscript_acceptance.sh
                                     (deleted cellscript_gate.sh reference
                                      removed)
cellscript/scripts/validate_ckb_cellscript_production_evidence.py
                                     (deleted cellscript_gate.sh reference
                                      removed)
exec/src/serialization/molecule_compat.rs
                                     (PoW doc comments rewritten as
                                      "Compact CKB header target field"
                                      and "CKB header nonce field")
```

This hardening pass did not find any further broken references. The
`vendor/` directories and `tools/ckb-tx-measure` are deliberately
flagged above as candidates for a future follow-up.

## 4. Final non-Rust inventory

After the previous preparation pass plus this hardening pass, the
non-Rust artefacts remaining in the Myelin tree are:

```text
44 .md        active docs and active wiki tutorials
 1 .gitattributes
 1 .gitignore
 1 LICENSE
 2 .rustfmt.toml + clippy.toml + .cargo/config.toml
 9 Cargo.toml  (root + 8 crates)
 2 Cargo.lock  (root + cellscript)
27 .json       test fixtures and compatibility/benchmark manifests
 1 .toml       (syntax-combo matrix)
13 .sh         (8 cellscript + 3 myelin-protocol + 2 helpers)
 3 .py         (cellscript audit + production-evidence driver)
 1 .nix        (none — shell.nix was deleted in the previous pass)
 1 .gitignore  (cellscript)
```

No additional non-Rust artefact was deleted in this hardening pass.
The cleanup budget is fully spent on the previous preparation pass.
