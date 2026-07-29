# Myelin CKB semantic deviations

This register lists deliberate differences between the current Myelin protocol boundary and upstream CKB. Wire-aligned behavior is listed separately so it is not mislabeled as a deviation.

## Deliberate deviations

| ID | Deviation | Code boundary | Reason and consequence |
| --- | --- | --- | --- |
| D-01 | `Script::hash_v1` is a Myelin-local BLAKE3/domain-separated identity, not the CKB script hash. | `exec/src/celltx/types.rs` | Local registries use a stable Myelin identity. CKB projection uses the CKB Molecule script hash instead. These hashes must never be substituted for one another. |
| D-02 | `VmSemantics::MyelinExtended` exposes Myelin-only helper behavior. | `exec/src/vm/` | It exists for non-court workloads. Session and court paths force `CkbStrict` and record `vm_profile = "ckb-strict-basic"`; extended execution cannot support a CKB script-verification claim. |
| D-03 | `SchedulerPlan` and `CellDAG` are trusted off-chain sidecars, not CKB transaction fields or witnesses. | `exec/src/scheduler/`, `cellscript-adapter/`, `state/src/conflict.rs` | CKB validates transaction conflicts through consumed Cells and scripts; Myelin additionally schedules logical application domains. The plan is bound to the raw txid and concrete conflict hashes, and its commitment is carried only in Myelin evidence. |
| D-04 | Typed conflict hashes and typed-data hashes use Myelin BLAKE3 domains. | `exec/src/celltx/types.rs`, `state/src/conflict.rs` | These are scheduler/evidence commitments, not CKB consensus hashes. A conflict key may be Cell ID, one schema field, or a canonical composite, but it must be resolved from authenticated Cell state. |
| D-05 | `MyelinBlock` contains `consensus_kind`, state roots, ordered raw-tx commitments, DA commitments, and a scheduler commitment. | `consensus/src/lib.rs` | It is a finite-session block, not a CKB header. Its canonical hash binds execution and finality evidence at the Myelin layer. |
| D-06 | Finality is a configured closed validator/authority set: static committee, rotating PoA, or finite-session Tendermint. | `consensus/src/lib.rs`, `consensus/src/proof_of_authority.rs`, `consensus/src/tendermint.rs` | This supports benchmarks and controlled sessions. It is not Nakamoto consensus or permissionless L2 security. PoA binds one scheduled authority to each height; Tendermint implements proposal/prevote/precommit, locks, nil/round changes and equivocation rejection. Networking and permissionless membership remain outside Myelin. All engines finalise the same execution/state transition under distinct consensus and signature domains. |
| D-07 | State roots use Myelin's live/consumed/created Cell-state commitment rather than CKB's global state model. | `state/` | The root is local session evidence. Atomic transitions require the exact pre-root and produce an authenticated post-root; it is not a CKB header field. |
| D-08 | The mempool uses deterministic package fee density, unlockability, conflict domains, and explicit base-root binding. | `mempool/` | Mempool policy is local and never part of a CKB projection claim. Multi-parent packages are rejected until a combined-overlay proof exists. |
| D-09 | DA manifests, provider-neutral DA certificates, court bundles, settlement packages, and submission/readiness reports are Myelin host artifacts. | `cli/src/main.rs`, `state/src/store/segment.rs`, `state/src/da.rs` | They are hash-bound input/evidence shapes. A DA certificate proves only that its configured provider/fault-domain/retention/probe policy verifies; it does not prove an L1 anchor. A dry run proves only request construction; RPC admission proves validation and observation but not commitment; commitment/finality requires separate chain evidence. None alone proves an on-chain court verdict. |
| D-10 | The externally versioned CellScript compiler is connected through an attested process adapter. | `cellscript-adapter/` | Compiler metadata is untrusted until the binary/source/artifact/metadata hashes and pinned version are verified. Binding names are diagnostic only; state-side resolution creates the scheduler domains. |
| D-11 | Higher-stage CKB projection evidence is obtained through an authoritative-node adapter rather than by embedding a CKB full node. | `ckb-adapter/` | Myelin commits the resolved context and node/rule identity, runs strict local VM verification over that context, and locally verifies inclusion proofs. Contextual consensus authority still comes from the selected CKB node; node trust, endpoint policy, and chain selection remain explicit operational inputs. |

## CKB-aligned behavior that is not a deviation

- New `CellTx` values use transaction version `0`.
- Producer `OutPoint`s and ordered transaction commitments use the raw transaction hash; witnesses affect only the witness-inclusive hash.
- Inputs, outputs, output data, cell deps, header deps, scripts, and witnesses use the CKB Molecule-shaped transaction boundary.
- Header dependencies are 32-byte header hashes, matching CKB.
- DepGroup data accepts only the CKB Molecule `OutPointVec`; the historical Myelin encoding was removed.
- `CellInput.since`, output occupied-capacity checks, `DepType::Code`, and `DepType::DepGroup` follow the CKB-shaped model.
- Strict script verification resolves lock/type groups, declared code deps and DepGroups, and applies one shared transaction cycle budget.

## Enforcement rules

1. Pure projection begins at `Rejected` or `WireEncoded`; higher stages require a linked `CkbEvidenceProjection` receipt chain and cannot be asserted with booleans.
2. `SchedulerPlan` data never changes CKB raw transaction bytes or identity.
3. Compiler access metadata never supplies a final conflict key.
4. Session/court commands reject non-strict VM profiles.
5. Every higher projection stage must have canonical commitments and negative tests for forged, mismatched, stale, reordered, or reorged evidence.

See `MYELIN_CKB_PROJECTION_AUDIT.md` for the current stage model.
