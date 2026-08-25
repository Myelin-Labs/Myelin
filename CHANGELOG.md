# Changelog

## Unreleased

No unreleased changes.

## 0.20.0 — 2026-08-25

Myelin 0.20.0 turns the finite-Cell execution kernel into an embeddable,
continuously operated session runtime. Consensus, application execution,
production timing, persistence, transport, funding, and external evidence are
separate trust boundaries. A session binds its selected closed-validator
module and exact validator configuration at genesis.

This remains a pre-production release for controlled sessions and research.
The new modules do not make Myelin a CKB full node, a new L1, or a finished
permissionless L2.

### Continuous session runtime

- Added `myelin-session` with deterministic candidate preparation, exact
  finality verification, atomic head advancement, consensus WAL records,
  transactional outbox delivery, and full recovery audit before writes resume.
- Added `myelin-session-store-rocksdb` with synchronous WAL, optimistic head
  CAS, atomic block/checkpoint/head/outbox transactions, rolling checkpoints,
  bounded recovery pages, and durable network queues.
- Added `myelin-session-runtime` as an optional composition root with declared
  service dependencies, readiness and criticality, panic containment, bounded
  lifecycle calls, reverse shutdown, and a writer health gate.
- Added `myelin-session-network` with recipient-bound signed envelopes,
  closed-peer authorization, ordered per-peer sequences, replay/equivocation
  rejection, mTLS transport, ACK-after-durability, and bounded queues.
- Added `myelin-session-escrow` and `myelin-wallet-auth` for optional finalized
  CKB funding attachment, conserved off-chain balances, evidence-bound exits,
  standard CKB identities, and recoverable authorization signatures.

### Production policy

- Added `myelin-session-producer` with `Instant`, `Interval`, `Open`, `Never`,
  and serialized manual production policies.
- Added bounded reserving batches, availability-started open windows,
  explicitly configured empty interval blocks, and release-on-failure or
  shutdown. Source acknowledgement occurs only after durable head advancement.
- Kept production timing outside execution and finality: the commit port must
  still execute the exact candidate, verify its genesis-bound proof, and commit
  through the session store.

### Genesis-bound finality modules

- Added a closed `ConsensusCatalog` with immutable module descriptors binding
  the consensus kind, proof schema, message schema, WAL schema, capabilities,
  and exact validator or authority configuration.
- Added typed `FinalityProof` encoding and dispatch so committee certificates,
  PoA seals, and Tendermint decisions cannot cross engine boundaries.
- Added complete closed-validator Tendermint rounds with deterministic
  proposer selection, proposal/prevote/precommit phases, lock and valid-round
  rules, nil votes, round changes, and equivocation rejection.
- Added proof-of-authority rotation and height-bound CKB-compatible seals as a
  third operational trust model beside static committee and Tendermint.
- Verified that the same workload retains the same CellTx identities,
  scheduler commitment, and pre/post state roots under all three engines;
  consensus-bound block hashes and proof material remain distinct.

### CKB and CellScript evidence

- Added provider-neutral DA certificates with provider and fault-domain quorum,
  retention commitments, and auditor-signed retrieval probes.
- Added evidence-backed public-testnet CKB key, Cell creation, multisig funding,
  and multisig-v2 spending workflows. The archived multisig-v2 funding and
  spending transactions each reached six observed confirmations and include a
  locally re-verifiable receipt chain.
- Corrected strict `SOURCE_CELL_DEP` handling to expose the ordered expanded
  DepGroup member view used by CKB scripts while preserving the declared
  DepGroup bytes in transaction identity.
- Made context anchors reorganization-safe at their own height and separated
  them from the independently sampled transaction-pool validation tip.
- Updated the attested CellScript process boundary to the pinned 0.22.0 patch
  revision and its exact witness ABI, metadata schemas, and artifact digests.

### Validation and compatibility

- Expanded the production gate across static committee, PoA, Tendermint,
  Session court bundles, DA and settlement packages, public-chain evidence
  checks, CellScript reproduction, and the parent CKB integration devnet.
- Added negative tests for wrong proof kinds, forged signatures, stale roots,
  queue replay/equivocation, recovery corruption, outbox retries, reservation
  races, invalid DA evidence, and CKB receipt mutation.
- Replaced the prior unreleased internal session and report shapes directly.
  No compatibility decoder or dual-format migration path is provided.
- Removed stale pre-release version suffixes from draft OpenStrike identity
  domains; no released identity or stored session format used those labels.

### Known boundaries

- All bundled finality choices use known validator or authority sets.
- Application integrations remain external; the runtime is an embeddable
  composition root rather than a production daemon.
- The archived public-testnet evidence exercises CKB transaction and multisig
  paths, not a complete deployed public-testnet Session/court/DA system.
- The full disputed-chunk court, production external DA, operator key custody,
  public-testnet verifier deployment, and long-running adversarial soak remain
  future work.

## 0.10.0 — 2026-07-29

Myelin 0.10.0 turns the retained finite-Cell runtime into an evidence-bound CKB execution kernel. It hardens transaction identity, conflict scheduling, state transitions, mempool admission, validator authentication, CKB projection, and the Session L2 path. It also removes the vendored CellScript compiler in favor of an attested adapter to one exact upstream toolchain.

Myelin remains an unpublished, pre-production project. This version deliberately removes obsolete internal formats instead of carrying compatibility shims. It must not be described as a finished trustless or permissionless L2.

### Highlights

- Added a fail-closed CKB evidence adapter with explicit `wire-encoded`, `context-resolved`, `consensus-validated`, `scripts-verified`, `node-accepted`, `committed`, and `finalized` stages.
- Resolved every transaction input, code dependency, dep-group member, and header dependency under a stable CKB tip before local verification.
- Added authoritative node validation through `test_tx_pool_accept`, exact-hash submission and observation, local CKB transaction-proof verification, canonical-header checks, reorganization detection, and configurable confirmation depth.
- Exercised valid DA and settlement transitions against the parent CKB 0.207.0 integration devnet, including committed transactions and rejection of tampered payloads and a competing settlement.
- Added `myelin ckb prove`, `myelin ckb observe`, and `myelin ckb verify` commands for producing and independently checking evidence projections.

### CellScript integration

- Removed the vendored CellScript workspace from this repository.
- Added a process adapter that verifies compiler identity, source revision, target profile, metadata schema, artifact digests, and metadata digests before accepting compiler output.
- Pinned CellScript release `v0.22.0` at peeled commit `830b5971237401a74dd7848b200f48b4d2ed79f4`.
- Pinned the compiler's CKB SDK v5.1.0 dependency at commit `1fbf3d4c9b35ef90bdb9e6621a8d26edde6325ce`.
- Added four Myelin-owned CellScript fixtures for DA carrier, settlement carrier, final DA, and final settlement verification.
- Added a reproducible toolchain gate that clones, builds, attests, and exercises the exact locked upstream compiler.

### Execution and state integrity

- Aligned transaction identity with CKB: witnesses affect `wtxid`, but not the raw transaction hash.
- Standardized the CKB wire version and Molecule dep-group representation and removed obsolete decoders and witness bridges.
- Made conflict identities state-resolved. Compiler metadata now supplies access templates rather than trusted final logical keys.
- Preserved READ/READ parallelism while ordering READ/WRITE and WRITE/WRITE access to the same logical key.
- Added physical OutPoint double-spend detection, logical conflict ordering, global barriers for non-parallelizable actions, and transitive failure handling in the CellDAG.
- Enforced a shared transaction-level VM cycle budget across all lock and type script groups.
- Made state application atomic and bound every transition to exact pre-state and post-state roots.

### Mempool and finality

- Made admission and replacement atomic and bound entries to the state root against which they were validated.
- Added deterministic conflict scoring and rejected unsupported multi-parent overlays unless their combined state is proven.
- Replaced placeholder validator authentication with secp256k1 Schnorr signatures.
- Kept static-committee and weighted-precommit finality in separate signature domains while preserving consensus-independent transaction execution and state roots.

### Session and submission evidence

- Required live submissions to pass node pool validation, return the expected raw transaction hash, and be observable as pending, proposed, or committed.
- Separated submission acceptance from publication and finality claims.
- Bound DA manifests, settlement intents, carrier payloads, verifier dependencies, authority evidence, and readiness reports to their exact lineage.
- Added negative tests for tampering, hash drift, under-capacity outputs, missing dependencies, replay, competing settlement, insufficient confirmation depth, and forged readiness evidence.

### Validation

- The root workspace passes formatting, build, Clippy with warnings denied, and the full test suite.
- The exact upstream CellScript v0.22.0 reproduction and fixture gate passes.
- The parent CKB integration devnet passes live admission, commitment, inclusion, stability, configured-depth finality, tamper rejection, and competing-settlement rejection checks.
- The production gate runs the parent CKB devnet path by default. The external Teeworlds workload remains optional because its repository and prebuilt replayer are not vendored.

### Known production blockers

The following work is required before Myelin can make production or permissionless security claims:

1. Deploy and exercise a canonical CKB threshold lock that directly verifies participant signatures on chain. The current devnet funding lock is an integration fixture.
2. Implement the complete disputed-chunk court, including challenge windows, adjudication, timeout behavior, bonds, slashing, and payout economics.
3. Deploy the full path to a public CKB testnet and preserve independently verifiable finalized transaction and header proof artifacts.
4. Integrate durable external data availability with retrieval proofs, redundancy, service guarantees, signed receipts, and failure drills.
5. Bind Session readiness directly to the generic finalized CKB evidence object so that the two evidence surfaces cannot diverge.
6. Add broader differential testing against the parent CKB implementation, fuzz and property testing, and long-running state, mempool, contention, and reorganization soak tests.
7. Prove combined state overlays for safe multi-parent mempool packages instead of conservatively rejecting them.
8. Establish production validator and operator key custody, rotation, recovery, monitoring, and incident-response procedures.

The current `finalized` stage means canonical-chain revalidation plus a configured confirmation depth. It is operational finality evidence, not a claim of absolute CKB irreversibility.
