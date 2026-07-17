# CellScript 0.21.0-rc.1 — Wiki + Extension Stale-Audit Conclusion

> **Historical / resolved audit snapshot.** This file is an untracked audit
> artifact from the 2026-07-04 pre-remediation state. Several findings below
> have already been fixed in the current checkout, including the wiki update
> date, `cellscript-mcp` naming, Tutorial 09 canonical filename, compile receipt
> snippets, and the release-pinned Status lines called out in the TL;DR. Do not
> treat this file as the current 0.21 action plan unless each finding is
> re-verified against the live tree.

**Audit date:** 2026-07-04
**Pinned release boundary:** `0.21.0-rc.1` (`Cargo.toml:19`, released 2026-07-01)
**Scope:** `docs/wiki/`, `editors/vscode-cellscript/`, `docs/CELLSCRIPT_*.md`
**Verifier verdict:** PASS (all spot-checks, cross-cutting patterns, action-plan counts, executive summary held)
**Full report:** `synthesis.md` in the plan scratchpad; per-track audits in `plans/plan_491ac04d/outputs/`

---

## Snapshot TL;DR (2026-07-04) — yes, all three layers were stale, but for three different reasons

| Layer | Stale? | Single most important reason |
|---|---|---|
| **`docs/wiki/`** | **YES** | `Home.md:9` says `Last updated: 2026-06-28` — pre-dates the 0.21.0-rc.1 cut by 3 days. On top of that, the wiki still uses the 0.20-era binary name `cellc-mcp` in 5 places, still teaches 0.13-syntax framing in `Tutorial-09`, and ships a `Cookbook-Recipes.md:118-132` destruction recipe that **fails compile** under the very `--primitive-strict 0.16` gate the recipe itself recommends. The wiki has no automated self-check, so staleness accumulates silently. |
| **`editors/vscode-cellscript/`** | **YES — drift on scope-cut transparency, not functionality** | The extension is operationally current (package version matches, every CLI command resolves, `validate.mjs` passes, every keyword highlighted except `i32`). But the intentional 0.21 scope cut (MCP server, receipt family, `verify-artifact`, one-click `cellc check`/`build`) is **honest in `CHANGELOG.md:3-11`** and **silent in `README.md`**. A user reading the README cannot tell the gaps are by design. |
| **`docs/CELLSCRIPT_*.md`** | **YES — Status-line drift on release-pinned docs that README points at** | `README.md:428-429` links to `CELLSCRIPT_CKB_ADAPTER.md` and `CELLSCRIPT_CKB_STD_COMPAT.md`, both of which still say "0.19" in their `Status:` line while the body documents 0.21 surface. Same problem on `CELLSCRIPT_GRAMMAR_GOVERNANCE_RFC.md:5` and `CELLSCRIPT_WEBSITE_PARADIGM_UPGRADE_RFC.md:5`. The supersession chain (SURFACE_ELEGANCE → GRAMMAR_GOVERNANCE) is topologically intact, but the **head of the chain misrepresents current reality**, so a reader landing on it sees a 0.19 label and assumes the body is also stale (it is not). |

**Bottom line:** wiki is the worst (compile-breaker + identity drift + missing 0.21 surface). Extension is the best in absolute terms but has a transparency gap. Docs/ is the most insidious because the Status-line drift is invisible until a reader cross-checks against `Cargo.toml`/`CHANGELOG`, and the `README → CELLSCRIPT_CKB_*.md` link chain amplifies it into the top of the documentation tree.

---

## Snapshot top critical findings (5 of 14)

| # | File:line | One-line summary |
|---|---|---|
| 1 | `docs/wiki/Cookbook-Recipes.md:118-132` | "Choose A Destruction Policy" recipe: four `destroy_*` calls without `resource … has store, consume, burn {…}` declarations → fails compile under `--primitive-strict 0.16`. |
| 2 | `docs/wiki/_Sidebar.md:16`, `Home.md:50,82`, `Tutorial-12:138`, `Tutorial-13:1,11,21` | Multi-file `cellc-mcp` → `cellscript-mcp` rename not propagated (5 places in the wiki; 1 cross-link in `docs/skills/cellscript-diagnostics/SKILL.md:5`). |
| 3 | `docs/wiki/Tutorial-09-Action-Model-and-0-13-Syntax.md` (filename + H1) | File is titled "0-13 Syntax" and `_Sidebar.md:12` mirrors it; body teaches canonical 0.21 syntax. |
| 4 | `docs/CELLSCRIPT_CKB_STD_COMPAT.md:6`, `CELLSCRIPT_CKB_ADAPTER.md:3-4`, `CELLSCRIPT_GRAMMAR_GOVERNANCE_RFC.md:5`, `CELLSCRIPT_WEBSITE_PARADIGM_UPGRADE_RFC.md:5` | Four release-pinned docs carry `Status:` lines that still say "0.19" / "0.20" / "**Proposed**" while the body documents 0.21. |
| 5 | `docs/wiki/Tutorial-08-Bundled-Example-Contracts.md:11-22` | Bundled-examples table lists 7; repo ships 10 (`examples/{token,nft,timelock,multisig,vesting,amm_pool,launch,registry,atomic_swap,multi_phase_dao}.cell`). |

---

## Cross-cutting patterns (the highest-leverage signal)

A defect reported in **multiple tracks** is more actionable than the same defect in one track.

### Triplet (3-track) feature_gaps — block-release

1. **`cellc` nested CLI groups** (`tx`, `deploy`, `registry`, `package`, `auth capability`)
   — wiki: still uses flat aliases in Tutorial-04/08/12/Cookbook/Home; extension: wires `hide(true)` legacy aliases at `extension.js:652,660,668` (so 0.22 will silently break the extension); docs-ref: no contract doc teaches the nested form.
2. **Compile receipts** (`cellc receipt` / `sign-receipt` / `verify-receipt`, `verify-artifact --receipt`)
   — wiki: one buried snippet inside a section headed `## v0.16 Assurance Checks`; extension: no commands surface receipts at all; docs-ref: zero coverage.
3. **`cellscript-mcp` + 6 programming skills** (`docs/skills/cellscript-{diagnostics,language-basics,metadata-audit,package-cli,ckb-model,builder-deployment}/SKILL.md`)
   — wiki: lists 10 MCP tools but never names the 6 skills; extension: silent; docs-ref: zero coverage. The skills reference the wiki (which still uses the old `cellc-mcp` filename) and the wiki references the skills — circular cross-link with no anchoring.

### Two-track feature_gaps — should-fix

`template_layouts` cycle policy + `consensus_checked=true` rejection · `--message-format=json` / `--color=` / `NO_COLOR` · `--primitive-strict 0.17` / `PP0170` / `gap:runtime-helper-required` · flow-edge membership validation · `atomic_swap.cell` / `multi_phase_dao.cell` non-production additions.

### Identity defects

`cellc-mcp` → `cellscript-mcp` rename incomplete across 5 wiki files + 1 skill cross-link. Source of truth is `Cargo.toml:58` and `CHANGELOG.md:37`.

### Date drift

`Home.md:9` says `2026-06-28`; release is `2026-07-01`. One-line fix.

---

## Snapshot action plan — 25 items, sorted by severity

### Block-release (5) — must land before `0.21.0-stable`

| Tag | Track | Invasiveness | Fix |
|---|---|---|---|
| **BR-1** | wiki | new section | `Cookbook-Recipes.md:118-132` — add resource declarations (`resource Config has store, consume, burn {…}`) before each `destroy_*` call. |
| **BR-2** | wiki + skills | ~7 one-line edits | `cellc-mcp` → `cellscript-mcp` rename across 4 wiki files + 1 skill file (`docs/skills/cellscript-diagnostics/SKILL.md:5`). |
| **BR-3** | wiki | 6 file edits | Rename `Tutorial-09-Action-Model-and-0-13-Syntax.md` → `Tutorial-09-Action-Model-and-Canonical-Syntax.md`, update H1, update 5 cross-refs (`_Sidebar.md:12`, 4× in `Home.md`, 1× in `Tutorial-03:328`). |
| **BR-4** | docs-ref | 4 Status edits + 3 follow-up sections | Rewrite Status lines of `CELLSCRIPT_CKB_STD_COMPAT.md:6`, `CELLSCRIPT_CKB_ADAPTER.md:3-4`, `CELLSCRIPT_GRAMMAR_GOVERNANCE_RFC.md:5`, `CELLSCRIPT_WEBSITE_PARADIGM_UPGRADE_RFC.md:5` to remove the pinned-to-old-release shape; add "0.21 extensions" cross-reference paragraphs where the body documents 0.21 surface. |
| **BR-5** | docs-ref | gate-script check | Add `README-linked CELLSCRIPT_*.md Status freshness` check to `scripts/cellscript_gate.sh` so the chain `README → release-pinned-doc → CHANGELOG` is enforced at the release gate. |

### Should-fix (9) — ship in `0.21.x`

| Tag | Track | Invasiveness | Fix |
|---|---|---|---|
| **SF-1** | wiki + ext + docs-ref | new section (~30 lines) | `Tutorial-04-Packages-and-CLI-Workflow.md` — add `## 0.21 CLI Command Groups` appendix listing the canonical nested forms + a footnote on flat-alias deprecation. |
| **SF-2** | wiki + ext + docs-ref | new section + 1 recipe | Move the receipts snippet out of `## v0.16 Assurance Checks` into a new `## 0.21 Compile Receipts` section in `Tutorial-06`; add a "Sign and Verify a Receipt" recipe in `Cookbook-Recipes.md`. |
| **SF-3** | wiki | new subsection (~20 lines) | `Tutorial-06` — explain `cycle_policy = RootRequired` / `PathOnlyAllowed`, the `consensus_checked=true` rejection in `src/lib.rs:3159-3162`, and the three ProofPlan coverage states (`gap:metadata-only` / `gap:runtime-helper-required` / `checked-runtime`). |
| **SF-4** | wiki + ext + docs-ref | new subsection (~15 lines) | `Tutorial-13` — add `## Skills` listing the 6 skill-pack names, pointing at `docs/skills/*/SKILL.md`, and mentioning the freshness check in dev/ci gates. |
| **SF-5** | wiki + docs-ref | new subsection (~12 lines) | `Tutorial-04` — add `## Diagnostic Output Formatting` covering `--message-format=json`, `--color=auto|always|never`, `NO_COLOR`. |
| **SF-6** | wiki + docs-ref | table extension + 1 bullet (~5 lines) | `Tutorial-11:298-305` — extend progressive-guarantee table from 3 rows to 4 (`0.17`, `PP0170`, `gap:runtime-helper-required`). |
| **SF-7** | extension | 3 one-line edits | `extension.js:652,660,668` — replace `["explain-assumptions", …]` → `["explain", "assumptions", …]`; `["solve-tx", …]` → `["tx", "solve", …]`; `["deploy-plan", …]` → `["deploy", "plan", …]`. |
| **SF-8** | extension | new bullet (~6 lines) | `editors/vscode-cellscript/README.md` — extend "Evidence Boundary" (L158-167) with an explicit statement that the 0.21 MCP server + receipt family are intentional scope cuts per `CHANGELOG.md:8-11`. |
| **SF-9** | wiki + docs-ref | table extension + 1 new section | `Tutorial-08:11-22, 42-44` and `CELLSCRIPT_EXAMPLE_BUSINESS_FLOWS.md:8-22` — add `atomic_swap.cell` / `multi_phase_dao.cell` marked "non-production" per `CHANGELOG.md:51`. |

### Nice-to-have (11) — drift cleanup before `0.22`

| Tag | Track | Invasiveness | Fix |
|---|---|---|---|
| **NT-1** | wiki | one-line edit | `Home.md:9` — `Last updated: 2026-07-01 (CellScript 0.21.0-rc.1)`. |
| **NT-2** | docs-ref | one-line edit | `CELLSCRIPT_0_20_ROADMAP.md:3` — Status "In progress" → "Scope complete; released 2026-06-28 as 0.20.0". |
| **NT-3** | docs-ref | 3 file moves | Move `CELLSCRIPT_0_{18,19,20}_ROADMAP.md` into `docs/archive/0.{18,19,20}/` (parallels existing `0.13/0.15/0.17` archives). |
| **NT-4** | docs-ref | new section (~15 lines) | `CELLSCRIPT_MOLECULE_IFRN_DESIGN_SPACE_IMPROVEMENT_REPORT.md` — add "0.21 closure addendum" naming `cell_data_codec_manifest` / `action_scan_selectors` / TemplateLayout metadata-only as shipped. |
| **NT-5** | docs-ref | 1 Status edit + 4 version bumps | `CELLSCRIPT_PACKAGE_PROVENANCE_AND_DEPLOYMENT_IDENTITY.md` — rewrite Status + bump 4 `compiler_version = "0.19.0"` examples to `"0.21.0-rc.1"`. |
| **NT-6** | docs-ref | 1 Status + 2 version bumps | `CELLSCRIPT_REGISTRY_PHASE1.md` — add Status line at top + bump 2 `compiler_version` examples. |
| **NT-7** | extension | one-token regex edit | `syntaxes/cellscript.tmLanguage.json:182` — add `i32` to the `storage.type.primitive.cellscript` regex (currently `u8|u16|u32|u64|u128|bool|Address|Hash`). |
| **NT-8** | extension | 1 snippet body edit + 5 new snippets | `snippets/cellscript.json` — replace the `transition` placeholder with the canonical field-path form; add `flow-edge-by-action`, `#[capabilities(...)]`, `module`/`use` skeleton, `enum`, `invariant` block. |
| **NT-9** | wiki | 5 new glossary entries | `CKB-Glossary.md` — add ProofPlan coverage state, TemplateLayout, ProtocolGraph, Compile receipt, `args_parts`, Scan-selector evidence, Manifest-backed CellDep. |
| **NT-10** | docs-ref | one-line cross-link | `CELLSCRIPT_CKB_ADAPTER.md:300-310` — cross-link to `CELLSCRIPT_GATE_POLICY.md:74-83` clarifying that the ecosystem-reuse and adapter-acceptance scripts are standalone tools, not part of the unified gate. |
| **NT-11** | docs-ref | new paragraph (~6 lines) | `CELLSCRIPT_LINEAR_OWNERSHIP.md` — cross-reference 0.21 flow-edge membership validation as a linearity rule change. |

---

## Snapshot audit chain health

The supersession chain (SURFACE_ELEGANCE_RFC → GRAMMAR_GOVERNANCE_RFC; 0.19 ROADMAP → both; 0.20 ROADMAP → GATE_REDUNDANCY_AUDIT → GATE_POLICY) is **topologically intact**. The links point in the right direction. **But four head-of-chain labels misrepresent current reality** (`CELLSCRIPT_CKB_ADAPTER.md`, `CELLSCRIPT_CKB_STD_COMPAT.md`, `CELLSCRIPT_GRAMMAR_GOVERNANCE_RFC.md`, `CELLSCRIPT_WEBSITE_PARADIGM_UPGRADE_RFC.md`). BR-4 fixes the labels; BR-5 enforces the chain at the release gate.

The redundancy audit (`CELLSCRIPT_GATE_REDUNDANCY_AUDIT.md`) is the only release-pinned audit doc whose Status line is current — a healthy reference point that the stale docs can be re-anchored to.

---

## Where to read more

| Document | Lines | Purpose |
|---|---|---|
| `plans/plan_491ac04d/outputs/wiki-audit/deliverable.md` | 3.3 KB | Wiki-track summary (17 files, 4 critical + 5 critical feature_gap + 25 warning/minor) |
| `plans/plan_491ac04d/outputs/extension-audit/deliverable.md` | 2.5 KB | Extension-track summary (`editors/vscode-cellscript/`, 0 critical + 2 warning + 13 minor) |
| `plans/plan_491ac04d/outputs/docs-ref-audit/deliverable.md` | 5.3 KB | Docs-ref-track summary (26 `docs/CELLSCRIPT_*.md`, 4 critical + 6 warning + ~12 minor) |
| `plans/plan_491ac04d/outputs/synthesize/deliverable.md` | 5.3 KB | Cross-audit synthesis summary |
| `synthesis.md` (plan scratchpad) | 842 lines | Full cross-audit synthesis — 75-row master finding table, 6 cross-cutting patterns, full 25-item action plan with file:line/proposed-text/invasiveness, audit-chain self-check |
