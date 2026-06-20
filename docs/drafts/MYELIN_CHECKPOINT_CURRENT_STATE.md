# Myelin Current-State Checkpoint

> Archived checkpoint. This file records an intermediate dirty-worktree state
> from 2026-06-20 and is kept only as historical context. It is not the current
> release status, production-readiness source of truth, or public-testnet
> rehearsal report. Use `MYELIN_PRODUCTION_REHEARSAL_REPORT.md` for current
> release positioning.

Date: 2026-06-20

Scope: `/Users/arthur/RustroverProjects/Myelin`

This checkpoint intentionally stops the broader "make Myelin production-ready"
thread and narrows the next milestone to settlement uniqueness / anti-replay
hardening for final L1 settlement scripts.

## Git Status Summary

Current `git status --short` contains 74 entries:

- 70 tracked files appear in `git diff --stat`.
- Several untracked milestone artefacts are present:
  - `MYELIN_SESSION_L2_PLAN.md`
  - `MYELIN_USE_CASE_POSITIONING.md`
  - `cellscript/examples/myelin/`
  - `scripts/myelin_ckb_devnet_smoke.sh`

The tracked diff stat at checkpoint time is:

```text
70 files changed, 14147 insertions(+), 1481 deletions(-)
```

This checkpoint does not attempt broad cleanup. Existing dirty files are treated
as the baseline for the narrower anti-replay milestone.

## Known Green Commands

Recent green evidence before this checkpoint:

```bash
cargo check --locked -p myelin-cli
cargo fmt --all --check
bash -n scripts/myelin_ckb_devnet_smoke.sh scripts/myelin_production_gate.sh
cargo test --locked -p myelin-cli session_settlement_package -- --nocapture
cargo test --locked -p myelin-cli session_carrier_submission -- --nocapture
cargo test --locked -p myelin-cli session_submission_inclusion -- --nocapture
scripts/myelin_production_gate.sh
scripts/myelin_ckb_devnet_smoke.sh
```

The broad production gate passed, and the parent-CKB devnet smoke passed after
the 192-byte settlement authority lineage change.

## Live Devnet Evidence

Latest live parent-CKB smoke report:

```text
/tmp/myelin-production-gate/myelin-ckb-devnet-smoke.json
```

Relevant report facts at checkpoint time:

- `ckb_root`: `/Users/arthur/RustroverProjects/Myelin/../ckb`
- `ckb_version`: `ckb 0.206.0 (5ebbc39 2026-04-10)`
- `all_live_readiness_passed`: `true`
- `all_live_checks_passed`: `true`
- final DA readiness:
  - `production_submission_ready`: `true`
  - `strict_production_submission_ready`: `true`
  - `final_l1_script_submission_ready`: `true`
  - `readiness_evidence_mode`: `final-l1-script`
- final settlement readiness:
  - `production_submission_ready`: `true`
  - `strict_production_submission_ready`: `true`
  - `final_l1_script_submission_ready`: `true`
  - `readiness_evidence_mode`: `final-l1-script`

## Strict Readiness Semantics

- Mock/offline readiness is not strict production readiness.
- Live carrier readiness is not strict production readiness.
- Live final L1 script readiness can be strict production submission readiness
  when final script evidence, DA evidence, inclusion, stability, finality, and
  the required settlement checks all pass.

The next milestone must not weaken these semantics.

## Remaining Blockers

The project must still not claim full production readiness. Known remaining
blockers include:

- settlement uniqueness / anti-replay for final settlement proof cells;
- real DA availability beyond the finite devnet evidence path;
- complete court/dispute semantics and economics;
- operator policy for public-chain reorgs, retries, fees, key management, and
  monitoring.

## Target Of This Goal

The exact target is settlement uniqueness / replay protection only:

- define a minimal CKB-compatible uniqueness model for final settlement cells;
- harden the final settlement CellScript against duplicate, replayed, malformed,
  or semantically invalid settlement identities where the transaction-local
  script context can enforce it;
- expose machine-visible uniqueness evidence in CLI/readiness reports;
- prove the negative path with local CellScript regression coverage and the
  existing parent-CKB devnet smoke.
