# Glossary

Terms used across the Myelin documentation. Cross-references go to
the page where the term is defined in detail.

## A

**Anchor package** — A CKB-compatible CellTx package that binds a
DA manifest to an L1-anchored cell. See
[DA flow](../interactions/da-flow.md).

**Asset custody** — The CKB Cells that participants lock into a
Myelin session at open. See
[Session lifecycle](../interactions/session-flow.md).

## B

**Block hash** — The canonical hash over the Molecule-serialised
`MyelinBlock` header plus all commitments. Stable for the same
input; sensitive to any field mutation. See
[Consensus engines](../architecture/consensus.md).

**Boundary (claim)** — Where an evidence path stops being a claim.
See [Claim ladder](../security/claim-ladder.md).

## C

**Cell** — A unit of CKB state, containing capacity, data, a lock
script, and an optional type script. See
[What is CKB?](../concepts/what-is-ckb.md).

**CellDAG** — The static conflict graph the scheduler builds from
input OutPoints, consumed Cells, created Cells, read references,
typed-cell conflict hashes, and declared read/write domains. See
[CellDAG scheduler](../architecture/scheduler.md).

**CellScript** — The language Myelin's application code is written
in. Vendored under `cellscript/` with an added `typed-cell` profile.
See [CellScript & typed-cell metadata](../architecture/cellscript.md).

**CellTx** — A unit of state transition: consumes Cells, creates
Cells, carries witnesses and dep references. See
[Anatomy of a Myelin CellTx](../getting-started/anatomy.md).

**CKB** — The Nervos CKB proof-of-work layer-1 blockchain. See
[What is CKB?](../concepts/what-is-ckb.md).

**CKB-VM** — The RISC-V-based virtual machine that runs every CKB
script. See [What is CKB-VM?](../concepts/what-is-ckb-vm.md).

**`ckb-compatible`** — A semantic profile meaning a CellTx is
projectable into a CKB-style transaction without changing
semantics. See [Semantic profiles](../concepts/semantic-profiles.md).

**`ckb-inspired-only`** — A semantic profile meaning a CellTx
follows the Cell model but has unsupported projection flags. See
[Semantic profiles](../concepts/semantic-profiles.md).

**Claim ladder** — The four-tier ladder from "designed to stay
close to CKB semantics" up to "CKB-aligned adjudication path." See
[Claim ladder](../security/claim-ladder.md).

**Closed committee** — A configured set of validators with a known
public-key set. Used by the `StaticClosedCommittee` consensus
engine. See [Consensus engines](../architecture/consensus.md).

**Committee certificate** — The signature set that finalises a
`MyelinBlock`. Quorum-weighted for the static engine; strict-majority
precommit for the Tendermint engine. See
[Consensus engines](../architecture/consensus.md).

**Conflict domain** — A typed grouping of Cells over which two
CellTxs would conflict if both wrote to the same element. See
[CellDAG scheduler](../architecture/scheduler.md).

**Conflict hash** — A typed-cell metadata field that commits to
`(read_set, write_set, conflict_domains)`. See
[CellScript & typed-cell metadata](../architecture/cellscript.md).

**Court bundle** — A self-contained input to the future CKB court
verifier: chunk payload bytes, CKB Molecule tx bytes, projection
report, challenge payload hash, and committee certificate. See
[Court path](../interactions/court-path.md).

**Court verifier** — A future CKB type script that takes a court
bundle as input and emits an accept / slash verdict. **Not yet
implemented.**

## D

**DA** — Data availability. See
[State & data availability](../architecture/state.md) and
[Data availability flow](../interactions/da-flow.md).

**DA manifest** — The artefact that records `payload_hash`,
`segment_root`, `segment_proof`, optional external DA receipt, and
`da_availability` level. See
[Data availability flow](../interactions/da-flow.md).

**Determinism** — The strong property that the same script binary,
with the same input transaction and the same Cell deps, produces
the same exit code on every node. See
[What is CKB-VM?](../concepts/what-is-ckb-vm.md).

**Devnet smoke** — The local CKB devnet smoke script
(`scripts/myelin_ckb_devnet_smoke.sh`) that runs live carrier
submissions against a parent CKB devnet. See
[Local CKB devnet smoke](../operations/devnet-smoke.md).

## E

**End-to-end production readiness** — The aggregate flag that says
"all six production gates pass." Stays `false` until DA is
published, court economics are deployed, threshold-lock is
enforced, the external DA SLA is present, and the operator policy
is typed and verified. See
[L1 submission flow](../interactions/submission-flow.md).

**Evidence path** — A chain of artefacts that together prove a
specific claim. Myelin has four: execution, projection, court,
settlement. See [Evidence paths](../security/evidence-paths.md).

**Execution report** — The `MyelinExecutionReport` that records the
VM's verdict on a CellTx: cycles, exit code, state-root transition,
semantic profile. See
[Execution pipeline](../architecture/exec-pipeline.md).

## F

**Fee density** — The primary ordering key in the scheduler:
`fee / max(declared_cycles, 1)`. See
[CellDAG scheduler](../architecture/scheduler.md).

**Finalised block** — A `MyelinBlock` plus its committee
certificate. The output of `finalise_block`. See
[Consensus engines](../architecture/consensus.md).

## G

**Gate** — A check that the runtime produces a specific report
with a specific flag. The production gate runs nine gates in
sequence. See [Production gate](../operations/production-gate.md).

## L

**L1 / L2 / off-chain** — The three layers Myelin bridges. L1 is CKB;
L2 is Myelin's session runtime; off-chain is producer / witnesses /
DA. See
[The three-layer model](../interactions/l1-l2-offchain.md).

**Live Cell** — A Cell currently spendable inside a Myelin session.
See [State & data availability](../architecture/state.md).

**Lock script** — The script that controls who can spend a Cell.
Runs when the Cell is consumed. See
[What is CKB?](../concepts/what-is-ckb.md).

## M

**Mempool** — The `myelin-mempool` queue that CellTxs sit in
between submission and admission. See
[Mempool & admission](../architecture/mempool.md).

**Molecule** — CKB's deterministic, zero-copy binary serialization
format. Used by Myelin throughout. See
[What is CKB?](../concepts/what-is-ckb.md).

**MyelinBlock** — The block shape that the committee finalises. See
[Consensus engines](../architecture/consensus.md).

**Myelin-only syscall** — A syscall that exists in Myelin's VM but
has no CKB equivalent. CellTxs that use them get
`semantic_profile = "myelin-native"`. See
[Semantic profiles](../concepts/semantic-profiles.md).

## O

**Off-chain** — The layer where producers, witnesses, and DA
storage live. See
[The three-layer model](../interactions/l1-l2-offchain.md).

**OutPoint** — The reference to a Cell by `(tx_id, output_index)`.

## P

**Parallel batch** — A list of CellTxs that the scheduler
guarantees are safe to execute together. See
[CellDAG scheduler](../architecture/scheduler.md).

**Producer** — The off-chain actor that submits CellTxs and
witnesses to Myelin.

**Production gate** — The `scripts/myelin_production_gate.sh` script
that runs the full local release gate. See
[Production gate](../operations/production-gate.md).

**Projection** — The function from a Myelin CellTx to a
`CkbProjectionReport`. See
[CKB-style projection](../architecture/projection.md).

## R

**RBF** — Replace-by-fee. A higher-fee CellTx can replace a
lower-fee conflicting one in the mempool. See
[Mempool & admission](../architecture/mempool.md).

**Readiness chain** — The five-step sequence (context, economics,
inclusion, stability, finality) that proves a CellTx was actually
accepted by CKB. See
[L1 submission flow](../interactions/submission-flow.md).

## S

**Scheduler** — The `myelin-exec` component that builds the
CellDAG and emits parallel batches. See
[CellDAG scheduler](../architecture/scheduler.md).

**Script group** — A set of inputs and outputs that share the same
`type` script. Verified in parallel by the CKB-VM verifier. See
[Execution pipeline](../architecture/exec-pipeline.md).

**SegmentProof** — A Merkle proof that a payload hash is included
in a segment tree. See
[State & data availability](../architecture/state.md).

**Semantic profile** — The label that says what a transition means:
`ckb-compatible`, `myelin-native`, or `ckb-inspired-only`. See
[Semantic profiles](../concepts/semantic-profiles.md).

**Session** — The bounded context in which off-chain Cell execution
happens. Has an open, a sequence of finalised blocks, an optional
dispute, and a close. See
[Session lifecycle](../interactions/session-flow.md).

**Settlement intent** — The off-chain artefact that binds a
verified court bundle and verified DA manifest to a disputed-close
decision. See
[Court path](../interactions/court-path.md).

**Settlement package** — A CKB-compatible CellTx package that
encodes the settlement intent. See
[L1 submission flow](../interactions/submission-flow.md).

**State root** — The 32-byte commitment to the live Cell set. See
[State & data availability](../architecture/state.md).

**Static closed committee** — A consensus engine with a configured
validator set and a quorum-weight finality rule. See
[Consensus engines](../architecture/consensus.md).

**Syscall** — A VM-level call that scripts use to inspect the
transaction they're validating. See
[What is CKB-VM?](../concepts/what-is-ckb-vm.md).

## T

**Tendermint** — A consensus engine with weighted precommit
finality. Same trait as the static engine; different certificate
shape. See [Consensus engines](../architecture/consensus.md).

**Threshold-lock** — A canonical lock-args scheme enforced by the
final settlement verifier. Requires an authority Cell with declared
threshold-lock args. See
[Local CKB devnet smoke](../operations/devnet-smoke.md).

**Type script** — The script that enforces state rules across a set
of Cells sharing a type. Runs at commit time for every output with
a matching `type`. See
[What is CKB?](../concepts/what-is-ckb.md).

**Typed-cell metadata** — The compiler-emitted artefact that
commits to a CellTx's read/write sets, scheduler witness, and
proof obligations. See
[CellScript & typed-cell metadata](../architecture/cellscript.md).

## V

**VM profile** — The VM profile that a chunk was verified under
(e.g. `ckb-strict-basic`). Reports which syscall surface the chunk
used. See [Execution pipeline](../architecture/exec-pipeline.md).

## W

**Witness** — Off-chain provided data attached to a CellTx
(signatures, arguments, game tape, etc.). See
[Anatomy of a Myelin CellTx](../getting-started/anatomy.md).

**Witness slots** — The numbered slots in a CKB transaction that
the script reads. Teeworlds uses slots 1 (tape), 2 (map), 3
(config). See [Execution pipeline](../architecture/exec-pipeline.md).