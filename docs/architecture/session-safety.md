# Session safety integration contracts

Myelin 0.31.0 adds three reusable boundaries for continuous applications:
construction snapshots, durable command reconciliation, and per-batch operational
admission. These complement existing strict execution, finality verification and
atomic head CAS. They do not implement a DEX or make session claims enforceable
against arbitrary CKB custody scripts.

## Construct against one exact head

`CandidateCommitter::current_head` supplies a `SessionHead` before reservation.
`TransactionSource::reserve` receives that head and must build or validate its
selection against it. The returned selection retains the same head. An `Open`
window cannot combine reservations from different heads.

`ProductionCandidate.base_head` preserves the construction snapshot through
handoff. Explicit manual submission also requires the caller's construction head.
Inside the session writer lock, the committer must check `validate_base` and use
`RuntimeHost::prepare_block_at`. The host rejects a changed head rather than
rebasing a candidate. Final commit retains the existing prepared-head check and
durable store CAS.

The bundled FIFO stores each transaction with its construction head. Stale
entries are rejected without consuming a valid prefix. It is an in-memory helper,
not a durable command journal or automatic transaction rebuilder. Applications
with changing snapshots should implement a journal-backed source that rebuilds
transactions from the original signed commands.

## Commit and reconcile stable commands

Set `ApplicationProfile.command_identity_required = true` at genesis for an
application that requires command-level replay protection. Each non-empty frame
then supplies one `CommandIdentity` per input position. An identity contains an
application-scoped ID and a digest of the original command. IDs and digests must
be nonzero; duplicate IDs within a frame are rejected. The IDs and digests are
committed by the frame input root. Tape-based applications can explicitly disable
this requirement in their genesis profile.

The application must authenticate each ID and digest against the payload and
authorized action. Myelin does not parse arbitrary signed commands or prove that
two different IDs represent different economic intentions. Authorization and
payload interpretation remain in application execution rules. A receipt proves
frame inclusion; it does not by itself mean an order filled or an L1 payout
occurred.

RocksDB writes command receipts in the same transaction as the block, checkpoint,
head and outbox. A command identity cannot be committed twice in one session,
even with a different payload digest or reconstructed transaction. Recovery
checks the receipt index against the audited block history. Successor sessions
have their own ID namespace; applications must bind command signatures to the
session and specify any command transfer across a successor.

After a crash, query `SessionChain::command_status` on the recovered chain:

| Result | Journal action |
| --- | --- |
| `Committed(receipt)` | Reconcile the recorded frame result; do not execute the command again |
| `IdentityConflict(receipt)` | Reject the changed payload and investigate the mismatch |
| `Missing` | Not committed at this audited head; rebuild only after resolving any in-flight or uncertain writer |

`SessionStore::load_command_receipt` is the underlying indexed lookup. It is not
a replacement for recovery audit or a remote cryptographic inclusion verifier.
Custom stores must provide atomic persistence, uniqueness and an explicit receipt
lookup. The interface has no full-history scan fallback.

Producer commit errors distinguish definite rejection, stale construction, and
an unknown outcome. Definite failures release reservations. `OutcomeUnknown`
stops production and retains reservations for reconciliation. Likewise, an ACK
failure after successful commit returns `AcknowledgeAfterCommit` with the durable
block identity and does not release the work. Never classify a timeout after
submission as a definite rejection without checking the durable result.

## Bounded recovery

`RecoveryAudit::advance` verifies chain finality and durable command/handoff
indexes from genesis. Each call has limits for completed blocks, logical store
reads, returned encoded bytes and elapsed time. `RecoveryCancellation` can stop
the audit between reads and verification operations. The default step allows
10,000 blocks, 100,000 reads, 512 MiB of returned bytes and 30 seconds.

`RecoveryOutcome::Paused` includes a reason, per-step/cumulative usage, the
captured head and the next unverified height. Its owned cursor can resume with
a fresh budget and cancellation token. A partly checked block is rechecked;
only completely audited blocks advance the cursor. Budget exhaustion never
returns `AuditedRecovery` or a writable chain. `SessionChain::audit_recovery`
uses the default budget and preserves a paused cursor in
`SessionError::RecoveryPaused`.

Only `Complete` supplies a checkpoint for `recover_audited`. Attaching an
executor requires the same store instance and unchanged durable head. A changed
head during a pause requires a fresh audit. Cursors are process-local; serialized
progress is informational and cannot skip verification after a restart. This
is a finality/index audit, not application reexecution or a trusted-anchor suffix
audit. Full application replay remains a separate operation.

Variable-sized reads reserve the 128 MiB codec ceiling before calling storage,
then charge the actual returned encoding, including repeated dependency reads.
A step needs that much remaining capacity to attempt such a read. Byte/read
usage measures the logical store API, not native I/O, decoded heap size or RSS.
RocksDB pages normally stop at 16 MiB; one larger record can occupy a page alone,
up to the 128 MiB codec limit. A short nonempty page does not end iteration.
Cancellation and deadlines are cooperative: synchronous storage calls and
finality verifiers cannot be preempted. Deployment-level hard time/RSS limits
still require process isolation.

`SessionChain::start_replay_range` constructs application reexecution without
performing I/O. `RangeReplay::advance` uses the same budget, cancellation,
usage and progress types as recovery. Its restore callback and executor are owned
across pauses. Warm-up blocks count against work budgets even though receipt
resource totals cover only the requested inclusive range. The executor is cloned
for each candidate block; a cancellation or dependency-read limit after execution
discards that candidate, preserving the last completely checked state. Resume
does not restore the original snapshot or restart completed blocks.

| Entry point | Work performed | Checkpoint assumption |
| --- | --- | --- |
| `RecoveryAudit` | Genesis-to-head finality and durable-index audit | Application execution is not checked |
| `replay_range`, or `start_replay_range` with no anchor | Application execution from session genesis, including warm-up | Restored genesis must produce its configured root |
| `start_replay_range` with `ReplayAnchor` | Application execution strictly after the trusted anchor | Caller independently trusts the complete anchor identity |

An explicit anchor binds genesis commitment, height, block hash, state root and
snapshot hash. It must precede the requested range and reference a retained
snapshot with a valid finality proof. Missing bytes fail; no older checkpoint is
substituted. An anchor copied from an untrusted database does not establish trust.
Receipt checkpoint fields distinguish suffix execution from execution starting
at genesis, and participate in the receipt commitment.

`ReplayOutcome::Paused` never contains a successful receipt. Resume it with a
fresh per-call budget/token; only `Complete` returns a receipt and cumulative
usage. The default convenience method preserves the same cursor in
`SessionError::ReplayPaused`. Authority replay exposes the same start/advance
flow outside the writer lock and retains default-budget pauses in
`AuthorityError::ReplayPaused`. A changed durable head requires a new replay.
These cursors remain process-local. Synchronous application restore/execute calls
are cooperative boundaries too, so untrusted code still requires worker isolation;
application-owned I/O is not included in the session store-read counters.

## Recheck admission at both boundaries

`RuntimeHost::create` and `recover` require an explicit `BatchAdmission` policy.
Use `UnrestrictedAdmission` only for a deployment that deliberately has no external
publication prerequisite. Install the intended policy on every restart before
starting services. Policy replacement requires stopping the writer.

The host exposes immutable chain inspection and guarded prepare/commit methods;
it no longer hands out a mutable chain reference. Critical-service health and
admission are checked before execution and again before commit. A denial leaves
the durable head unchanged. A candidate whose evidence becomes invalid during
finality collection cannot be committed through the host.

`RecoverabilityAdmission` is a supplied policy with these checks:

1. The candidate base equals the durable head in the configured canonical store.
2. The evidence names this session and an exact canonical block hash/state root,
   or its exact genesis state.
3. A deployment-owned `RecoveryEvidenceVerifier` accepts the bounded evidence
   for that anchor. This runs again at each check, including after restart.
4. The prospective committed block count cannot exceed the independently
   recoverable count by more than the configured lag allowance.

Every batch counts, including an empty batch or a transfer of already deposited
assets. Post-commit publication needs a nonzero lag allowance; zero-lag admission
requires a separate pre-publication protocol. Genesis recovery also needs verified
evidence. There is no built-in assumption that the operator's disk is independent
data availability.

The source supplies evidence and the verifier checks authenticity, freshness,
retention and sufficient replay/state/witness coverage. A local test verifier or
arbitrary nonempty byte string is not a production DA provider. This release
supplies the integration contract and canonical-history/lag enforcement, not an
independent provider deployment.

This gate is local operational policy. If publication is a consensus validity
condition, bind it in the application protocol and have every validator enforce
it. CKB-authorized settlement progress is a separate boundary; recovery evidence
does not authorize withdrawals. On evidence failure, continue read-only queries
and recovery work while halting writes. Do not silently roll back finalized
ownership or already paid withdrawals.

## Escrow application binding

`EscrowCellVerifier::new` requires an `EscrowApplicationBinding` in addition to
the strict VM delegate. The binding runs on every transaction, even when no
matching escrow ledger Cell is present. It must validate the relationship between
the application action, authorization, and the ledger update. Its rules must also
be enforced by any deployed court path that claims equivalent validation.

The generic escrow checker validates conservation, participant bounds, successor
linkage and a nonzero `application_evidence_root`. It cannot validate the meaning
of that root without application rules. A nonzero hash is not evidence that two
ownership models agree. The required binding makes this responsibility explicit;
it does not turn the existing fixed-participant ledger into an open DEX ledger.

## Current formats

Only MREC 14, block 6 and RocksDB schema 16 are supported. Earlier records and
stores are rejected; there is no schema-marker upgrade or historical decoder.
Schema 13 atomically indexes each retained archival checkpoint by height and
block hash. Lookup seeks one index entry and verifies its referenced block,
instead of scanning and decoding intervening history. Bundle imports reconstruct
the same checkpoint-height index from their content-checked block inventory.
Application-authorized successor conversion remains a current protocol operation,
independent of storage format compatibility.
