# Myelin architecture

## Scope

Myelin is an off-chain finite-Cell ledger runtime. It reuses CKB-shaped transactions and CKB-VM verification while adding session scheduling, local state roots, closed-validator finality, DA evidence, and dispute/settlement input packages.

It is not a CKB full node, a source-chain fork, a permissionless consensus system, or a deployed public-network L1 court.

## Continuous chain and application modules

`myelin-session` advances a durable session head only after deterministic execution and verification through its session-owned `FinalityVerifier` port. Session genesis commits the complete `ApplicationProfile` (program, input schema, state codec, time/entropy policies, VM capabilities, resource envelope, court procedure, and handoff policy), the consensus kind, canonical validator/quorum configuration, compiled-in module descriptor, and WAL schema. Startup supplies the expected local application profile and fails if it differs from genesis. Each block commits its session id and an `ExecutionFrame`: contiguous input and logical-time ranges, input root, pre/post state roots, ordered raw txids, cycles, transaction bytes, and input bytes.

Version 0.31 also binds `command_identity_required` in the application profile.
Command-aware frames commit ordered IDs and payload digests. The session rejects
previously committed IDs, RocksDB indexes receipts atomically with the block,
and recovery checks that index against the audited history. `command_status`
supports application-journal reconciliation after commit/ACK failures. Command
authentication and economic interpretation remain application responsibilities.

`myelin-session-store-rocksdb` is the production persistence implementation for this interface. It uses a versioned database schema, synchronous RocksDB WAL, optimistic head CAS, module/config/schema-bound consensus-round WAL, and durable network queues. One transaction commits the block, latest checkpoint, head, outbox, issued handoffs, consumed-handoff markers, and—when declared—the successor genesis. Rolling checkpoints avoid storing a full state snapshot with every historical block. Recovery audits the finalised chain, application/frame links, successor lineage, and latest checkpoint before reopening the writer. Bounded range replay begins at the newest retained checkpoint before a requested range and deterministically reexecutes through its end.

`InspectPort` receives an immutable snapshot plus bounded query bytes and returns a bounded result with a receipt binding the session, height, root, application profile, query, and result. It has no writer or outbox capability. `EvidenceRuntime` advances registered outbox topics by one descriptor-defined stage at a time. Collection can use an external service, but a stage is persisted only after the exact adapter verifier rechecks the evidence locally. Terminal records survive acknowledgement, so a crash between the terminal CAS and outbox acknowledgement resumes without repeating or promoting a stage.

A successor is not an upgrade flag on a live chain. The final predecessor block commits a `SuccessorDeclaration`, seals that head permanently, and atomically creates one target genesis. Exact continuation preserves the complete application/effect policy and state bytes; changed rules require an explicit compatibility declaration and vectors, while schema or seasonal conversion requires a separately identified migration program, inputs and converted snapshot. Replay reruns that policy. All paths preserve the input cursor, logical time and timestamp floor, and the target genesis carries a reverse predecessor reference to the source final root. See [session succession](architecture/session-succession.md). A handoff is different: it transfers an application payload without joining histories. The source block commits the target or intake-policy commitment, expiry, authorization material, and optional minimum evidence stage. The target executor interprets authorization under its genesis-bound application policy, while RocksDB marks consumption in the target block transaction. Concurrent or repeated consumption loses the CAS.

`myelin-session-network` provides a module-neutral transport primitive rather than consensus logic. Every protobuf envelope commits to protocol version, session, consensus module commitment, sender, recipient, per-pair sequence, timestamp, message class, owner-defined format version/type tag, payload hash, payload, and Schnorr signature. The mTLS receiver checks a closed peer set and acknowledges only after durable enqueue. Durable receive receipts keep an old exact retry idempotent after application acknowledgement and process restart; different content at one accepted sequence is equivocation. Outbound messages are retained per recipient until the exact remote message id is acknowledged.

`myelin-session-producer` owns the operational policy for closing candidate transaction windows. It implements immediate transaction-driven production, fixed intervals with operator-controlled empty blocks, availability-started cumulative open windows, disabled automatic production, and serialised manual production. Its strict application-neutral config and reusable schedule let embedded hosts select timing without reimplementing it. Its transaction-source port reserves one bounded batch from the current durable head. A failed finality attempt or orderly shutdown releases uncommitted work; after a durable head advance, source acknowledgement happens in a separate durability domain. Count and byte limits are checked before the fixed candidate reaches a `CandidateCommitter`; that port must still execute, obtain and verify genesis-bound finality, and atomically commit through `myelin-session`. Production timing does not decide VM validity, create finality, or become part of application protocol identity.

`myelin-session-runtime` is the optional embeddable composition root. It adapts the closed `ConsensusCatalog` to the session verifier port, runs receipt-stage evidence adapters, and supervises a declared service dependency graph with readiness, criticality, bounded lifecycle calls, panic containment, reverse shutdown, and a writer gate. It does not turn Myelin into a CKB full node or an official daemon, and it does not provide dynamic consensus loading.

The runtime requires an explicit `BatchAdmission` policy and exposes guarded
prepare/commit methods instead of mutable chain access. `RecoverabilityAdmission`
checks a canonical recovery anchor, re-verifies deployment-supplied evidence,
and limits all advancement beyond independently recoverable state. It runs both
before execution and before commit. This is operational control; consensus-level
DA conditions and L1 exit authority require their own protocol enforcement.
Producer selections and candidates carry their construction head; `prepare_block_at`
rejects stale construction. Ambiguous commit outcomes retain reservations for
reconciliation. See [integration contracts](architecture/session-safety.md).

`myelin-session-escrow` is optional. Funding can attach only when participant authorizations and a finalized `myelin-ckb-adapter` receipt chain verify for the exact CKB open transaction, at the required depth and before the configured start deadline. The off-chain ledger conserves the funded asset and enforces participant debit caps. Native-capacity states reject nonzero participant balances below the exact payout Cell occupied-capacity floor; zero balances produce no payout output, and fee change must also meet its floor. Exit builders recover the latest finalized snapshot and bind it to the engine proof; witness collection, node submission, commitment, and configured-depth finality remain explicit later stages. A prepared exit is not a CKB acceptance or finality claim.

The escrow verifier requires an application binding on every transaction,
including transactions without ledger Cells. Generic conservation and a nonzero
application evidence root do not establish equivalence with another economic
ledger. DEX funding should derive claims from canonical DEX state rather than
independently maintained participant balances.

Application integrations live in external modules. The Veloren adapter in the sibling checkout journals authoritative game events before mutation, commits strict-VM Cell transitions under PoA/committee/Tendermint, reconciles an optional receipt inventory from the conserved ledger, and exposes the exit builders. Its embedded local signer driver is a controlled session deployment, not a distributed or permissionless L2.

## Independent services and actual court scope

The chain/session module already combines deterministic execution, finality and
atomic persistence; it is more than a DA module. A complete CKB application
sidechain still needs its own application/VM profile, writer and consensus model,
data availability and custody/exit protocol. an external compiler's CKB-shaped program
bytes alone do not supply these deployment and economic rules.

The [independent-store handoff reference](architecture/remote-handoff.md) now
verifies external source proofs with separately pinned genesis/consensus/rules,
retains a bounded inbox and commits consumption atomically at the target. Its
scarce test assets use one explicit trusted durable terminal authority to order
consume against cancel. Unknown outcomes retain their lock and original target
candidate. This is asynchronous coordination under named trust assumptions.

The [CKB microgame court](architecture/ckb-microgame.md) deploys a strict CKB-VM
rule verifier and exercises actual devnet opening, commit/finalize/challenge,
settlement, timeout, duplicate/stale exits and reorg handling. It separates
application execution, authority attestation, external evidence retrieval, script
validity and inclusion/depth. This narrow test-asset evidence does not promote
generic court bundles or larger games into a public-network court deployment.

## Format boundaries

Current records use MREC 14, block 6 and RocksDB schema 16. Journal format is v2;
bundle format is v3. Decoders reject older formats rather than upgrading them.
Hash domains are explicit, and independent vectors cover protocol commitments.
There is no separate identity-register gate.
