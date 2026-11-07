# Authoritative handoff cancellation, refund and retirement

P1 #11 now has a shared-store `Issued -> Consumed` or
`Issued -> Cancelled -> Reclaimed` lifecycle, plus explicit
`Consumed -> Retired` source bookkeeping. The reference application uses an
owner signature for cancellation, retains the source lock until cancellation is
durable, then returns that exact amount in a later block. Expiry never refunds.
Exact, owner-authorized continuations inherit outstanding locks through the
canonical successor chain. This remains a shared-store protocol.

## Durable identity and atomicity

`StoredHandoff.lifecycle` binds the exact issuing block and one `HandoffTerminal`.
Every terminal reference contains session ID, height and canonical block hash.
Consumption is derived from the mandatory lifecycle terminal. No duplicate
consumption marker fields are persisted.

Cancellation, consumption, refund and retirement all `get_for_update` the same RocksDB
handoff key. Their marker, application state, command receipt, finalised block,
checkpoint, head and outbox are written in one synchronous-WAL transaction.
Two candidates may both prepare successfully; only one competing terminal
transition commits. A losing transaction changes no balances and returns a
conflict for reconciliation. Never create a new handoff as an ambiguous retry.

Cancellation permanently rejects later intake even if a prepared target candidate
uses a timestamp earlier than cancellation. Refund requires the exact cancellation
block earlier in the current session or in a sealed predecessor. Cancelling and refunding in the same block,
duplicate actions, and consuming/cancelling the same handoff in one request fail.
A second refund cannot recreate rights; a retry queries the original command
receipt or reconciles the terminal state.

## Application authorization

`BlockRequest.handoff_actions` contains typed `Cancel`, `Reclaim` or `Retire` requests.
Preparation resolves each to its original handoff and, for reclaim, the committed
cancellation reference. The actor must be the original source or its canonical
byte-preserving successor, under an `Application` effect policy. The kernel
verifies every sealing declaration, predecessor, exact target genesis, copied
snapshot and sealed source head. Payload claims alone do not confer ownership.

`TransitionExecutor::execute_block_with_lifecycle` defaults to rejecting nonempty
actions. Empty actions delegate to context execution. Applications that implement
only context execution cannot silently ignore lifecycle operations. Live
execution and replay use the same entry point. The native worker
has a distinct appended operation tag carrying the complete bounded witnesses.

The reference policy verifies the original transfer signature and exact pending
lock. Owner cancellation signs canonical postcard encoding of
`(current_owner_session_genesis, transfer_id, handoff_id, owner)` after the literal domain
`myelin:reference-transfer-cancel:v1`. The transfer ID already binds original
version, nonce, recipient, amount, target, expiry and evidence requirement.
Cancellation does not release the lock or advance the asset version. Later reclaim
removes that lock, returns its amount to the original owner and advances the
version. No arbitrary beneficiary is accepted, and used nonces remain retained.
Any caller may trigger the already-authorized exact refund; no additional owner
signature is needed after cancellation.

Copyable receipts have no source lock to refund. Their cancellation/revocation is
not supported by this reference application. Consumed scarce transfers cannot be
cancelled or refunded. The retirement policy below removes their
nonspendable source backing after verifying the exact consuming block.

## Commitments, budgets and replay

All lifecycle blocks use version 6 and MREC envelope 14. The current record
contains resolved actions, original snapshot size and any typed successor receipt.
The action commitment always uses `myelin:handoff-lifecycle-actions:v2`.

Actions share the genesis-bound consumed-handoff count budget. Their original
payload/authorization bytes are charged together with issued and consumed handoffs,
and cancellation signatures count as canonical frame bytes. Action commitments,
outbox, snapshots, finality proofs and the full extended record retain their
existing aggregate bounds. Count checks precede action iteration; preparation,
commit, imports, recovery and replay all enforce the retained limits.

Recovery cross-checks issuing, consuming, cancelling and refunding references
against exact blocks. A cancelled record remains valid when the live index has
subsequently progressed to reclaimed; its original cancellation must still match.
Missing, reset, partial, mismatched or forged indexes fail recovery. Recovery is
not full owner-authorization replay: full range replay reproduces commands,
signatures, state disposition, events, snapshots and all commitments. Complete
portable dependency/artifact verification remains #13.

## Current storage format

Only MREC envelope 14, session block 6 and RocksDB schema 16 are accepted.
Every config binds resource limits and an explicit protocol-effects policy.
Every block records its original snapshot byte count, and every successor carries
its typed transition receipt. Historical record/store decoders and schema-marker
upgrade APIs have been removed. Older data must remain with its original binary;
the current executable does not import or upgrade it.

## Executed checks

The reference suite covers owner/source rejection, missing cancellation, duplicate
or same-block refund, expiry without refund, backdated/prepared intake after
cancellation, both ordered race outcomes and barrier-synchronized real-store races,
aggregate witness budgets, exact record sizing, encoding downgrade, default-policy
rejection, missing/reset/forged lifecycle indexes, restart and full replay.

Twelve real child exits cover before/after sealing, cancellation and refund for
explicit and policy targets. Each reopens storage and resumes twice without a second right.
The existing fourteen source/target/delivery/ACK crash cases remain enabled. The
native worker executes and replays the real issuance/cancellation/refund policy.
The standalone `shared_store_transfer` example exposes `issue`, `cancel`, `refund`,
`resume`, and `replay` with exact lifecycle references.


## Consumed-lock retirement

`TransferPolicy::bind_config` binds the current command schema
`myelin:reference-transfer-commands:v3` and `MTRD` envelope. All command IDs and
payloads use their v2 domains. Retirement is available for scarce transfers;
copyable receipts have no source lock to retire. There is no profile selector.

The protocol `Retire` action additionally carries the exact consuming session,
height and block hash. Preparation and the atomic store commit check that the
same authoritative handoff index is `Consumed` with that reference, and that
the referenced target block contains the consumption exactly once. Target block
decoding uses the target's own genesis resource/profile configuration. The source
application verifies the original signed payload against every pending-lock
field, subtracts only `locked`, removes that lock and advances the source account
version. It never credits `available`, changes the recipient, or deletes used
nonces or target receipt/nullifiers. Anyone may trigger this already-determined
cleanup; ownership was authorized at issuance. Copyable receipts have no lock
and cannot retire. Expiry neither authorizes nor prevents cleanup after a valid
consumption. The original source or its unsealed canonical continuation can retire the lock.

`Retired` retains the exact consuming block and adds the retiring source block.
The consumed reference remains in `Retired`, preventing a second intake. Duplicate
retirement, refund after consumption, and cancellation after retirement reject.
A caller reconciles the original command receipt/terminal record after an unknown
commit outcome rather than inventing a fresh transfer.

Every lifecycle action vector hashes `(session_id, height, resolved_actions)`
under `myelin:handoff-lifecycle-actions:v2`. Events use
`myelin:reference-transfer-event:v2`, the `MTRF` envelope and topic
`myelin/reference-transfer/v2`. `fixtures/protocol/retirement.json` fixes
independently assembled synthetic commitment and envelope vectors.

Recovery audits both consuming and retiring blocks. Archives require a causal
edge from the retirement block to the consuming target block, and from the
terminal index to both. Removing the source lock does not permit dropping this
history. The archive regression removes the original database and replays both
sessions, then rejects deletion of the mandatory consuming-block edge.

## Inherited obligations

`Continue` preserves the original handoff ID, signed payload, pending lock,
nonces and receipt nullifiers. No duplicate owner index is persisted. Lifecycle
actions carry their actual acting session in terminal references; cancellation
signatures must be renewed for that session. Cancellation committed in any
earlier canonical predecessor can justify a later descendant's exact refund.

Ownership lookup permits at most 32 successor links. Before sealing, the reference
application checks every pending origin and rejects a continuation that would
exceed that bound. Resolving the pending locks permits further continuation.
`create_chain` and `restore_executor` obtain these depths from canonical storage;
an executor constructed without inherited-lineage bindings rejects continuation
with inherited locks. Rule/schema migration requires a separate explicit
obligation policy and is not enabled by the reference application.

Archives require direct edges from inherited lifecycle actions to every sealing
block, as well as issuance, cancellation or target consumption. Outbox deliveries
remain in their original session and retain their IDs and ACK state; sealing
neither reissues nor acknowledges them.

Local retention can compact Retired/Reclaimed handoff payload copies only while
preserving permanent identity and lifecycle markers plus the exact archived
issuing block. It never resets availability, consumption or refund state. Reads
reconstruct the full handoff from that pinned block; missing cold data fails.
See [terminal payload retention](session-bundles.md#terminal-handoff-payload-compaction).
