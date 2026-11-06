# Same-session worker migration

This runbook applies only to a deployment using the explicit
[canonical authority gateway](../architecture/writer-fencing.md) at every write,
signing and economic-effect boundary. An existing local-CAS deployment cannot gain
these guarantees by adding a routing lease or copying its database.

## Preconditions

- Pin exact session genesis, application artifacts, finality configuration and
  `FencingPolicy`, including its external-effect uniqueness domain.
- Keep administrator, finality and certificate keys at their respective trusted
  authority services. Worker backups contain only worker request credentials.
- Maintain one authoritative ownership DB and canonical store; validate their
  durable watermark before servicing requests. Keep their backups together, but
  never activate a cloned authority with the same secrets automatically.
- If sessions share a `CanonicalLedger`, retain each session's separate ownership
  DB and policy with the common canonical data. Open one ledger and explicitly
  share its handle; do not start independently writable copies for claim targets.
- External consumers accept canonical certificates under that pinned policy.
  One authoritative effect ledger owns each economic uniqueness domain.
- Configure the [mTLS service/client](../architecture/authority-network.md) with
  explicit server DNS/CA, client CA membership, pinned certificate policy, finite
  request deadlines and body/concurrency limits. TLS credentials are separate
  from administrator and worker signing authorization. This is an operator
  endpoint whose authorized clients can read canonical application history.

The [database backup API](../architecture/session-bundles.md#consistent-database-backup-and-restore)
captures a live sessionstore and its exact cold archive closure. Restore requires
an independent backup commitment and archive trust anchors, validates the complete
keyspace, and returns a new directory without granting writer authority or running
application replay. It does not copy a separate ownership DB, journal DB or key
store. Preserve those components through the deployment's own coordinated backup
procedure; restoring session data alone cannot satisfy the authority watermark.

## Planned worker move

1. Stop admission on the old worker and drain its producer. Retain original
   commands, reservations, authorizations and unknown outcomes; do not generate
   replacement IDs or a new session.
2. Query canonical receipts for in-flight commands. Record the authority's exact
   current head. Reconcile known commits and durable business results. Unknown
   missing records remain reserved at this stage.
3. Prepare the new worker from an audited canonical checkpoint/history and exact
   program/profile. Worker-local speculative or stale blocks are not canonical.
   Copy only stopped/checkpointed replicas; keep the original journal recoverable.
4. The administrator signs the new worker key, expected epoch and exact authority
   head. Persist the grant through `SessionAuthority::grant` or the authenticated
   `AuthorityClient::grant` RPC. If the head changed,
   re-query and review the new anchor before signing another grant.
5. Confirm the durable new grant. An old-epoch request must fail at the authority.
   A timeout is not confirmation; repeat the identical signed request or query the
   current grant. Do not advance based on a gateway route change alone.
6. Reconcile outstanding claim IDs through each issuer's pinned
   `AuthorityClient::reconcile_claims`, requiring at least its confirmed migration
   epoch. Keep affected reservations and locks pending on Missing, Issued,
   transport failure or unverifiable evidence. Consumed/Retired prohibit refunds;
   Cancelled still needs the source's canonical reclaim command; Reclaimed
   resolves the original refund, without creating another command identity.
7. The new worker verifies the current epoch and its audited construction head,
   then submits requests signed with its own key under that epoch. The authority
   independently executes, signs and commits each accepted request.
8. Deliver canonical outbox certificates through the domain's idempotent effect
   ledger. Historical messages remain deliverable after rotation. Retain ACK and
   business results so a lost client response cannot recreate the operation.

For a journal created with `open_fenced`, retain its exact `FencedSubmission` and
query `reconcile_submission` after the new grant. Apply the signed response with
`reconcile_fenced`: matching receipts recover business results; only missing
commands observed at a strictly newer epoch return to the queue. Rebuild those
original commands at the canonical head and persist a new signed submission before
sending it. Never change their IDs or original authorization. A response for an
older reservation cannot release newly submitted work.

For a remote authority, use `reconcile_remote` to obtain and apply the same signed
evidence. `submit_fenced_remote` durably marks Unknown before sending, and
reconciles even when a repeated commit receives a stale-head rejection. Such a
rejection describes the repeated attempt, not the outcome of the earlier request.
Do not cancel a local reservation because an HTTP/gRPC future or deadline expired:
the authority's admitted execution may still be running and committing.

Unfenced unknown reservations have no admissible epoch binding. They remain
pending; do not call `release_unsubmitted`, edit their database or invent a new
command ID. Likewise, a same-epoch
`Missing` observation is insufficient for release.

## Unplanned loss or partition

Worker lease expiry and clock drift do not transfer ownership. The administrator
may revoke the worker by granting a higher epoch at the authoritative head; the
old request key then cannot obtain a canonical signature or effect certificate.
If the authority itself is unavailable, canonical writes stop. If its old process
or storage cannot be invalidated, do not start a second authority from backup.
Recovery then requires an external monotonic/quorum authority or a verifiable new
configuration/closure, neither supplied by this reference deployment.

## Crash inspection and rollback rejection

On gateway restart, inspect the pending-head/watermark recovery result. Only the
exact retained or pending head is admissible. Preserve original files when a
stale canonical store or ownership record is rejected. Never delete an epoch
record to force startup. Changing rules, state codecs or validator keys is a
successor operation under #14, not a same-session worker move.

## Reproduce local drills

```bash
cargo test --locked -p myelin-session-authority --features test-fixture
```

The lane exercises active independent replicas, a forged replica proof, changed
effect payloads/domains, synchronized takeover races, stale backups and nine real
process exits with two recoveries each. It also starts actual mTLS endpoints and
a TCP proxy that discards server responses or closes connections before sending.
It checks old-worker reconnect, exact original outcomes, request cancellation,
global admission and TLS/size failures. Two more authority child exits occur
before commit intent and after receipt creation; each process is observed stopped
and restarted twice through new network endpoints.

The shared-ledger transfer drill keeps two stale RocksDB worker copies active,
loses a committed target response and reconciles the original command and claim
through the issuer. It checks all lifecycle observations, competing consume/cancel,
old-epoch rejection, close/reopen and exact application replay with conserved assets.
It runs in the same authority suite, without a separate migration gate.

Tests require `openssl` to create short-lived TLS material in temporary directories.
Application/signing fixtures use public fixed TEST keys. These local network
drills establish neither physical multi-host deployment nor WAN latency/quorum HA.
Application deployment and measured workload evidence remain tracked in the P1 plan.
