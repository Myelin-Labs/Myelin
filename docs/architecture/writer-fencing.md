# Authoritative writer fencing

`myelin-session-authority` is an explicit, trusted single-authority reference for
P1 #15. It owns canonical RocksDB storage, a single-key PoA signer and a separate
canonical-certificate key. Workers own only request-signing keys and disposable,
non-authoritative replicas. Routing/discovery Registries do not participate.

This is a deployment trust model, not a permissionless or Byzantine-fault-tolerant
ownership service. A deployment must expose this gateway as its only canonical
write/signing path and pin its complete `FencingPolicy` at consumers. Ordinary
PoA proofs from a replica do not establish canonical membership in this model.
The library includes an embeddable [mTLS authority service](authority-network.md)
and client, with local process/network fault drills. Physical multi-host
deployment remains integration work. It must not be advertised as generic distributed fencing
for an existing deployment that still accepts local-store writes or ordinary PoA
proofs independently of the gateway.

## Authority and identity

`FencingPolicy` binds session ID, exact genesis commitment, administrator key,
certificate key and one external economic-effect uniqueness domain. Its BLAKE3
commitment is the literal `myelin:writer-fencing-policy:v1` followed by those five
32-byte fields, in that order. Role keys must be distinct. The owner of a worker
epoch cannot be the administrator, PoA signer or certificate signer.

`CanonicalLedger::open` owns the writable RocksDB handle. Pass its `Arc` to
`SessionAuthority::open`; there is no path-based compatibility constructor.
Independent sessions may explicitly share that ledger while retaining separate
ownership databases, grants and signing policies. A session ID has only one live
authority within the ledger, even if another caller supplies a different epoch
directory. `genesis` exposes only immutable dependency records; workers never
receive a writable store capability. Application preparation runs under each
session's own lock. Canonical commit/watermark updates and claim observations
serialize through a shared ledger lock, so consume/cancel use one authoritative
claim transaction. A panic while holding the shared lock poisons it; a write
error marks the affected authority uncertain. Reopen and audit instead of
bypassing either condition.

An administrator signs a `GrantRequest` containing the policy commitment,
expected current epoch, new worker public key and complete canonical recovery
head. Granting requires the exact current head and epoch. The new epoch is stored
with synchronous WAL before being returned. An identical latest grant request is
idempotent; an older request cannot undo a subsequent grant. Epoch history is
monotonic and retained; it is not derived from clocks or routing leases.

The signed request and certificate encodings are BLAKE3(domain || canonical
postcard tuple). Integers use postcard unsigned varints; arrays have their fixed
length; byte vectors have postcard length prefixes. All signatures are BIP340
secp256k1 Schnorr. This is distinct from the ADR's proposed generic encoding.

| Domain | Ordered tuple |
| --- | --- |
| `myelin:writer-fence-grant:v1` | policy, expected epoch, owner, encoded anchor |
| `myelin:writer-fence-request:v1` | policy, epoch, owner, encoded construction head, candidate digest |
| `myelin:writer-fence-candidate:v1` | transactions, frame input, scheduler commitment, request data commitments, timestamp, optional encoded successor config, issued intents, consumed IDs, lifecycle actions |
| `myelin:writer-fence-block:v1` | policy, epoch, owner, height, parent hash |
| `myelin:writer-fence-canonical:v1` | policy, epoch, owner, height, block hash, state root |
| `myelin:writer-fence-message:v1` | message ID, consensus module commitment, topic, payload |
| `myelin:writer-fence-effect:v1` | canonical receipt digest, message ID, message commitment |
| `myelin:writer-fence-reconciliation:v1` | policy, submission digest, submitted epoch, observed epoch, encoded canonical head, ordered command observations, unique encoded blocks |
| `myelin:writer-fence-claim-reconciliation:v1` | issuer policy, observed issuer epoch, ordered claim observations, involved genesis configurations and head records, unique encoded blocks |
| `myelin:writer-fence-rpc-request:v1` | exact bounded postcard request body (raw bytes, no additional vector prefix) |
| `myelin:writer-fence-rpc-reply:v1` | RPC request digest, result or rejection |

Hash serialization streams into BLAKE3 with a 64 MiB bound rather than allocating
a second complete candidate. Host `local_now_ms` is not signed worker input: the
authority replaces it with its own supplied clock. Block timestamp remains signed.

## Three enforced boundaries

1. **Canonical storage.** `SessionAuthority` privately owns the store handle.
   It verifies the current epoch, worker signature, exact construction head and
   candidate bytes under its mutex, then executes the candidate independently.
   Replica state roots and locally committed blocks are not imported as truth.
2. **Finality signing.** The same critical section prepares the block, adds its
   fencing commitment, signs with the authority-held PoA key and commits. Neither
   the raw signing function nor its secret is exposed to workers. No finality
   response or canonical certificate is returned before durable commit.
3. **Economic effects.** `CanonicalEffect` binds the exact canonical block and
   outbox message under the separate certificate key. `CanonicalEffectStore`
   verifies the pinned policy/domain, deduplicates permanent message IDs and
   updates its application state in one synchronous write. A local PoA proof,
   changed payload, altered block certificate or another domain is rejected.

The external effect store is itself the authoritative service for its declared
uniqueness domain. Do not run independent restored copies as two accepted economic
ledgers. Its callback mutates only its own bounded state; external HTTP calls or
another database inside the callback are not atomic. Such integrations need their
own authoritative idempotency/outbox boundary. Historical canonical messages remain
valid after worker rotation and are delivered at most once per retained domain.

## Remote claim reconciliation

`reconcile_claims` reads the issuer's shared canonical ledger under the commit
lock and signs a bounded observation under its pinned certificate key. It binds
the exact ordered claim IDs, observed issuer epoch, original typed handoffs,
lifecycle references, involved genesis configurations and head records, and
canonical blocks. The client checks source genesis, record commitments, target policy/expiry, exact
consumption/actions and referenced-block completeness. At most 64 IDs and 32 MiB
of evidence are accepted; duplicate/zero IDs, foreign issuers, unsupported epochs
and malformed evidence reject. Application authorization still requires replay;
a source certificate attests this trusted ledger, not an independent authority's
entire consensus history.

| Observation | Recovery action |
| --- | --- |
| Missing or Issued | Preserve uncertainty/locks; no automatic refund from absence, epoch advance or expiry |
| Consumed | Recover the original target command receipt; reject cancellation and duplicate credit |
| Retired | Consumption remains permanent; the source backing lock has already been retired |
| Cancelled | Consumption is excluded; use the source application's separately committed reclaim command |
| Reclaimed | Recover the original refund receipt; do not issue a new refund |

The query accepts a minimum issuer epoch, so observations preceding a migration
cannot settle its recovery. Fresh network challenges bind each RPC response.
Historical terminal evidence remains useful, but does not grant a new writer or
change a balance. A consume/cancel race can leave the losing authority's exact
pending intent uncertain; reopening reconciles its watermark before further work.
All retained original commands and claim IDs remain unchanged.

## Crash and rollback protocol

The ownership database stores a canonical-head watermark and at most one exact
pending head. Before signing, the gateway persists the candidate intent. After
the session store commits, it durably advances the watermark and clears the
intent. Only then does it return a certificate.

On restart, exclusive locks exclude the old authority process. The gateway
audits session genesis/history/finality, every signed ownership grant and its
recovery anchor, and the epoch commitment of every canonical block. It accepts
only the retained watermark or the one exact pending next head. If the block is
absent, it clears the unsent signing intent; if committed, it advances the
watermark. A stale canonical backup, stale epoch history, missing commitment or
different pending outcome blocks opening. A write I/O error poisons the live
gateway until reopening; uncertain cached state cannot advance another epoch.

This coordination does not claim an atomic transaction across its two databases.
The pending-head record makes the crash gap explicit and recoverable. Rolling
back **both** authoritative databases and reusing their secrets is outside the
model: an external monotonic witness/quorum or verifiable closure is required to
authorize such disaster recovery. The gateway does not attempt automatic authority
takeover from an old backup.

## Failure outcomes and supported scope

| Condition | Outcome |
| --- | --- |
| Worker partition, timeout or discovery lease expiry | No owner change; uncertain work stays pending |
| Clock drift | Cannot change epochs; authority clock still bounds candidate timestamps |
| Administrator grants a newer epoch | Old worker requests are rejected before execution/signing |
| Commit races a grant | One serial order; a grant built at the old head fails if commit won |
| Stale worker backup | Wrong epoch or construction head rejects; audit/refresh from canonical history |
| Grant response lost | Exact signed retry recovers the persisted latest grant |
| Commit response lost | Query canonical command receipt/block; recover the same certificate |
| Authority unavailable | Canonical writes and new certifications stop |
| Old authority cannot be excluded | No automatic takeover; stop writes or use a separately verified closure/configuration |
| Rule/codec/validator change | Successor compatibility protocol required; this gateway rejects successor requests |

The current subset supports one authority-owned PoA key per session, worker
rotation, explicit shared-ledger sessions and remote claim terminal observations.
It does not provide quorum authority HA, validator-key rotation, L1 custody or
terminal consensus between independently owned claim ledgers.
Signed command observations now support the journal's exact-submission/later-epoch
recovery path; see [journal contracts](command-journal.md). The network client
and journal adapter now preserve this contract across transport failures.
The local #15 acceptance mapping and retained drill reports are recorded in the
[P1 delivery record](../plans/p1-delivery.md#requirement-mapping--writer-fencing-and-migration).
Deployment-specific key custody and availability still follow this trust model;
the reference does not supply automatic authority failover.

The shared-ledger network flow uses the actual scarce-transfer executor with two
fenced sessions and two independently open stale worker replicas. It discards a
committed target reply through a TCP proxy, reconciles the original command and
source claim, rejects old-epoch cancellation and source impersonation, and checks
Consumed/Retired/Cancelled/Reclaimed/Issued/Missing outcomes. Competing consumption
and cancellation have one winner. Reopening both ownership stores and the ledger
preserves terminal states and total assets; both original applications replay to
their exact roots. These checks extend the existing authority network suite.

`MFEN` v1 ownership records have a 4 MiB/1,024-epoch hard bound. `MEFX` v1 effect
records retain up to 4,096 IDs and 1 MiB application state, capped at 16 MiB encoded
bytes. Capacity exhaustion rejects further operations; IDs/epochs are not silently
evicted. [Archive retention](session-bundles.md) and
[workload measurements](../operations/workload-measurements.md) describe their
separate boundaries; they do not remove these authority/effect-store caps.
Checksums detect accidental
corruption, not Byzantine replacement of trusted storage.

## Evidence

The tests keep two independent RocksDB worker replicas active, revoke the old
worker, and show that even a deliberately forged local block with the TEST PoA
key cannot create accepted canonical progress or external rights. They check
admin/epoch/anchor/signature mutation, same-epoch canonical rollback, stale epoch
backup, eight synchronized grant-versus-commit races and durable effect dedup.

With `test-fixture`, nine actual child exits exercise grant, intent, signing,
canonical commit, watermark, receipt and effect delivery boundaries. Parent tests
observe termination, reopen twice and replay the canonical purchase. Old epochs
remain revoked, balances debit once and the external effect increments once.
The [network suite](authority-network.md) additionally exercises real mTLS
connections, response loss, partitions, cancelled RPCs and two authority process
exits with two recoveries each. These are local independent-store/process/network
drills, not physical multi-host deployment evidence. See the
[migration runbook](../operations/writer-migration.md).
