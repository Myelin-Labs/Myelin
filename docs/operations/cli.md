# CLI reference

`myelin-cli` is the executable surface of the Myelin kernel. It
exposes subcommands that produce and verify reports. This page is
the canonical reference.

## Top-level shape

```text
myelin-cli
├── celltx
├── committee
├── runtime
├── session
├── teeworlds
└── (a few lower-level helpers)
```

Run `myelin-cli --help` for the full subcommand list. Each
subcommand takes its own flags; `--help` after the subcommand name
prints them.

## `celltx` — CellTx reports

| Subcommand | What it does |
| --- | --- |
| `celltx simple-report` | Builds a trivial CellTx, runs it through Myelin, and emits an execution report + projection report. |

Output: `MyelinExecutionReport` + `CkbProjectionReport` for the
trivial CellTx. The default semantic profile is `ckb-compatible`.

## `committee` — finality engine

| Subcommand | What it does |
| --- | --- |
| `committee finalise-demo --config <toml>` | Builds a `MyelinBlock` candidate, signs it with the configured committee, emits a finalised block with a certificate. |

The config file uses the same TOML schema documented in
[Consensus engines](../architecture/consensus.md#static-closed-committee).
The subcommand picks `static-closed-committee` or `tendermint`
based on the `kind` field.

## `runtime` — end-to-end smoke

| Subcommand | What it does |
| --- | --- |
| `runtime smoke` | Exercises the runtime spine: a small CellTx batch through mempool → scheduler → verifier → state. Useful as a single-shot sanity check. |

The smoke command runs both consensus engines and asserts the
session ID, CellTx commitments, scheduler commitment, and state
roots are identical between them — only the certificate shape
differs.

## `session` — the Session L2 path

This is the largest subcommand surface. It maps 1:1 onto the
[lifecycle](../interactions/session-flow.md):

| Subcommand | Phase |
| --- | --- |
| `session open-fixture` | Session open (Phase 2) |
| `session commit-fixture` | Session commit (Phase 3) |
| `session court-bundle` | Court bundle construction (Phase 5) |
| `session verify-court-bundle` | Court bundle verification |
| `session da-manifest` | DA manifest emission (Phase 4) |
| `session verify-da-manifest` | DA manifest verification |
| `session da-anchor-package` | DA anchor package construction |
| `session verify-da-anchor-package` | DA anchor package verification |
| `session submit-da-anchor-package` | L1 submission (dry-run by default) |
| `session settlement-intent` | Settlement intent (Phase 6) |
| `session verify-settlement-intent` | Settlement intent verification |
| `session settlement-package` | Settlement package construction |
| `session verify-settlement-package` | Settlement package verification |
| `session submit-settlement-package` | L1 submission (dry-run by default) |
| `session carrier-submission` | Devnet smoke carrier submission |
| `session verify-submission-context` | Step 2 (context) |
| `session verify-submission-economics` | Step 3 (economics) |
| `session verify-submission-inclusion` | Step 4 (inclusion) |
| `session verify-submission-stability` | Step 5 (stability) |
| `session verify-submission-finality` | Step 6 (finality) |
| `session verify-submission-readiness` | Aggregate readiness |

### Common flags

| Flag | Used by | What it does |
| --- | --- | --- |
| `--consensus <kind>` | `open-fixture`, `commit-fixture` | `static-closed-committee` or `tendermint`. |
| `--out <path>` | (most) | Where to write the report JSON. |
| `--rpc-url <url>` | `submit-*`, `verify-submission-*` | CKB JSON-RPC endpoint. |
| `--dry-run` | `submit-*` | Build the request without sending. |
| `--storage-dir <path>` | `da-manifest`, `verify-da-manifest` | Local DA store location. |
| `--external-da-receipt <path>` | `da-manifest` | Bind a provider-signed receipt. |
| `--operator-custody-policy <path>` | readiness commands | Bind a typed operator custody policy. |
| `--operator-runbook <path>` | readiness commands | Bind a typed operator runbook. |
| `--court-economics-deployment-evidence <path>` | `settlement-package` | Bind checked mainnet court economics. |
| `--threshold-lock-deployment-evidence <path>` | `settlement-package` | Bind checked mainnet threshold-lock deployment. |
| `--chunk-index <n>` | `court-bundle` | Which chunk to dispute. |
| `--min-fee-shannons`, `--min-fee-rate-shannons-per-kb`, `--max-fee-shannons` | `verify-submission-economics` | Fee policy. |
| `--min-status <s>` | `verify-submission-inclusion` | Required CKB tx status. |
| `--min-confirmations <n>` | `verify-submission-finality` | Required confirmation depth. |

## `teeworlds` — the reference workload

| Subcommand | What it does |
| --- | --- |
| `teeworlds inspect` | Inspect a Teeworlds mock transaction, emit per-chunk CellTx reports with projection status. |
| `teeworlds bench` | Run the mock transaction through Myelin `runs` times, emit benchmark JSON with cycles, latency, scheduler overhead, and committee finalisation latency. |
| `teeworlds court-bundle` | Build a court bundle for chunk N of the mock transaction. |
| `teeworlds verify-court-bundle` | Re-verify the court bundle. |
| `teeworlds doctor` | Check whether the cloned Teeworlds repo is ready for fixture generation, CKB replayer build, and Myelin VM probing. |
| `teeworlds build-fixture` | Generate a Teeworlds mock transaction from a tape + map + config, then run it through Myelin. |
| `teeworlds vm-probe` | Construct the Teeworlds witness layout and run the replayer binary as a type-script group through Myelin's CKB-VM verifier. |

For the deepest walk-through, see
[Production gate](production-gate.md) and the project's
`MYELIN_TEEWORLDS_REPRODUCIBILITY.md`.

## Common JSON shape

Every CLI subcommand that produces a report emits JSON with at least:

```text
schema_version
generated_at_ms
inputs { ... }            // what was fed in
outputs { ... }           // what was produced
verification { ... }      // what was checked
readiness { ... }         // what the readiness ladder says
```

The readiness object is the *honest label*:

```json
{
  "readiness": {
    "semantic_profile": "ckb-compatible",
    "ckb_projection_possible": true,
    "l1_da_published": false,
    "l1_court_implemented": false,
    "production_submission_ready": false,
    "end_to_end_production_ready": false,
    "production_blockers": ["external_da_sla_missing", "court_economics_not_deployed"]
  }
}
```

Always read the readiness object. The rest of the report is the
*evidence*; the readiness object is the *claim*.

## Conventions

- **All times are milliseconds since epoch**, unless otherwise
  noted.
- **All hashes are 32 bytes**, hex-encoded with a `0x` prefix.
- **All paths are absolute.** Use `$(pwd)/reports/foo.json`, not
  `reports/foo.json`.
- **The CLI never writes outside the directory you give it.** No
  hidden files, no temp dirs, no log files.

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Success. The report was written and (where applicable) verified. |
| `1` | User error — bad flag, missing file, malformed input. |
| `2` | Verification failure — the report didn't pass its own checks. |
| `3` | L1 RPC error — the chain didn't behave as expected. |
| `4` | Internal error — a kernel panic, an unhandled Result, etc. |

A non-zero exit code is always paired with a written JSON report
that explains the failure mode.

## Where to look next

- [Production gate](production-gate.md) — what runs the gate and
  what each step proves.
- [Local CKB devnet smoke](devnet-smoke.md) — the live chain path.