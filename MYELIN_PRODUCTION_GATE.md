# Myelin Production Gate

> The executable release gate for the standalone Myelin runtime prototype.
> The gate is `scripts/myelin_production_gate.sh`. It proves the current
> Session L2 spine and workload evidence; it is not, by itself, proof that
> Myelin is ready for mainnet custody or permissionless production use.

## 1. What the gate runs

In order:

```text
1. cargo fmt --all --check
2. git diff --check
3. cargo check --locked --workspace --all-targets
4. cargo clippy --locked --workspace --all-targets -- -D warnings
5. cargo test --locked --workspace (focused protocol crates)
5b. cargo test --locked -p myelin-state
5c. cargo test --locked -p myelin-mempool
6. cargo test --locked -p myelin-consensus
7. cargo check --locked -p cellscript --all-targets
8. CLI smoke: myelin-cli committee finalise-demo for static
9. CLI smoke: myelin-cli committee finalise-demo for tendermint
10. CLI JSON contract validation
11. Runtime smoke for static-closed-committee and tendermint
12. Session fixture open/commit/court/DA/settlement plus mock CKB context,
    economics, inclusion, stability, finality, and aggregate readiness
    verification for both consensus modes
13. Dependency-tree scan for forbidden workflow crates
14. Stale-surface grep
15. Forbidden parent-path audit
16. Teeworlds acceptance, if enabled
17. Teeworlds reproducibility report regeneration
```

## 2. Why each step exists

| Step | Why |
|---|---|
| 1 | Formatting drift is the easiest silent regression. Catching it before any other check keeps the diff minimal. |
| 2 | Whitespace and conflict markers should never land in committed code. |
| 3 | The full workspace must compile, locked, with no missing dependencies. |
| 4 | Strict Clippy keeps the release candidate free of lints across all workspace targets, including examples and tests. |
| 5 | The focused protocol crates have explicit unit tests. The full `--workspace` test is intentionally not used here because the cellscript subworkspace has its own test surface; we run that separately. |
| 5b/5c | The state and mempool crates have larger test sets; running them is required for production-readiness. |
| 6 | The consensus crate has 22 tests; this is the consensus completeness gate. |
| 7 | The cellscript compiler is a separate workspace and is required for the typed-cell execution path. |
| 8-10 | The CLI smoke tests prove that both consensus modes are reachable from the binary. |
| 11 | Runtime smoke proves `CellPool`, `CellStateTree`, and both consensus engines are wired through the CLI. |
| 12 | Session fixture checks prove the Session L2 spine can open, commit, materialise a court bundle, verify that bundle, emit and verify a DA manifest, emit and verify a deterministic CKB-compatible DA anchor package, emit and verify a disputed-close settlement intent, emit and verify a deterministic CKB-compatible settlement package, and run mock CKB context, capacity/fee-rate economics, inclusion, committed-block stability, confirmation-depth finality, and aggregate readiness verification for both consensus modes. Configurable `session open` / `session commit` commands are covered by CLI tests. |
| 13 | The dependency-tree scan guards against broad workflow surfaces leaking back into hot-path crates. |
| 14 | The stale-surface grep is the structural guard against re-introducing the removed Spora / NovaSeal / certifier / website / cellscript_gate.sh / release-note vocabulary. |
| 15 | The forbidden parent-path audit is the structural guard against re-introducing a Cargo `path = ".../Spora/..."` reference. |
| 16 | The Teeworlds acceptance is the executable workload evidence target for the CKB-style projection / court-bundle / static-committee finality path. |
| 17 | The Teeworlds reproducibility report is the canonical JSON artefact that downstream consumers read. |

## 3. Failure modes the gate catches

```text
- Spora references in active Myelin surfaces
- NovaSeal / proposal / certifier surfaces (re-added by mistake)
- broken CLI examples
- missing consensus mode
- broken Session L2 open/commit/court/verify/DA/settlement flow
- Session state transition drift across consensus engines
- court-compatible session report without the minimal CKB-strict profile
- DA manifest that is not bound to the exact court replay payload
- settlement intent that is not bound to a verified court bundle and elapsed
  challenge window
- settlement package that is not bound to the exact intent, court bundle, DA
  manifest, challenge payload, and deterministic CKB-compatible CellTx
- inclusion report without a committed block number for confirmation-depth
  finality
- stability report where a previously committed transaction moved to a different
  block hash or block number
- submission economics report with underfunded outputs, insufficient fee, or
  insufficient fee rate
- submission economics report that burns surplus above the configured max-fee
  cap instead of proving exact funding or explicit change handling
- finality report that cannot prove the configured confirmation depth
- aggregate readiness report assembled from mismatched transaction or block
  evidence
- projection false-positive
- Teeworlds acceptance regression
- cargo fmt / cargo check / cargo clippy / cargo test failure
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
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked -p myelin-consensus
cargo check --locked -p cellscript --all-targets
cargo run -p myelin-cli -- session open-fixture --out /tmp/session-open.json
cargo run -p myelin-cli -- session commit-fixture --session /tmp/session-open.json --out /tmp/session-commit.json
cargo run -p myelin-cli -- session court-bundle --commit /tmp/session-commit.json --chunk-index 0 --out /tmp/session-court.json
cargo run -p myelin-cli -- session verify-court-bundle --bundle /tmp/session-court.json --out /tmp/session-court-verify.json
cargo run -p myelin-cli -- session da-manifest --bundle /tmp/session-court.json --storage-dir /tmp/session-da-store --out /tmp/session-da.json
cargo run -p myelin-cli -- session verify-da-manifest --manifest /tmp/session-da.json --bundle /tmp/session-court.json --storage-dir /tmp/session-da-store --out /tmp/session-da-verify.json
cargo run -p myelin-cli -- session settlement-intent --bundle /tmp/session-court.json --da-manifest /tmp/session-da.json --kind disputed-close --current-time-ms 60000 --challenge-window-ms 60000 --out /tmp/session-settlement.json
cargo run -p myelin-cli -- session verify-settlement-intent --intent /tmp/session-settlement.json --bundle /tmp/session-court.json --da-manifest /tmp/session-da.json --out /tmp/session-settlement-verify.json
```

The full gate is the recommended form for any merge or prototype release.

## 5. How to run

```bash
scripts/myelin_production_gate.sh
```

The default `RUN_TEEWORLDS=1` runs the Teeworlds acceptance and
**the gate hard-fails** if the Teeworlds replayer or the
`rust-tools` manifest is missing. To intentionally run on a
machine without the Teeworlds checkout, set
`ALLOW_SKIP_TEEWORLDS=1` to downgrade the missing-Teeworlds case
to a skip with a printed warning instead of a hard fail.

```bash
# Default: Teeworlds acceptance is required; missing replayer is a
# hard failure.
scripts/myelin_production_gate.sh

# Explicit skip: Teeworlds acceptance is skipped with a warning.
ALLOW_SKIP_TEEWORLDS=1 scripts/myelin_production_gate.sh

# Opt out of Teeworlds entirely (same as ALLOW_SKIP_TEEWORLDS=1).
RUN_TEEWORLDS=0 scripts/myelin_production_gate.sh
```

The Teeworlds path is at:

```text
TEEWORLDS_ROOT=/Users/arthur/RustroverProjects/teeworlds
```

This is overridable by env var. The gate verifies both the
replayer and the `rust-tools` manifest are present before
running the Teeworlds acceptance.

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
court_checks              : 22
```

The Session L2 section additionally emits static and Tendermint reports under
`/tmp/myelin-production-gate/session-*.json`; both verified court bundles must
be valid, and the gate asserts that the session id, CellTx commitments,
scheduler commitment, and state roots match across consensus engines while the
block/finality domains remain separated. It also emits and verifies
`session-da-*.json`, `session-da-anchor-*.json`,
`session-da-anchor-submit-*.json`, `session-da-anchor-context-*.json`,
`session-da-anchor-economics-*.json`, `session-da-anchor-inclusion-*.json`,
`session-da-anchor-stability-*.json`, `session-da-anchor-finality-*.json`,
`session-da-anchor-readiness-*.json`,
`session-settlement-*.json`, and
`session-package-*.json` / `session-package-submit-*.json` /
`session-package-context-*.json` / `session-package-economics-*.json` /
`session-package-inclusion-*.json` / `session-package-stability-*.json` /
`session-package-finality-*.json` / `session-package-readiness-*.json`.
The DA reports must prove the exact court replay payload under the current
single-segment Merkle profile, be backed by sealed local segment storage, and
keep `l1_da_published = false` visible. The DA anchor reports must recompute
their embedded Molecule transaction, CellTx ids, and CKB-compatible projection
while keeping `l1_da_publication_implemented = false` visible. The dry-run DA
anchor submission reports must build and hash a CKB `send_transaction` JSON-RPC
request while keeping `submitted_to_rpc = false` and `l1_da_published = false`
visible. Direct live DA-anchor package submission preflights projected inputs
and cell deps through CKB `get_live_cell` and refuses to broadcast stale or
underfunded projected transactions; it records `accepted_by_rpc = true` when
the RPC result matches the projected CKB raw transaction hash, but keeps
`l1_da_published = false` because the final L1 DA publication script is still
not implemented. The settlement
reports must bind to the verified court bundle, verified DA manifest, and
elapsed challenge window, and must keep both
`l1_da_published = false` and `l1_court_implemented = false` visible. The
package reports must recompute their embedded Molecule transaction, CellTx ids,
and CKB-compatible projection while keeping `l1_court_script_implemented =
false` visible. The dry-run settlement submission reports must build and hash a
CKB `send_transaction` JSON-RPC request while keeping `submitted_to_rpc = false`
and `l1_court_submitted = false` visible. Direct live settlement package
submission preflights projected inputs and cell deps through CKB `get_live_cell`
and refuses to broadcast stale or underfunded projected transactions; it records
`accepted_by_rpc = true` when the RPC result matches the projected CKB raw
transaction hash, but keeps `l1_court_submitted = false` because the final L1
court script is still not implemented. The mock inclusion reports must query
CKB-style `get_transaction` through the same HTTP JSON-RPC client and observe a
committed status with a block hash and block number. The mock stability reports
must re-query `get_transaction` and prove that the committed block hash and
block number are unchanged. The mock finality reports must query CKB-style
`get_tip_header`, compute confirmation depth, and satisfy the configured
confirmation policy. The mock context reports must query
CKB-style `get_live_cell` and prove every transaction input and cell dep
resolves as live. The mock economics reports must query CKB-style
`get_live_cell`, prove live input capacity covers output capacity, and prove the
configured fee floor, fee-rate policy, max-fee cap, and output accounting are
met. The economics verifier reports data-bearing outputs, empty-data outputs,
and explicit change-like outputs so surplus returned as change is distinguishable
from surplus burned as implicit fee. The gate uses exact-funded mock inputs so
missing change-output construction cannot be hidden as a large implicit fee. The
readiness aggregate must also reject report mixing: context, economics, and
inclusion reports must share the same submission artefact/schema lineage, and
stability/finality reports must point back to the supplied inclusion artefact.
It must also re-open the referenced submission report and prove that it is
readable, schema-matching, and bound to the same expected CKB transaction hash.
The stricter `--require-live-submission` readiness mode additionally requires
the referenced submission report to prove non-dry-run RPC acceptance through
either the deployed carrier path or the deployed final-script path with a
matching `rpc_result`, non-empty RPC URL, and no RPC error; the live parent-CKB
devnet smoke uses that stricter mode for both evidence roles. Final-script
strict readiness also requires the referenced submission report to carry its
positive live pre-submit markers: funding input, lock dep, verifier code dep and
verifier code hash checks, plus final DA evidence and settlement-authority
input data/lock checks for final settlement. The production gate keeps its
direct package checks in dry-run/mock-evidence mode.
Readiness JSON now labels this distinction explicitly with
`readiness_evidence_mode`, `live_carrier_submission_ready`, and
`final_l1_script_submission_ready`; the default production gate expects
`coherent-offline-or-mock`, while the live devnet smoke expects
`live-ckb-carrier` for carrier submissions and `final-l1-script` for
final-script submissions.
The carrier submission builder rejects carrier and change outputs below their
estimated CKB occupied-capacity minimum before it emits a `send_transaction`
request. It also rejects package/payload drift: the compact carrier payload kind
must match the package schema, and each 32-byte DA-anchor or settlement payload
field must match the package's declared hashes.

And the reproducibility report was regenerated at:

```text
reports/myelin-teeworlds-repro.json
```

## 7. Conclusion

The production gate is the single source of truth for whether the current
release-candidate prototype is internally consistent. It exercises both
consensus modes, runtime smoke, the Session L2 open/commit/court/verify spine,
DA payload proof binding, DA anchor RPC dry-run construction, mock live-cell
context verification, mock capacity/fee-rate economics verification, mock inclusion
verification, mock committed-block stability verification, mock finality-depth
verification, readiness lineage binding, settlement-intent binding, settlement
RPC dry-run construction, the projection layer, scheduler witnesses, the celltx
execution report, strict Clippy, the Teeworlds acceptance, and the structural
stale-surface guarantees. It does not prove mainnet custody safety, live
public-chain DA/court inclusion and finality, wallet funding/change-output
construction, an implemented CKB court script, permissionless validator
operation, or sustained production load.

The separate optional live-devnet smoke is:

```bash
scripts/myelin_ckb_devnet_smoke.sh
```

It uses the parent `../ckb` checkout, starts a throw-away devnet, mines a
spendable always-success funding cell, deploys separate compact CellScript
DA-anchor and settlement carrier verifiers, commits a 160-byte Myelin DA-anchor
carrier payload into CKB output data guarded by the DA `data2` type script, and
binds carrier type args to `ckb_data_hash(carrier_payload) ||
carrier_identity_hash`, where the identity hash is the DA manifest hash or
settlement intent hash. The checked-in carrier verifiers reject zero hash fields
and use `ckb::cell_exists(source::group_output(1))` to reject duplicate
same-type group outputs. The generated settlement package is submitted through
a second 160-byte compact carrier funded from the first carrier's change
output, using the settlement verifier and its own type-args
data-hash-plus-identity layout.
Carrier transaction reports and the exact `send_transaction` JSON-RPC requests
are built by `myelin session carrier-submission`; the smoke uses that same
command's `--submit --require-accepted` path to send valid carrier transactions
to CKB and record the accepted transaction hash in the submission report. The
carrier command computes the projected CKB raw transaction hash from the same
transaction it broadcasts, stores that hash in `ckb_raw_tx_hash`, and only marks
`accepted_by_rpc = true` when a live `--submit` RPC result equals the projected
raw hash. Operator-supplied `--accepted-tx-hash` evidence is recorded
separately and cannot satisfy strict live production readiness.
The smoke copies `cellscript/examples/myelin/da-anchor-carrier.cell`,
`cellscript/examples/myelin/settlement-carrier.cell`,
`cellscript/examples/myelin/da-anchor-final.cell`, and
`cellscript/examples/myelin/settlement-final.cell` into the throw-away workdir,
compiles all four checked-in CellScript sources under the `typed-cell` profile
before compiling their CKB-profile ELFs, and records the typed-cell ELF plus
metadata sidecar paths in `carrier_verifiers.*` and `final_script_verifiers.*`.
Carrier submission reports
also record a SHA-256 hash of the CellScript verifier source when that source
file is available, next to the deployed verifier code hash and code dep. Live
carrier submission requires the verifier source path to be readable, so accepted
live carrier reports cannot silently carry a null verifier source hash. Before
broadcasting through `send_transaction`, the CLI preflights the declared input
and code deps through CKB `get_live_cell`, rejects a missing or stale input,
rejects an input capacity that differs from the declared funding capacity,
rejects a live funding-cell lock that differs from the declared funding/change
lock, and rejects a verifier code dep whose live `cell.data.hash` differs from
the declared verifier code hash.
It requires CKB to report both carrier transactions as `committed`, and runs
Myelin's existing submission context, economics, inclusion, stability, finality,
and aggregate readiness verifiers against live devnet evidence for both carriers.
The aggregate readiness step passes `--require-live-submission`, so
`production_submission_ready` cannot become true unless the original carrier
submission report proves carrier-schema non-dry-run RPC acceptance with a
matching RPC result as well as inclusion, stability, finality, context, and
economics readiness. Those readiness reports must also carry
`readiness_evidence_mode = "live-ckb-carrier"` and
`live_carrier_submission_ready = true`.
It then submits tampered compact-payload carriers under both deployed verifiers
and requires CKB script verification to reject them. The inclusion
verifier treats carrier inclusion as live only when CKB `outputs_data[0]`
matches the declared carrier payload and `outputs[0].type` matches the declared
verifier code hash, `data2` hash type, and expected data-hash-plus-identity
args. The context verifier treats carrier context as ready only when the
declared verifier code dep is live and CKB returns a `cell.data.hash` matching
the declared verifier code hash. The smoke only sets
`all_live_checks_passed` when both valid carriers are ready and both tampered
carriers are rejected. This is live
CKB devnet evidence for the deployed compact carrier path plus deployed final
verifier artefact code cells; it is intentionally not an always-on gate and
does not yet replace final DA/court submission integration.

The local CellScript tests
`v0_18_myelin_package_commitment_has_typed_cell_metadata_and_ckb_vm_rejects_tamper`
and
`v0_18_generic_package_commitment_binds_data_hash_to_type_args_in_ckb_vm` keep
the package-commitment carrier boundary covered. The
`v0_18_myelin_da_and_settlement_carriers_bind_compact_payloads_to_type_args_in_ckb_vm`
test compiles typed-cell DA-anchor carrier, settlement carrier, final DA
publication, and final settlement resources and executes their compact-payload
verifier shapes as CKB type scripts under `ckb-testtool`, accepting matching
output data plus identity-bound type args, and rejecting duplicate same-type
group outputs, tampered output data, mismatched identity args, non-canonical
extended type args, and self-consistent zero-hash-field payloads. The CLI
inclusion tests additionally reject a carrier transaction
committed under a different type script code hash from the verifier declared in
the submission report. The CLI context tests reject a carrier submission whose
declared verifier code dep resolves to a different data hash. The CLI economics
tests distinguish explicit returned change from surplus burned as implicit fee.
The optional live-devnet smoke now provides the matching separate verifier
deployment evidence and a live negative rejection check.

The latest local run passed against parent CKB:

```text
Report: /tmp/myelin-ckb-devnet.J0dAY0/myelin-ckb-devnet-smoke.json
ckb_version: ckb 0.206.0 (5ebbc39 2026-04-10)
da_anchor_ready: true
settlement_ready: true
da_anchor_typed_cell_profile_checked: true
settlement_typed_cell_profile_checked: true
tamper_rejected_by_rpc: true
all_live_checks_passed: true
```
