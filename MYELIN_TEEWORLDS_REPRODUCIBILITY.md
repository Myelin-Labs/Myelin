# Myelin Teeworlds Reproducibility

> This document records the executable Teeworlds reproducibility
> path for the standalone Myelin runtime. The path uses xxuejie's
> cloned Teeworlds repository at `$HOME/RustroverProjects/teeworlds`
> as an external pressure workload, not as a Spora module.

## 1. Repo path

The Teeworlds repo is consumed by an explicit path. There is no
hidden local state; the path is part of the acceptance script and
is overridable by `TEEWORLDS_ROOT`.

```bash
TEEWORLDS_ROOT="${TEEWORLDS_ROOT:-$HOME/RustroverProjects/teeworlds}"
```

The acceptance script requires the following files to exist at
the Teeworlds repo path:

```text
$TEEWORLDS_ROOT/rust-tools/Cargo.toml
$TEEWORLDS_ROOT/ckb/build/replayer_stripped
$TEEWORLDS_ROOT/build/data/maps/dm1.map
$TEEWORLDS_ROOT/build/myelin_replay_40265.cfg
```

`scripts/myelin_teeworlds_acceptance.sh` checks each of them at
startup and exits non-zero if any is missing.

## 2. Determinism guarantees

The acceptance script is fully deterministic:

```text
- The scripted tape is built with a fixed seed (1), 300 ticks,
  1 client, and inputs every 5 ticks.
- The chunk size is 262144 bytes (one chunk covers the entire
  2162-byte scripted tape).
- The runs parameter is 3, so the benchmark reports
  3 measured runs of the inspect path.
- The CKB mock transaction is built deterministically from the
  replayer, tape, map, and config via xxuejie's
  `teeworlds-cli utils build-test-tx`.
```

The CLI smoke tests, the Teeworlds acceptance script, and the
unit tests all produce byte-stable outputs across runs (the only
varying component is the elapsed-ns measurement in the benchmark
report, which is wall-clock dependent).

## 3. Reproducible metrics

The current numbers from the live acceptance run (Teeworlds
acceptance gate, `RUN_TEEWORLDS=1`, 2026-06-19):

```text
tape_bytes                : 2162
fixture_chunks            : 1
vm_cycles                 : 15_139_695
semantic_profile          : ckb-compatible
ckb_projection_possible   : true (every chunk)
static_committee_finalised: true
court_checks              : 22
```

The numbers match the previously reported values byte-for-byte.

The court-bundle proof in the same run:

```text
molecule_transaction_bytes: 2616
ckb_raw_tx_hash           : present
ckb_wtx_hash              : present
static_committee_evidence.finalised: true
signatures                : 2 (validator-0, validator-1)
quorum_weight             : 2
challenge_payload_hash    : present, 64 hex chars
verify-court-bundle       : valid (all 22 checks ok)
```

## 4. Verifiable properties

The audit required:

| # | Property | Evidence |
|---|---|---|
| 1 | Locate Teeworlds by explicit path | `TEEWORLDS_ROOT` env var; `scripts/myelin_teeworlds_acceptance.sh` resolves it at the top of the script. |
| 2 | No hidden local state | `TEEWORLDS_ROOT` is the only required path; the rest of the state lives in `OUTPUT_DIR` (default `/tmp/myelin-teeworlds-acceptance`). |
| 3 | Deterministic fixture / chunk loading | `teeworlds-cli utils build-scripted-tape --seed 1 --ticks 300 --clients 1 --input-every 5` is deterministic. The mock-tx is built from the resulting tape. |
| 4 | Structured VM execution report | `vm-probe.json` reports `replayer`, `tape_bytes`, `map_bytes`, `config_bytes`, `max_cycles`, `ckb_strict: true`, `success: true`, `cycles: <u64>`, `error: null`. |
| 5 | Court-bundle verification passes | `verify-court-bundle` returns `valid: true` and all 22 checks ok. |
| 6 | CKB-compatible projection passes | Every chunk reports `semantic_profile: "ckb-compatible"` and `ckb_projection_possible: true`. |
| 7 | Static committee finality passes | `static_committee_finalised: true` on the inspect path; `static_committee_evidence.finalised: true` on the court-bundle path. |
| 8 | Tendermint finality also works | `teeworlds inspect --consensus tendermint`, `teeworlds bench --consensus tendermint`, and `teeworlds court-bundle --consensus tendermint` all produce a Tendermint-mode report that verifies. The new test `teeworlds_court_bundle_tendermint_precommit_path_verifies` proves the round-trip. |
| 9 | Reported metrics are reproducible | The acceptance gate re-runs the inspect path `runs=3` times; the average is reported. The other metrics (tape_bytes, chunks, vm_cycles, etc.) are deterministic and stable across runs. |

## 5. Tendermint Teeworlds path

The hardening pass extended the Teeworlds commands with a
`--consensus` flag that defaults to `static-closed-committee` and
accepts `tendermint`. This makes the Tendermint path a
first-class CLI surface for the same Teeworlds workload:

```bash
myelin-cli teeworlds inspect --mock-tx <path> --consensus tendermint
myelin-cli teeworlds bench   --mock-tx <path> --consensus tendermint --runs 3
myelin-cli teeworlds build-fixture --teeworlds-root <path> ... --consensus tendermint
myelin-cli teeworlds court-bundle --mock-tx <path> --consensus tendermint
myelin-cli teeworlds verify-court-bundle --bundle <path>
```

The Tendermint path produces a `TeeworldsCourtBundleReport` with:

```text
tendermint_evidence:
  consensus_kind   : "tendermint"
  block_hash       : <32-byte hex>
  quorum_power     : 2
  height           : 1
  round            : 0
  signer_ids       : [validator-0, validator-1]
  signatures       : [validator-0 sig, validator-1 sig]
  certificate_step : "precommit"
  finalised        : true
```

## 6. Court-bundle data-binding contract

The hardening pass added a structural data-binding guarantee to
the Teeworlds court-bundle path. The court-bundle now embeds the
canonical `MyelinBlock` whose fields are derived from the
runtime evidence:

```text
bundle.block:
  version                     : 1
  parent_hash                 : 0x00..00
  number                      : 1
  timestamp_ms                : 0
  consensus_kind              : "static-closed-committee" | "tendermint"
  state_root_before           : <derived from session_id + chunk_index>
  state_root_after            : <derived from old_state_root + commitment + map_hash + config_hash>
  ordered_cell_tx_commitments : [chunk_commitment]
  data_commitments            : [chunk_commitment]
  scheduler_commitment        : <derived from session_id + chunk_index + commitment>
  block_hash                  : <MyelinBlock::hash()>
```

The static-closed-committee certificate and the Tendermint
precommit certificate are both bound to `bundle.block.block_hash`,
which means both engines sign the **same canonical block** but
with different signature domains. This is the structural
guarantee that the runtime evidence, the finality certificate,
and the court-bundle record all refer to the same block.

The verifier (`verify-court-bundle`) now performs six new
data-binding checks:

```text
- block-hash-recomputes:
    MyelinBlock::hash() over bundle.block fields == bundle.block.block_hash
- block-state-root-before-matches:
    bundle.block.state_root_before == bundle.old_state_root
- block-state-root-after-matches:
    bundle.block.state_root_after == bundle.new_state_root
- block-scheduler-commitment-matches:
    bundle.block.scheduler_commitment == bundle.scheduler_report_hash
- block-data-commitment-matches:
    bundle.block.data_commitments[0] == bundle.chunk_commitment
- evidence-block-hash-matches-canonical-block:
    finality_evidence.block_hash == bundle.block.block_hash
```

The new tests in `cli/src/main.rs::tests` cover both the
positive path (well-formed bundle verifies with 22/22 checks
pass) and the negative path (tampered `state_root_after` causes
the data-binding check to fail). The audit tests now pass at
22/22 instead of the previous 14/14.

The new test `teeworlds_court_bundle_tendermint_precommit_path_verifies`
covers the full round-trip:

```text
- builds a Tendermint-mode court bundle;
- asserts the static and Tendermint block hashes agree (both
  engines sign the same canonical block);
- asserts the static and Tendermint signatures differ (they
  use different signature domains);
- asserts the bundle verifier passes with 22/22 checks;
- asserts the verifier emits `tendermint-certificate` and the
  data-binding checks (`block-hash-recomputes`,
  `evidence-block-hash-matches-canonical-block`);
- asserts the verifier does NOT emit `committee-certificate`
  (Tendermint is the active finality path).
```

## 7. The reproducible JSON report

The Teeworlds reproducibility report is generated locally by the acceptance
gate. It is intentionally not committed, because it records local checkout
paths and machine-specific evidence:

```text
reports/myelin-teeworlds-repro.json
```

The report shape is exactly what the audit asked for. The current
report is generated by `scripts/myelin_teeworlds_acceptance.sh`
plus the new Tendermint-mode reports. The hardening pass emits the
report from a single run that exercises both consensus modes
against the same Teeworlds workload, then writes the merged JSON
to `reports/myelin-teeworlds-repro.json`.
