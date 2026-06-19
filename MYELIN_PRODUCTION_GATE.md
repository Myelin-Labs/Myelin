# Myelin Production Gate

> The single production-readiness gate for the standalone Myelin
> runtime. The gate is `scripts/myelin_production_gate.sh`. It is
> the final executable check for a release.

## 1. What the gate runs

In order:

```text
1. cargo fmt --all --check
2. git diff --check
3. cargo check --locked --workspace --all-targets
4. cargo test --locked --workspace (focused protocol crates)
4b. cargo test --locked -p myelin-state
4c. cargo test --locked -p myelin-mempool
5. cargo test --locked -p myelin-consensus
6. cargo check --locked -p cellscript --all-targets
7. CLI smoke: myelin-cli committee finalise-demo for static
8. CLI smoke: myelin-cli committee finalise-demo for tendermint
9. CLI JSON contract validation
10. Stale-surface grep
11. Forbidden parent-path audit
12. Teeworlds acceptance, if the Teeworlds repo path exists
13. Teeworlds reproducibility report regeneration
```

## 2. Why each step exists

| Step | Why |
|---|---|
| 1 | Formatting drift is the easiest silent regression. Catching it before any other check keeps the diff minimal. |
| 2 | Whitespace and conflict markers should never land in committed code. |
| 3 | The full workspace must compile, locked, with no missing dependencies. |
| 4 | The focused protocol crates have explicit unit tests. The full `--workspace` test is intentionally not used here because the cellscript subworkspace has its own test surface; we run that separately. |
| 4b/4c | The state and mempool crates have larger test sets; running them is required for production-readiness. |
| 5 | The consensus crate has 22 tests; this is the consensus completeness gate. |
| 6 | The cellscript compiler is a separate workspace and is required for the typed-cell execution path. |
| 7-9 | The CLI smoke tests prove that both consensus modes are reachable from the binary. |
| 10 | The stale-surface grep is the structural guard against re-introducing the removed Spora / NovaSeal / certifier / website / cellscript_gate.sh / release-note vocabulary. |
| 11 | The forbidden parent-path audit is the structural guard against re-introducing a Cargo `path = ".../Spora/..."` reference. |
| 12 | The Teeworlds acceptance is the executable evidence target for the CKB-style projection / court-bundle / static-committee finality path. |
| 13 | The Teeworlds reproducibility report is the canonical JSON artefact that downstream consumers read. |

## 3. Failure modes the gate catches

```text
- Spora references in active Myelin surfaces
- NovaSeal / proposal / certifier surfaces (re-added by mistake)
- broken CLI examples
- missing consensus mode
- projection false-positive
- Teeworlds acceptance regression
- cargo fmt / cargo check / cargo test failure
- forbidden dependency on the parent Spora folder
```

## 4. Reduced form

If the full workspace check is too slow for a quick iteration loop,
the gate is structured so the steps after step 6 are independent
and can be run individually:

```bash
cargo fmt --all --check
git diff --check
cargo check --locked --workspace --all-targets
cargo test --locked -p myelin-consensus
cargo check --locked -p cellscript --all-targets
```

The full gate is the recommended form for any merge or release.

## 5. How to run

```bash
scripts/myelin_production_gate.sh
```

The default `RUN_TEEWORLDS=1` runs the Teeworlds acceptance. Set
`RUN_TEEWORLDS=0` to skip it on machines that don't have the
Teeworlds checkout.

The Teeworlds path is at:

```text
TEEWORLDS_ROOT=/Users/arthur/RustroverProjects/teeworlds
```

This is overridable by env var. The gate also detects whether the
replayer and the `rust-tools` manifest are present, and skips the
Teeworlds acceptance if either is missing.

## 6. Live run

The hardening pass ran the full production gate and the gate passed.
The key line is:

```text
Myelin production gate passed.
Reports written under: /tmp/myelin-production-gate
```

The Teeworlds section of the gate produced:

```text
tape_bytes                : 2162
fixture_chunks            : 1
vm_cycles                 : 15_139_695
semantic_profile          : ckb-compatible
court_checks              : 14
```

And the reproducibility report was regenerated at:

```text
reports/myelin-teeworlds-repro.json
```

## 7. Conclusion

The production gate is the single source of truth for whether
Myelin is production-ready. It exercises both consensus modes, the
projection layer, the scheduler witnesses, the celltx execution
report, the Teeworlds acceptance, and the structural stale-surface
guarantees. The gate fails on any of the documented failure modes
and passes when the working tree is honest.
