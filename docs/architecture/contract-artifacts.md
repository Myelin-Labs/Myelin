# Contract artifacts

Myelin accepts already compiled programs without selecting or running a source
compiler. `CellTxScriptVerifier` verifies a transaction and inputs resolved from
pre-state, returning its consumed cycles. The program, VM semantics, application
configuration and resource limits determine execution. Metadata supplies access
templates; Myelin resolves identities against actual Cells before scheduling.

## CKB artifact handoff

`myelin_ckb_adapter::artifact::ContractArtifact` defines a current-only JSON
manifest. `session carrier-submission --verifier-artifact manifest.json` verifies
the referenced program before live submission or recording external acceptance.
A dry transaction sketch may omit an existing artifact, but cannot count as live
acceptance. An existing invalid manifest fails even for a dry sketch.

| Field | Meaning |
| --- | --- |
| `version` | Exactly `1` |
| `execution_profile` | Exactly `ckb-strict-vm2`; extended execution is a different contract |
| `format` | `riscv64-elf`, little-endian RISC-V ELF64 |
| `artifact` | Program file relative to the manifest directory, without parent components |
| `artifact_sha256` | SHA-256 of the exact program bytes, `0x` plus 64 hex digits |
| `code_hash` | CKB-personalized Blake2b-256 of the same bytes; equals the deployed Data2 script code hash |
| `compiler_identity` | Build producer identity, recorded as provenance |
| `source_sha256` | Optional source digest; `null` is valid for a binary-only handoff |
| `entry` | Application-defined entry name |
| `witness_abi` | Application-defined encoded witness contract |
| `dependencies` | Additional `CellDep` objects required in the consuming transaction |

Dependency objects use the Rust API JSON encoding: `out_point.tx_hash` is a
32-byte integer array, `out_point.index` is a number, and `dep_type` is `Code` or
`DepGroup`. The verifier's own deployment OutPoint is supplied by the existing CLI
arguments. The CLI adds the declared runtime dependencies to the transaction, preserving
its existing lock, evidence and verifier dependencies. Duplicate manifest entries
fail. Live resolution must still find and verify every dependency. It does not
fetch build artifacts or invoke a compiler.

The manifest is bounded to 64 KiB, the program to 4 MiB and dependencies to 128.
Unknown manifest fields and unsupported versions/profiles fail. SHA-256 and code
identity are recomputed from the ELF. Submission still resolves the live code
CellDep and checks its data hash against this same identity.

Build identity, optional source digest, entry and witness ABI are declarations,
not proofs of build reproducibility or application correctness. The application
encodes witnesses and verifies its exact semantics on the VM/node. The generic
consumer has no language-specific ABI parser or compiler allowlist.

## Independent producers

Rust, C or an optional language adapter can produce this manifest. The root CKB
profile builds the [Rust microgame](ckb-microgame.md). The optional compiler adapter
in `integrations/cellscript` has an independent workspace and compiler lock; it
exports this same handoff using the upstream `ckb` target. Core builds, tests and
the CKB court profile work without that directory or compiler.

Myelin application configuration owns session state, logical time, entropy,
execution profile and resource budgets. A language-specific compiler target is
needed only when an application deliberately introduces a distinct execution ABI.
No such target is required for the current strict CKB path.
