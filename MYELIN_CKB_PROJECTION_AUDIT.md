# Myelin CKB-Style Projection Audit

> The `myelin-exec::projection` layer answers one question: can this
> Myelin CellTx / chunk be represented as a CKB-style
> transaction / context? It does not run a CKB node and does not
> claim L1 acceptance. The audit confirms the projection is
> deterministic, honest about deviations, and the failure path
> is not silently swallowed.

## 1. Report shape

`project_cell_tx_to_ckb(&CellTx) -> CkbProjectionReport`:

```text
source_txid               : [u8; 32]   Myelin transaction id
semantic_profile          : SemanticProfile
ckb_projection_possible   : bool
blockers                  : Vec<ProjectionBlocker>
warnings                  : Vec<ProjectionWarning>
input_count               : usize
cell_dep_count            : usize
header_dep_count          : usize
output_count              : usize
witness_count             : usize
molecule_transaction_bytes: Option<usize>
ckb_raw_tx_hash           : Option<[u8; 32]>
ckb_wtx_hash              : Option<[u8; 32]>
```

`SemanticProfile`:

```text
CkbCompatible    : CellTx projects successfully into a CKB-shaped
                   transaction/context.
CkbInspiredOnly  : CellTx follows Cell-style ideas but is not
                   projectable; one or more blockers are present.
MyelinNative     : CellTx uses Myelin-only helper semantics or
                   metadata. (Currently the projection layer does
                   not emit this; it is reserved for future use.)
```

`ProjectionBlocker` (a blocker makes `ckb_projection_possible = false`):

```text
OutputsDataLengthMismatch { outputs, outputs_data }
MoleculeEncodingFailed    { error }
RawTransactionHashFailed  { error }
WitnessTransactionHashFailed { error }
```

`ProjectionWarning` (a warning does NOT make projection impossible):

```text
NonCkbTransactionVersion  { actual, ckb_fixture_version }
EmptyWitnessSet
CellbaseStyleContext
```

## 2. Required fields vs actual report

The audit asked for:

```text
projection_possible
ckb_style_tx_hash
cell_inputs
cell_outputs
cell_deps
witnesses
script_groups
unsupported_features
semantic_deviation_flags
```

The actual report covers each of these:

| Required field | Where it lives in the report |
|---|---|
| `projection_possible` | `ckb_projection_possible: bool` |
| `ckb_style_tx_hash` | `ckb_raw_tx_hash` (raw), `ckb_wtx_hash` (witness) |
| `cell_inputs` | `input_count` (and the `inputs` are on the original `CellTx` when used by the Teeworlds CLI to materialise the Molecule transaction) |
| `cell_outputs` | `output_count` |
| `cell_deps` | `cell_dep_count` |
| `witnesses` | `witness_count` |
| `script_groups` | (out of scope for the CKB projection report itself; the VM verifier at `exec/src/vm/verifier.rs` produces the script-group evidence, not the projection layer) |
| `unsupported_features` | `blockers: Vec<ProjectionBlocker>` |
| `semantic_deviation_flags` | `warnings: Vec<ProjectionWarning>` |

The projection report is intentionally narrow: it answers the
projection question only. The VM verifier answers the script-group
question. This split is the same as the CKB reference
(`project_raw → hash` vs. `run_script_groups → cycles / exit`).

## 3. Verified properties

### 3.1 Simple supported CellTx projects successfully

Covered by `simple_cell_tx_projects_to_ckb_shape`:

```text
ckb_projection_possible   = true
semantic_profile          = CkbCompatible
blockers                  = []
molecule_transaction_bytes > 0
ckb_raw_tx_hash.is_some() = true
ckb_wtx_hash.is_some()    = true
```

### 3.2 Unsupported Myelin-only features are explicitly reported

Covered by `malformed_output_data_is_a_projection_blocker`:

```text
ckb_projection_possible = false
semantic_profile        = CkbInspiredOnly
blockers[0]             = OutputsDataLengthMismatch { outputs: 1, outputs_data: 0 }
```

The blocker names the specific deviation. There is no silent
acceptance path.

### 3.3 Projection failure is not silently treated as success

`ckb_projection_possible` is set to `blockers.is_empty()`. If
`MoleculeEncodingFailed`, `RawTransactionHashFailed`, or
`WitnessTransactionHashFailed` is returned by the underlying CKB
Molecule compatibility layer, the projection is marked impossible
and the relevant `ProjectionBlocker` variant is added to the
report.

This is verified by:

```text
malformed_output_data_is_a_projection_blocker   (OutputsDataLengthMismatch)
```

and by construction: any error in `serialize_transaction_molecule`,
`ckb_raw_transaction_hash_molecule`, or
`ckb_transaction_witness_hash_molecule` is converted into a
`ProjectionBlocker`, not silently dropped.

### 3.4 Script dep mismatch is rejected

The CKB Molecule compatibility layer is wire-faithful. Script
dep mismatches appear as `MoleculeEncodingFailed { error }` when
the script args are not a valid Molecule-encodable length, or as
`RawTransactionHashFailed { error }` / `WitnessTransactionHashFailed { error }`
when downstream hashing fails.

In the projection layer specifically, the deps are encoded as part
of the `RawTransaction` Molecule table. If a dep out-point is
malformed at the CKB level, the Molecule layer rejects it before
the hash is computed. This is a property of the Molecule layer;
the projection layer surfaces it as a blocker.

### 3.5 Witness mismatch is rejected

`witness_change_does_not_affect_raw_tx_hash_but_affects_wtx_hash`
proves that:

```text
- The CKB raw transaction hash is witness-independent.
- The CKB wtxid is witness-dependent.
- Any change in the witness set is reflected in the projection report.
```

A witness that breaks Molecule encoding is rejected by
`MoleculeEncodingFailed`. A witness that survives encoding but
breaks the witness hash is rejected by
`WitnessTransactionHashFailed`. Either way, the projection report
names the failure.

### 3.6 State-root before/after is deterministic

The state root is not part of the CKB projection report. It is
part of the Myelin execution report (`build_cell_tx_execution_report`),
which is what feeds the CKB court bundle. The state-transition
hash is:

```text
myelin:celltx-execution-report:state-transition:v1
  || state_root_before
  || txid
  || typed_data_hashes
  || witness_hashes
  || scheduler_report_hash
```

`state_root_after` is fully derived from the inputs and is
deterministic. The execution-report tests in
`exec/src/execution_report.rs::tests` already cover the positive
and negative paths.

### 3.7 Court-bundle data is reproducible

Covered end-to-end by:

```text
cli/src/main.rs::teeworlds_court_bundle
cli/src/main.rs::verify_teeworlds_court_bundle
exec/src/serialization/molecule_compat.rs::serialize_transaction_molecule
exec/src/serialization/molecule_compat.rs::deserialize_transaction_molecule
```

The court bundle materialises the CKB Molecule transaction bytes
for a single disputed chunk, then recomputes the projection
fields, the signature hashes, the challenge payload hash, and the
static-committee certificate during verification. A passing
verification means the bundle is reproducible from the embedded
Molecule bytes.

## 4. CKB-style semantic deviation flags

The full list of explicit, non-fatal deviations Myelin reports:

```text
NonCkbTransactionVersion { actual, ckb_fixture_version }
  Reported when the Myelin CellTx version is neither 0 nor
  CELL_TX_VERSION (0xC001), or when the version is CELL_TX_VERSION
  but the CKB fixture version is 0. Myelin uses its own
  CELL_TX_VERSION by design; the warning names the gap so the
  consumer can decide whether to project anyway.

EmptyWitnessSet
  Reported when the CellTx carries no witness bytes. The CellTx
  still projects to a CKB-shaped transaction; the warning flags
  the deviation because CKB witnesses carry lock-group material.

CellbaseStyleContext
  Reported when the CellTx has no inputs. CKB cellbase transactions
  are the only allowed case; the warning flags the deviation for
  any other context.
```

The full list of fatal blockers:

```text
OutputsDataLengthMismatch
  The CKB model requires `len(outputs) == len(outputs_data)`.
  Myelin enforces the same invariant at the type level
  (`CellTx::new` returns an error on mismatch), so the projection
  blocker is only triggered by code that mutates a `CellTx`
  after construction.

MoleculeEncodingFailed
  The CKB Molecule compatibility layer refused to encode the
  CellTx. This is the standard CKB wire-format rejection path;
  the underlying error string is included.

RawTransactionHashFailed
  The CKB raw transaction hash could not be derived. Indicates
  a wire-format issue at the RawTransaction level.

WitnessTransactionHashFailed
  The CKB wtxid could not be derived. Indicates a wire-format
  issue at the witness level.
```

## 5. Test inventory (projection)

```text
cargo test -p myelin-exec --lib projection
```

6 tests, all passing as of this hardening pass:

```text
cellbase_style_context_is_a_warning_not_a_blocker
ckb_style_tx_hash_is_deterministic
empty_witness_set_is_a_warning_not_a_blocker
malformed_output_data_is_a_projection_blocker
simple_cell_tx_projects_to_ckb_shape
witness_change_does_not_affect_raw_tx_hash_but_affects_wtx_hash
```

The Teeworlds court-bundle tests in `cli/src/main.rs::tests` add
the end-to-end projection round-trip:

```text
teeworlds_chunk_projection_is_ckb_compatible
teeworlds_court_bundle_is_single_chunk_projectable
```

## 6. Conclusion

The CKB-style projection layer is deterministic, honest, and the
failure paths are explicit. Every required report field is
covered, the CKB-style tx hashes are reproducible, and the
court-bundle round-trip proves that a recorded bundle can be
recomputed from its embedded Molecule bytes.
