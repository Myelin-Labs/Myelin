# Durable original-command journal

`myelin-session-journal` implements the local journal foundation for P1 #12.
The journal and session store may be different RocksDB databases. Each journal
transition uses synchronous WAL; there is no transaction spanning both databases.
Unfenced submissions deliberately retain uncertain reservations. Fenced journals
can requeue exact original work after a signed, later-epoch absence observation.
Reusable examples and downstream game integration are tracked under #12/#19.

## Original intent and authorization

An `OriginalCommand` retains a stable `CommandIdentity`, original canonical
application bytes, session/genesis/profile scope, input kind, authentication
context and original authorization. Rebuilding a stale CellTx changes only the
reservation's derived work. It never changes the original identity or signature.
An identical authorized retry returns the original entry and business outcome;
changed command bytes, context, scope or payload digest conflict with that ID.
An alternative valid signature of the same intent does not replace the original.

`CommandAuthenticator` checks the application's actual signature/delegation and
payload-identity rules on admission and again before reconstruction. Mere presence
of authorization bytes is insufficient. The optional shared digest is BLAKE3 of:

```text
literal UTF-8 "myelin:command-journal-authorization:v1"
session_id[32] || genesis_commitment[32] || application_profile[32]
command_id[32] || payload_hash[32] || input_kind:u8
len(canonical_command):u64le || canonical_command
len(authentication_context):u64le || authentication_context
```

Input kinds are user command (0), input tape (1), and system event (2). The
authenticator decides each kind's credentials; the journal does not require a
wallet signature for every simulation sample. This journal requires an explicit
command-identity profile. Applications introducing this command encoding must
bind a new input-schema/program profile, not reinterpret existing tape history.

## Submission protocol

1. Authenticate and persist the original command before accepting it.
2. Reconcile existing receipts, then reserve a bounded ordered prefix at an exact
   construction head. Persist the reservation and derived work before returning.
3. Under the session writer lock, recheck that head. Persist `begin_commit` before
   calling the committer. A journal write error prohibits submission.
4. Obtain the exact session receipt through `reconcile`. Interpret the exact
   committed block to establish a business result. Inclusion alone is not success.
5. Persist ACK before returning a business response. Retries after an ACK or a
   lost response still retrieve the original retained outcome.

`submit` applies the existing `CommitError` contract: `StaleCandidate` requeues
the original work, a definite `Rejected` records admission rejection, and
`OutcomeUnknown` retains the reservation. It must run under the session writer
lock. The callback must never classify an uncertain backend error as rejection.
On success, the reservation remains pending until actual receipt reconciliation.

`release_unsubmitted` accepts only `Held` reservations which have never crossed
`begin_commit`. No force/requeue-on-Missing switch exists. A crashed `Submitting`
reservation is uncertain even when no receipt currently exists. Fencing the
previous writer and proving authoritative absence requires the signed protocol
below; no caller boolean substitutes for this evidence.

Reconstruction preserves a valid prefix when the next entry becomes stale or
rejected. Stale work blocks later queued work until an explicit retry; terminal
rejection is visible and later work may proceed. Unknown reservations block new
reservations. A definitely unsubmitted prefix can be released and rebuilt while
retaining the same economic identities.

## Receipt and response semantics

| Query/status | Journal behavior |
| --- | --- |
| `Committed`, matching ID/payload | Bind the exact durable receipt; decode business result from its committed block |
| `IdentityConflict` | Preserve the conflicting receipt, never return the other payload's business result |
| `Missing` | Preserve current state; never infer permission to resubmit an uncertain reservation |
| Included with no interpreted result | Await interpretation; ACK rejects |
| Business rejection in a committed frame | Preserve a `Rejected` business outcome and its inclusion receipt |
| ACK or HTTP response lost | Retry returns the original outcome without rebuilding or charging again |

`SessionChain::command_record` validates the command receipt before returning its
exact block. It rejects a mismatched payload. Journal reconciliation uses this
boundary, not a caller-supplied success receipt. Business interpretation remains
application-owned and must be deterministic from that record.

## Persistence and bounds

The journal has its own `MJNL` version-2 envelope and BLAKE3 body checksum. The
bounded JSON body retains scope, immutable limits, entries and the sole active
reservation, pinned optional fencing policy and signed recovery evidence. It
rejects unknown fields, bad checksum/version, conflicting scope,
duplicate IDs, reservation/index mismatch and excess count/bytes. A write I/O
error poisons the live handle: its durable outcome may be unknown, so close and
reopen before any further operation. Cached state cannot overwrite that outcome.
Opening storage validates all signatures and structural bindings. During a locked
update, complete recovery evidence equal to the verified in-memory evidence at
the same index does not need another signature verification; its bindings to the
current journal entries are still checked. New or changed evidence is verified.
No verification cache/flag is persisted. An operation that leaves the whole
journal state unchanged returns without another WAL write, after its ordinary
authentication and conflict checks.

The hard format bounds are 16,384 retained entries and 32 MiB encoded bytes;
defaults remain 4,096 entries and 16 MiB encoded state. Fenced recovery evidence
uses the configured entry count as its own count bound. Larger applications must
request an explicit limit; a larger entry budget never overrides the byte budget. Commands, authentication material, built
work and business results have separate bounds. ACK never deletes identity
history. At capacity, admission fails until a separately specified retention
policy is implemented; there is no automatic tombstone eviction.

Only the current v2 journal format is accepted. The v1 decoder, fixture and
upgrade API have been removed. Scope, bounds, checksum and exclusive locking
remain required for current records.

## Fenced submissions and recovery

Create an empty fenced journal with `open_fenced` and the deployment's trusted
`FencingPolicy`. It becomes immutable in that journal. Ordinary `begin_commit` and
replica-based `reconcile` reject on this path; callers cannot bypass canonical
membership with a locally valid PoA receipt or substitute a response's key.

`begin_fenced_commit` persists the exact signed `CommitAuthorization`, its
administrator-signed grant, policy and ordered original command identities. It
verifies the full candidate digest, construction head and exact frame identities
before changing `Held` to `Submitting`. Send only that persisted request. A crash
after persistence remains uncertain, even if no network call actually began.

`SessionAuthority::reconcile_submission` takes one linearizable observation under
the grant/commit lock. Its signed certificate binds the original submission,
submitted and observed epochs, canonical head and complete ordered command
statuses. Unique referenced block records retain exact receipts/inputs/outbox,
with snapshots pruned while preserving their original signed size fields. It
limits the query to 256 identities and 32 MiB of encoded records. Verification
shares decoded blocks rather than cloning one full block for every command.

`reconcile_fenced` verifies that certificate under the pinned key and retained
submission. It rechecks each receipt against its exact committed frame and the
application profile before interpreting business outcomes:

| Observation | Result |
| --- | --- |
| Same-epoch `Missing` | Keep the uncertain reservation |
| Higher-epoch `Missing` | Requeue only those original commands; the submitted epoch is permanently revoked |
| Matching committed receipt | Recover the exact business result; inclusion without interpretation remains unacknowledgeable |
| Committed different payload at the same ID | Retain `IdentityConflict`; never return another command's result |
| Evidence for an older reservation/request | Cannot reset a newer reservation or change a known result |
| Unfenced unknown submission | Reject fenced release; no binding can be manufactured after the fact |

A later current-writer commit can race the observation. If a signed missing
observation is applied after that commit, the retry still uses the same command
ID and payload. Canonical command uniqueness rejects a second execution; a fresh
observation recovers the original receipt. A certificate does not expire with a
lease: its safety relies on irreversible epoch revocation and canonical identity
uniqueness under the declared single-authority trust model.

Recovery evidence is retained atomically with journal transitions, including
idempotent retries after a lost reconciliation/ACK response. Certificate count
is bounded by the configured entry limit, within the aggregate encoded-byte cap.
Capacity exhaustion leaves the prior pending state intact. This is not a generic
proof of absence for an arbitrary database, a rollback-resistant quorum service,
or a portable independent application replay bundle.

This reference backend synchronously rewrites one bounded journal state record.
It is intended to establish correct recovery semantics, not a throughput claim.
Write amplification and retention require measurement and further work under
#20/#18. The directory lock coordinates one local owner; it is not cross-host
writer fencing. Checksums and invariants detect corruption, not Byzantine disk
replacement. Callbacks must not recursively call the journal while holding its
transition mutex.

## Validation

The suite uses a real owner-signed inventory executor, actual PoA finality,
separate journal/session databases, restart and deterministic range replay.
Tests cover same-ID conflicts, forged and retargeted signatures, stale prefixes,
unknown retention, definite rejection, business rejection, receipt conflicts,
prior committed receipts, capacity and checksum failures.

Nine child-process exits cover before/after journal write, after reserve, after
prepare, after durable submission intent, after session commit, after result
reconciliation, after ACK and after writing the simulated HTTP reply. Every
child's termination is observed before opening storage. Each recovery runs twice;
the uncertain pre-commit case remains pending with unchanged funds, while every
committed purchase is charged once and reproduces its original result.

The fenced suite covers bindings, pinned trust, response mutation,
mixed conflict/missing batches and eight synchronized
observation/commit races. Ten additional actual child exits span binding,
rotation, observation, requeue, rebinding, retry commit, result and ACK. Two
recoveries and canonical replays per case retain one purchase and its original
identity. The [authority network adapter](authority-network.md) additionally
exercises response loss, disconnected/stale writers, cancelled RPCs and real
authority process recovery. `submit_fenced_remote` retains Unknown before sending
and reconciles even a rejected retry; `reconcile_remote` recovers after reconnect.
See the [delivery record](../plans/p1-delivery.md) for example, downstream and deployment evidence.
