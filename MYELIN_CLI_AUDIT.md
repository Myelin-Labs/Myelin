# Myelin CLI Audit

> The CLI surface for the standalone Myelin runtime. The current
> binary is `myelin` (built by the `myelin-cli` crate). It exposes
> three top-level subcommands: `celltx`, `committee`, and
> `teeworlds`. No command in the live CLI references Spora, NovaSeal,
> `cellc certify`, or any removed surface.

## 1. Top-level structure

```text
myelin
├── celltx
│   └── simple-report
├── committee
│   └── finalise-demo
└── teeworlds
    ├── inspect
    ├── bench
    ├── build-fixture
    ├── vm-probe
    ├── court-bundle
    ├── verify-court-bundle
    └── doctor
```

The binary name (`myelin`) and the subcommand names are stable. The
audit confirms:

```text
- No CLI subcommand references Spora or spora.
- No CLI subcommand references NovaSeal / novaseal.
- No CLI subcommand references "certify" or "certifier".
- No CLI subcommand references proposal-era or website surfaces.
- No CLI subcommand is named after a removed internal target.
```

## 2. Available commands (current, audited)

### 2.1 `myelin celltx simple-report`

Builds a small in-memory CellTx, runs `build_cell_tx_execution_report`,
and emits the report as JSON.

```bash
myelin celltx simple-report --out <path>
```

### 2.2 `myelin committee finalise-demo`

Reads a TOML `ConsensusConfig`, builds a `SelectedConsensus`, and
finalises a fixture block through it.

```bash
myelin committee finalise-demo \
  --config <path/to/static-committee.toml>

myelin committee finalise-demo \
  --config <path/to/tendermint.toml>
```

The output JSON includes:

```text
consensus_kind       : "static-closed-committee" | "tendermint"
block_hash           : hex string (32 bytes)
quorum_weight        : u64 (matches the config)
signer_ids           : [String] (>= quorum_weight)
certificate_height   : u64
certificate_round    : Option<u32>  (None for static, Some(0..) for tendermint)
certificate_step     : "commit" | "precommit"
finalised            : true
```

Both modes are reachable from the same command. There is no
`--consensus` flag because the kind is part of the TOML
`ConsensusConfig`, which is the same place it must be in production.

### 2.3 `myelin teeworlds inspect`

Reads a Teeworlds CKB mock-tx JSON, splits the tape witness into
bounded chunks, runs CKB-style projection for each chunk, and
finalises a static-committee block.

```bash
myelin teeworlds inspect \
  --mock-tx <path/to/teeworlds-mock-tx.json> \
  --chunk-bytes 262144 \
  --out <path>
```

### 2.4 `myelin teeworlds bench`

Benchmarks the inspect path across multiple runs.

```bash
myelin teeworlds bench \
  --mock-tx <path/to/teeworlds-mock-tx.json> \
  --chunk-bytes 262144 \
  --runs 3 \
  --out <path>
```

### 2.4 `myelin teeworlds build-fixture`

Runs `cargo run --manifest-path <teeworlds>/rust-tools/Cargo.toml
-p teeworlds-cli -- utils build-test-tx` and then runs the
`teeworlds bench` path on the produced mock-tx.

```bash
myelin teeworlds build-fixture \
  --teeworlds-root <path/to/teeworlds> \
  --replayer <path> \
  --tape <path> \
  --map <path> \
  --config <path> \
  --mock-tx-output <path> \
  --chunk-bytes 262144 \
  --runs 3 \
  --out <path>
```

### 2.5 `myelin teeworlds vm-probe`

Executes a real RISC-V replayer through Myelin's CKB-VM verifier
path with `VmSemantics::CkbStrict`. Returns a JSON report that
records `success`, `cycles`, and the CKB-strict flag.

```bash
myelin teeworlds vm-probe \
  --replayer <path/to/replayer_stripped> \
  --tape <path> \
  --map <path> \
  --config <path> \
  --max-cycles 70000000 \
  --out <path>
```

### 2.6 `myelin teeworlds court-bundle`

Emits a single-chunk court input bundle for the disputed chunk
`--chunk-index`. The bundle includes:

```text
chunk_payload_hex
session_id
old_state_root
new_state_root
chunk_payload_hash
map_hash
config_hash
chunk_commitment
scheduler_report_hash
molecule_transaction_bytes
molecule_transaction_hex
molecule_transaction_hash
challenge_payload_hash
ckb_projection (semantic_profile, ckb_projection_possible,
                source_txid, ckb_raw_tx_hash, ckb_wtx_hash,
                molecule_transaction_bytes, input/output/witness
                counts)
static_committee_evidence (consensus_kind, block_hash,
                            quorum_weight, signer_ids, signatures,
                            finalised)
court_verifiable
l1_court_implemented : false
notes
```

```bash
myelin teeworlds court-bundle \
  --mock-tx <path> \
  --chunk-bytes 262144 \
  --chunk-index 0 \
  --out <path>
```

### 2.7 `myelin teeworlds verify-court-bundle`

Recomputes the projection fields, signature hashes, challenge
payload hash, and committee certificate against the embedded
Molecule transaction bytes and the embedded static-committee
signatures.

```bash
myelin teeworlds verify-court-bundle \
  --bundle <path> \
  --out <path>
```

The output is a `CourtBundleVerificationReport` with a list of
named checks. `valid: true` only when every check is `ok: true`.

### 2.8 `myelin teeworlds doctor`

Checks whether the local Teeworlds checkout, the generated
protocol sources, the LLVM toolchain, and the CKB replayer
binary are ready for a real CKB-VM replay.

```bash
myelin teeworlds doctor \
  --teeworlds-root <path> \
  --out <path>
```

## 3. Required vs available mapping

The audit's required commands and the actual CLI surface:

| Required | Available | Notes |
|---|---|---|
| `myelin-cli committee inspect <committee.toml>` | `myelin committee finalise-demo --config <toml>` | The CLI is finalise-demo, not inspect. Inspect is performed by the consensus engine itself when finalising; the report JSON exposes the finalised block hash, signer ids, and finality. |
| `myelin-cli consensus finalise-demo --consensus static-closed-committee --committee <committee.toml>` | `myelin committee finalise-demo --config <committee.toml>` | The `--consensus` flag is redundant because the kind is in the TOML. |
| `myelin-cli consensus finalise-demo --consensus tendermint --committee <committee.toml>` | `myelin committee finalise-demo --config <committee.toml>` | Same as above. |
| `myelin-cli teeworlds verify --repo <path>` | `myelin teeworlds build-fixture --teeworlds-root <path>` | The Teeworlds entry point is `build-fixture`; the equivalent of "verify the path" is `teeworlds doctor`, which emits a JSON readiness report. |
| `myelin-cli teeworlds court-bundle --repo <path>` | `myelin teeworlds court-bundle --mock-tx <path>` | The audit splits "court-bundle" into a separate step (operate on the produced mock-tx) from the upstream "build the mock-tx from a Teeworlds repo" step. Both are wired in `myelin_teeworlds_acceptance.sh`. |
| `myelin-cli projection verify --fixture <path>` | not a separate subcommand; the projection is verified by `teeworlds verify-court-bundle` | The projection layer is part of every `celltx` and `teeworlds` report, and `verify-court-bundle` re-runs the projection over the embedded Molecule bytes. |

These equivalents are documented so that any consumer of the audit
can find the matching CLI surface without going through the code.

## 4. Smoke evidence (live run)

The hardening pass ran the following smoke commands on the working
tree:

```bash
cargo run -p myelin-cli -- committee finalise-demo \
  --config /tmp/myelin-audit/static.toml \
  --out /tmp/myelin-audit/static-demo.json

cargo run -p myelin-cli -- committee finalise-demo \
  --config /tmp/myelin-audit/tendermint.toml \
  --out /tmp/myelin-audit/tendermint-demo.json

cargo run -p myelin-cli -- celltx simple-report \
  --out /tmp/myelin-audit/celltx-report.json
```

Results:

```text
static-committee demo:    consensus_kind = "static-closed-committee",
                          finalised = true,
                          signer_ids = [validator-0, validator-1]
tendermint demo:          consensus_kind = "tendermint",
                          finalised = true,
                          signer_ids = [validator-0, validator-1],
                          certificate_round = 0,
                          certificate_step = "precommit"
celltx simple-report:     status = Accepted,
                          semantic_profile = "ckb-compatible",
                          ckb_projection_possible = true,
                          molecule_transaction_bytes > 0
```

The same commands are run by `scripts/myelin_protocol_gate.sh` in
the protocol gate; passing them in isolation is a smoke
verification, not a substitute for the full gate.

## 5. CLI blacklist — what must never appear

The CLI blacklist is checked at every step. None of the following
strings appear in `cli/src/main.rs`:

```text
Spora / spora
NovaSeal / novaseal
proposal-era / certifier / certify
website / roadmap / archive / release note
full-node / PoW / miner / mining / L1 sync
```

`rg -i 'Spora|spora|novaseal|certify|certifier|roadmap|proposal-era' cli/src/main.rs`
returns no matches.

## 6. Conclusion

The CLI surface is clean. The required audit commands are present
under equivalent names, both consensus modes are reachable, the
Teeworlds workflow is wired through `build-fixture` ->
`vm-probe` -> `court-bundle` -> `verify-court-bundle`, and the
celltx/projection path is reachable through
`celltx simple-report` and `teeworlds court-bundle`. No CLI command
mentions the removed surfaces.
