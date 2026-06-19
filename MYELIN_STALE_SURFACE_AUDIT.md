# Myelin Stale Surface Audit

> Scope: every file inside `/Users/arthur/RustroverProjects/Myelin` that
> is committed or staged in the working tree. The parent `Spora` folder
> is out of scope and is not audited.
>
> This audit documents what stale or inherited surface remains in the
> standalone Myelin tree after the NovaSeal / proposal-era removal pass,
> why each remaining reference is (or is not) still justified, and which
> of them were further deleted in this hardening sweep.

## 1. Audit scope and method

The audit walks:

```text
README.md
docs/
scripts/
cli/        consensus/        exec/        state/       mempool/
crypto/     math/             utils/       cellscript/
```

Search vocabulary:

```text
Spora / spora
NovaSeal / novaseal
proposal / certifier / certify
website / roadmap / archive / release note
CKB fork / PoW / miner / mining / full node / L1 sync
```

Search method: every directory above is searched for each term. Any
match is classified as one of:

```text
[GONE]      file deleted in this sweep
[JUSTIFIED] reference kept, with a written reason
[CONFLICT]  reference is confusing but kept because the
            surrounding code path is correct (CKB comparison doc,
            CKB RawHeader Molecule field, or a domain-level
            governance word like "Proposal" in a CellScript multisig
            example, etc.)
```

## 2. Stale surface already removed in the previous pass

The previous preparation pass deleted or rewired the following surfaces.
The deletions are present in the working tree and recorded in `git status`.
They are not part of the remaining surface and are listed here only to
fix the audit trail.

```text
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

cellscript/roadmap/CELLSCRIPT_0_13_ROADMAP.md
cellscript/roadmap/CELLSCRIPT_0_13_TODOLIST.md
cellscript/roadmap/CELLSCRIPT_0_14_ROADMAP.md
cellscript/roadmap/CELLSCRIPT_0_15_ROADMAP.md
cellscript/roadmap/CELLSCRIPT_0_16_ROADMAP.md
cellscript/roadmap/CELLSCRIPT_ROADMAP.md
cellscript/roadmap/CELLSCRIPT_ROADMAP_OVERVIEW.md

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

cellscript/src/cli/novaseal_certification.rs

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

shell.nix
```

The above deletions are required to make the standalone Myelin tree
honest. The current section of this audit documents **only** what
remains in the working tree.

## 3. Remaining surface — case-by-case justification

### 3.1 `Spora` / `spora`

```text
[JUSTIFIED] no occurrences
```

`rg -i 'spora' Myelin/` returns zero matches. The parent folder is not
referenced from any active file.

### 3.2 `NovaSeal` / `novaseal`

```text
[JUSTIFIED] no occurrences
```

`rg -i 'NovaSeal' Myelin/` and `rg -i 'novaseal' Myelin/` both return
zero matches in committed and working-tree code, tests, fixtures, docs,
scripts, and Cargo metadata. The `novaseal_certification.rs` module,
the `cellc certify` CLI subcommand, and the related harness crates
were removed.

The only adjacent `mainnet-certifi*` style references that survive
(`cellscript/docs/CELLSCRIPT_PACKAGE_PROVENANCE_AND_DEPLOYMENT_IDENTITY.md`,
`cellscript/docs/CELLSCRIPT_CKB_ADAPTER.md`,
`cellscript/crates/cellscript-ckb-adapter/src/lib.rs`) are
*deployment-evidence* language, not the certifier module — they are
warnings that no deployment should be advertised as `mainnet-certified`
without external audit. They do not refer to the removed module.

### 3.3 `proposal` / `certifier` / `certify`

#### 3.3.1 `Proposal` as a CellScript domain type in `cellscript/examples/multisig.cell`

```text
[JUSTIFIED] domain type, kept
```

`cellscript/examples/multisig.cell`, its `cellscript/examples/multisig/src/main.cell`
mirror, the test in `cellscript/tests/examples.rs`, the
`cookbook/tutorial/wikipedia` references, and one source string in
`cellscript/src/lib.rs` all use a `Proposal` type. This is a
**multisig-governance domain type** (threshold-wallet proposal
records: `proposal_id`, `signatures`, `expires_at`) and is unrelated
to the removed proposal-era CellScript roadmaps. The CellScript
multisig example is the canonical demonstration of CellScript
ownership and lock-boundary predicates, and removing it would erase
the most readable witness-binding example in the tree.

#### 3.3.2 `proposals_hash` field on the CKB RawHeader struct

```text
[JUSTIFIED] CKB Molecule layout field, kept
```

`exec/src/serialization/molecule_compat.rs`, `exec/src/vm/verifier.rs`,
`exec/src/vm/syscalls/load_header.rs`, `exec/src/vm/syscalls/mod.rs`,
the syscall edge-case and header-timestamp tests, the VM-ABI
integration test, the serialization bench, the serialization usage
example, and `exec/API_GUIDE.md` all reference `proposals_hash` and
`HeaderField::ProposalsHash`. This is the CKB `RawHeader.proposals_hash`
field that is part of the CKB Molecule wire layout; it is a required
byte position when the projection layer encodes a CKB-shaped header.
The name is fixed by CKB; renaming it would break the Molecule
compatibility layer's responsibility to be wire-faithful to CKB.

The CKB RawHeader Molecule layout field is *not* a Myelin runtime
concept. It is a wire-format constant that the Myelin CKB-projection
layer must reproduce. The previous preparation pass already changed
the doc comments on these fields from "Proof-of-work" / "PoW nonce" to
"Compact CKB header target field" / "CKB header nonce field", so the
narrative around this field is no longer misleading.

#### 3.3.3 Cellscript cookbook / wiki prose

```text
[JUSTIFIED] tutorial prose, kept
```

`cellscript/docs/wiki/Cookbook-Recipes.md`,
`cellscript/docs/wiki/Tutorial-04-Packages-and-CLI-Workflow.md`,
`cellscript/docs/wiki/Tutorial-08-Bundled-Example-Contracts.md` use
the word "proposal" when describing CellScript multisig proposal
flows. This is the same domain word as 3.3.1. It is the right word
for the example the page is teaching.

#### 3.3.4 `cellscript/docs/README.md` — one explicit acknowledgment

```text
[JUSTIFIED] explicit deletion note, kept
```

The file states: *"Historical release notes, archived plans,
standalone website material, and proposal-era roadmaps have been
removed from this tree."* This is the deliberate deletion log
line, not a stale reference. It is the one place in the
documentation where the cleanup is named. Removing it would make
the deletion less self-describing.

### 3.4 `website` / `roadmap` / `archive` / `release note` / `certifier`

#### 3.4.1 `cellscript/CODING_STYLE.md` — guidance prose

```text
[JUSTIFIED] guidance prose, kept
```

References:
```text
"Run `cargo check --locked -p cellscript --all-targets` before
 committing routine compiler or documentation changes."
"Release notes should separate highlights, scope boundaries, validation
 commands, and links to detailed docs."
"Do not keep roadmap promises in `docs/`. Active docs describe the
 current compiler boundary."
```

These are guidance for the maintainer, not references to deleted
files. They are correct after the deletion pass.

#### 3.4.2 `cellscript/docs/README.md` — map and historical note

```text
[JUSTIFIED] documentation map, kept
```

States the active docs map and notes the deletion of historical
release notes, archived plans, standalone website material, and
proposal-era roadmaps. The map itself is the post-cleanup state.

#### 3.4.3 `cellscript/docs/CELLSCRIPT_CKB_DEPLOYMENT_MANIFEST.md`

```text
[JUSTIFIED] release-evidence guidance, kept
```

Contains the phrase *"leaving them scattered across scripts,
builders, and release notes."* This is a single-sentence general
observation about deployment evidence, not a reference to a
specific release note file. It is guidance prose, not stale.

#### 3.4.4 `cellscript/docs/CELLSCRIPT_GATE_POLICY.md`,
`cellscript/docs/CELLSCRIPT_SYNTAX_COMBO_AUDIT_METHODOLOGY.md`,
`cellscript/docs/CELLSCRIPT_SURFACE_ELEGANCE_RFC.md`,
`cellscript/docs/CELLSCRIPT_GRAMMAR_GOVERNANCE_RFC.md`,
`cellscript/docs/CELLSCRIPT_COLLECTIONS_SUPPORT_MATRIX.md`,
`cellscript/docs/CELLSCRIPT_EXAMPLE_BUSINESS_FLOWS.md`

```text
[JUSTIFIED] active reference docs, kept
```

These are the active reference documents for the CellScript
compiler that drives the Myelin typed-cell execution path. They
explicitly removed the `cellscript_gate.sh` references and the VS
Code references in the diff under audit. They do not advertise
proposal-era roadmaps.

#### 3.4.5 Wiki tutorials

```text
[JUSTIFIED] active tutorials, kept
```

`cellscript/docs/wiki/Cookbook-Recipes.md`,
`cellscript/docs/wiki/Home.md`,
`cellscript/docs/wiki/Tutorial-04-Packages-and-CLI-Workflow.md`,
`cellscript/docs/wiki/Tutorial-06-Metadata-Verification-and-Production-Gates.md`,
`cellscript/docs/wiki/Tutorial-07-LSP-and-Tooling.md`,
`cellscript/docs/wiki/Tutorial-08-Bundled-Example-Contracts.md`,
`cellscript/docs/wiki/Tutorial-11-Scoped-Invariants-and-ProofPlan.md`
all lost their VS Code / 0.13 / 0.14 / 0.15 / 0.16 / 0.17 release-note
links in the previous preparation pass. They now point at the
current release-gate boundary and the metadata-system audit. They
do not carry stale references.

### 3.5 `CKB fork` / `PoW` / `miner` / `mining` / `full node` / `L1 sync`

```text
[JUSTIFIED] no occurrences
```

The Myelin tree is not a CKB full-node fork. `rg -i 'full[_ -]node'`
in the active Myelin source paths returns no matches. The only
"PoW" / "miner" / "mining" matches are the doc comments on
`CkbRawHeader.compact_target` and `CkbHeader.nonce` (now renamed to
"Compact CKB header target field" and "CKB header nonce field"
respectively, see §3.3.2). These are honest: the bytes are
required to exist in the CKB Molecule wire layout even though
Myelin does not perform PoW with them.

The Myelin `README.md`, `docs/ARCHITECTURE.md`, and
`docs/MYELIN_ARCHITECTURE.md` are explicit:

```text
"It is not a CKB full-node fork, not a new L1, and not a finished
permissionless L2 in its current phase."
```

## 4. Surfaces deleted in this hardening pass

No additional files had to be deleted in this hardening pass to
clear stale identity. The previous preparation pass already
removed the certifier, the `certify` CLI subcommand, the
NovaSeal/website/VS Code directories, the proposal-era roadmaps,
the release-note/roadmap/archive/0.13/0.17/0.18/0.19/0.20 docs,
the `cellscript_gate.sh` wrapper, and the `shell.nix`.

The hardening pass did make the following *targeted* identity
corrections to keep wording consistent:

```text
README.md                                              [updated]
docs/ARCHITECTURE.md                                   [updated]
docs/MYELIN_ARCHITECTURE.md                            [updated]
```

These are wording-only updates. They replace the phrase
"static committee finality" with "selectable closed-validator
finality" wherever the public claim is being made, so Myelin's
external description matches its actual two-engine
`SelectedConsensus` shape (static-closed-committee and Tendermint).

## 5. Conclusion

After the previous preparation pass plus this hardening pass,
the standalone Myelin tree:

```text
- has zero Spora / spora occurrences;
- has zero NovaSeal / novaseal occurrences;
- has zero references to the removed certifier module, the
  `cellc certify` subcommand, the removed proposal-era roadmaps,
  the removed release-note/roadmap/archive docs, the removed
  website/VS Code surfaces, or the removed `cellscript_gate.sh`
  wrapper;
- has zero CKB-full-node / PoW / miner / mining / L1-sync language
  in active code, except for the unavoidable wire-format fields
  on the CKB RawHeader Molecule struct that the projection layer
  is required to reproduce;
- documents its public claim boundary in terms of
  `selectable closed-validator finality` (static + Tendermint)
  rather than the older single-engine wording;
- depends on the parent `Spora` folder for nothing.
```

Myelin is now structurally independent of the deleted surfaces.
