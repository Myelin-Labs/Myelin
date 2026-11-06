# Authority network service

`myelin-session-authority::network` exposes the canonical single-authority
gateway over tonic gRPC with mandatory mutual TLS. The application still owns
its executor, genesis, key custody and service lifecycle. It constructs and audits
`SessionAuthority` before binding the service; this is not a dynamic application
loader or automatic authority failover mechanism.

## Host and client contract

The host binds a `tokio::net::TcpListener`, constructs
`AuthorityService::new(Arc<SessionAuthority<E>>, clock, limits)`, and calls
`network::serve_mtls(listener, service, tls, shutdown)`. The TLS configuration uses
the existing `myelin-session-network::MutualTlsServerConfig` and client CA roots.
Use `SystemClock` in an ordinary deployment. Test clocks are explicitly injected;
the worker's `local_now_ms` is never sent or used. The signed block timestamp is
still subject to the authority's future-drift check.

Clients pin both the TLS server CA/DNS identity and the complete `FencingPolicy`.
`AuthorityClient::connect` requires HTTPS, a client certificate, explicit limits
and a nonzero deadline no greater than 300 seconds. It never learns the canonical
certificate key from an untrusted reply. Even if a host accidentally mounts the
generated service on plaintext HTTP, the handler rejects requests without a TLS
peer certificate. Deploy the endpoint directly with mTLS; terminating TLS at a
proxy without an authenticated backend connection is not a supported bypass.

TLS CA membership authorizes access to this operator service, including its
history and command records. It is not a public player endpoint or a private-input
filter. Application signatures separately authorize grants and commits. The
administrator, worker, finality and certificate keys retain their distinct roles.
Issuing a TLS certificate does not grant a writer epoch.

| Client operation | Authority behavior |
| --- | --- |
| `observe` | Atomically observe the canonical head and current grant; verify the pinned RPC signature and administrator grant |
| `grant` | Persist an exact administrator-signed epoch/owner/anchor request; identical latest retries return that grant |
| `commit` | Verify owner, epoch, construction head and complete candidate; independently execute, sign and durably commit |
| `reconcile` | Return the existing exact-submission signed command observation certificate |
| `block` | Return a bounded encoded record and canonical membership certificate |
| `effect` | Return the requested canonical outbox message and its effect certificate |
| `reconcile_claims` | Observe exact ordered claim IDs in the issuer's canonical ledger, at or after a required issuer epoch; verify lifecycle records and their canonical block references |

`block` verifies canonical membership and exact block hash/root at the client. It
does not replace a restoring application's history/profile audit and independent
replay. A missing/pruned historical snapshot does not become a checkpoint merely
because its block has a valid certificate. This transport currently rejects
successor candidates, as does the underlying authority.

## Wire identity

The service is `myelin.session.authority.v1.AuthorityGateway/Call`. A protobuf
`Packet` has version `2` and a byte body. Version 1 is rejected. The inner body is
bounded postcard; unknown versions/discriminants and trailing bytes are rejected. This does not
change existing session, grant, certificate or journal encodings.

A request contains `(policy_commitment, random_32_byte_challenge, operation)`.
The SDK generates a fresh challenge for each attempt, including read-only calls.
The request digest is BLAKE3 of the literal
`myelin:writer-fence-rpc-request:v1` followed by the exact postcard body bytes.
The reply contains `(request_digest, Result<Outcome, RemoteFailure>, signature)`.
Its BIP340 signature authenticates BLAKE3 of the literal
`myelin:writer-fence-rpc-reply:v1` followed by the postcard tuple
`(request_digest, result)`. The client verifies this signature before consuming
either a result or a remote rejection. An earlier response cannot answer a new
challenge. TLS handshake, admission and malformed-wire errors may instead return
ordinary transport errors; these never prove command absence.

Operation discriminants are fixed in order: Observe (0), Grant (1), Commit (2),
Reconcile (3), Block (4), Effect (5), Claims (6). Outcomes use the same order.
Commit carries the existing `CommitAuthorization` and a candidate with transactions, frame input,
scheduler commitment, data commitments, timestamp, issued intents, consumed IDs
and lifecycle actions, in that order. It has no worker-clock or successor field.
Reconcile contains the authorization and ordered command identities. Block takes
a height; Effect takes height and message ID. Claims takes ordered claim IDs and
the minimum acceptable issuer epoch. Its signed certificate includes the observed
issuer epoch, typed claim records, involved genesis configurations and head
records, and unique snapshot-pruned blocks. See the exact structs in
`session-authority/src/network/wire.rs`; changes require an explicit new protocol.

## Bounds and cancellation

Default request and response body caps are 8 MiB and 40 MiB. Each is configurable
from 1 KiB through a hard 64 MiB maximum. Protobuf framing adds at most the separate
128-byte transport allowance. Bounded serializers count before allocating encoded
copies; bounded decoders also enforce container ceilings. The existing authority
and session validity/resource checks still run. Transport limits are local
operating policy, not new genesis consensus limits.

The default admission limit is 16 blocking operations, configurable from 1 to 64,
shared across all connections. Execution and synchronous WAL work run in
`spawn_blocking`; the permit remains with that job until completion even if the
RPC future is cancelled. This bounds admitted work, not arbitrary TCP/TLS socket
memory. Host network quotas and the existing execution-isolation policy remain
deployment responsibilities. gRPC cancellation cannot preempt a native executor.

Claim queries accept at most 64 distinct nonzero IDs and 32 MiB of evidence.
The issuer authority attests membership in its shared canonical ledger; foreign
ledger certificates or ordinary PoA from a worker copy do not substitute for its
pinned policy. `Missing` and `Issued` remain nonterminal observations. Neither
expiry, a later epoch nor an RPC failure grants cancellation/refund. Consumed and
retired claims cannot be cancelled; cancelled claims require the original source
application's separately committed reclaim command before returning funds.

Oversized query results produce a signed bounded rejection. Increase both peers'
explicit limits when sufficient, query fewer command identities, or use a
separate archival/replay delivery path. Do not assume every 128 MiB session
record fits this 64 MiB transport. A single oversized record cannot be fixed by
silently pruning its authenticated contents.

## Journal integration

1. Create `CommandJournal::open_fenced` with the same pinned policy.
2. Authenticate/enqueue the original commands and reserve against the canonical
   construction head. Build and sign the exact candidate.
3. Persist `begin_fenced_commit` before sending anything.
4. Call `submit_fenced_remote(token, client, request, interpreter)`. It persists
   Unknown before its first await, attempts submission, and queries the exact
   canonical observation whether that attempt succeeded or failed.
5. Use the returned entries and interpreted business outcomes. The separate
   `attempt` field describes only that RPC attempt. In particular, a stale-head
   rejection can mean that an earlier, response-lost submission already committed.
6. After disconnect or restart, call `reconcile_remote` with the retained token.
   Matching receipts recover the original outcome. Same-epoch Missing stays
   pending. Only a strictly newer signed epoch observation can requeue originals.
7. ACK only a recovered business result. Retried commands retain their original
   identity and authorization; a fresh transaction hash is not a fresh intent.

No network error automatically releases a reservation, rotates an epoch, creates
a successor, changes a command ID, or rolls back canonical state. Cancelling the
SDK future leaves the durable journal Unknown. The administrator must explicitly
resolve ownership when the canonical observation is still missing.

## Executed drill scope

The network suite uses ephemeral OpenSSL-generated CAs/certificates, real TCP and
TLS sockets, real RocksDB, actual signed inventory commands and separate worker
copies. No certificates or private keys are installed outside temporary test
directories. It covers downstream TLS response blackholing, disconnect before
submission, old-owner reconnect after revocation, result/ACK recovery, cancellation
while execution is active, global admission, untrusted/missing client credentials,
wrong DNS/pinned certificate identity, plaintext misconfiguration, malformed
versions and request/response caps. The shared-ledger flow additionally runs the
actual scarce-transfer executor: it loses a committed target reply, resolves the
original claim/command, races consume against cancel, and reopens both authorities
before replaying the source and target to their exact roots. Two independently
open stale worker replicas cannot override those terminal states.

With `test-fixture`, authority child processes exit before durable commit intent
and after the committed receipt is created but before RPC delivery. Each is
restarted twice and queried through TLS; the balance changes once. The child
endpoint is rebound on each restart. Process termination is observed before
reopening its databases. These are local process/network fault drills, not
physical multi-host deployment, WAN performance or quorum HA evidence.

```bash
# Requires the workspace toolchain, RocksDB/Clang/protoc build dependencies,
# and the openssl executable (only for ephemeral test TLS material).
cargo test --locked -p myelin-session-authority --features test-fixture
```

See the [migration runbook](../operations/writer-migration.md) for the ownership
and backup order, and [writer fencing](writer-fencing.md) for trust limitations.
