# Independent-store handoffs

Status: implemented reference for #16 and #17, with independent-store receipt
and scarce test-asset drills. Remote scarce intake requires a typed, independently
registered terminal policy and an exclusive certificate for the exact candidate.
Shared-store policies cannot acquire remote uniqueness through an opaque label.

## Proof and inbox contract

`HandoffProofBundle` format 1 retains the exact source configuration, issuing
record/finality proof, handoff identity and target genesis. A required evidence
message can have a second independently finalised source block. Import policy
is obtained locally, separately from that bundle. It fixes both genesis
commitments and a finite proof-byte limit.

Verification checks the registered source consensus descriptor and actual proof,
signed record/frame/payload commitments, exact handoff inclusion and target,
owner authorization, payload schema and the application's source allowlist.
Required evidence receipts are reverified by local stage adapters, including the
message's source commitment. Counts and nonempty bytes do not establish a stage.
A proof of a later block is a finalised source statement under that configured
authority; it is not a source application-history replay from genesis.

Remote policy targets are unsupported. One exact target is required. Inbox
identity includes the source genesis namespace and canonical handoff identity.
The primary proof key binds both; its lookup index rejects ambiguous identities
across namespaces.
Equivalent valid deliveries do not replace the first retained proof. Import is
durable before acknowledgement, and does not credit the target or create local
source genesis/block indexes. Consumption rechecks the source and commits target
state, permanent consumption and command receipt together.

The inbox has entry/byte quotas and bounded flat proof dependencies. Aggregate
external proof bytes for one transition also fit its genesis record-byte budget.
Observed signed source blocks are indexed by genesis and height. Conflicting
hashes or known adjacent links quarantine that source durably. Recovery/writes
that depend on it then stop; there is no automatic branch selection or refund.
This detects observed equivocation, not Byzantine safety for arbitrary unseen
forks signed by an otherwise trusted source authority.

Recovery, replay and archive export use `load_handoff_import` and the same
explicit source-resolution code. `load_block` continues to report a missing
local source block. Bundle v3 includes the original proof, source consensus and
execution/build dependencies with separate external trust anchors. It can replay
the target after both producing databases are deleted.

## Runnable receipt reference

Build the examples once, then run operations as separate native processes:

```bash
cargo build --locked -p myelin-session-transfer --examples
target/debug/examples/remote_receipt init /owned/fresh-deployment
target/debug/examples/remote_receipt issue /owned/fresh-deployment
target/debug/examples/remote_receipt export /owned/fresh-deployment /owned/relay.proof
target/debug/examples/remote_receipt import /owned/fresh-deployment /owned/relay.proof
target/debug/examples/remote_receipt consume /owned/fresh-deployment /owned/relay.proof
target/debug/examples/remote_receipt replay /owned/fresh-deployment
```

Use a new path for `init`. Public development keys, a copyable receipt and two
different PoA authorities are fixed by this fixture. Source/target RocksDB
directories are independent. The immutable operator configuration files supply
local trust; a relay cannot replace them through its proof. The original binary
is pinned. Existing commands recover receipts and do not reset a missing database.

The existing transfer acceptance command exercises both references:

```bash
python3 scripts/myelin_transfer_retirement_acceptance.py \
  --binary target/debug/examples/shared_store_transfer \
  --remote-binary target/debug/examples/remote_receipt \
  --terminal-binary target/debug/examples/remote_transfer \
  --output /owned/transfer.json
```

Its process drill retains original receipts, loses replies before client
observation, redelivers before/after consumption, makes the source database
unavailable, restarts native delivery/target processes, and replays the original
target result. This is a file-relay reference, not an HTTP/public deployment.

## Scarce reference decision

Use one explicit trusted terminal authority, an exact source/target pair, test
assets and no forwarding or automatic refund. Source and target stores remain
independent. `TransferCoordination::Remote` binds the full terminal policy into application
genesis. Intake independently recognizes the complete source policy as well as
the target policy. Both name this exact source/target pair and coordinator key;
the source cannot use shared-store cancellation semantics. Genesis exposes the
scope as `ProtocolEffectsPolicy::RemoteApplication`, allowing the session kernel
to require a matching certificate even when an operator opens a copied source
history as local data without an inbox registry. Store placement cannot downgrade
this rule. The kernel also forbids forwarding, continuation and policy targets
in this domain.

The coordinator must exclusively own its durable claim database and signing key.
Normal restart reopens that same database; missing state is an error, not an
empty claim set. Copying/rolling back the coordinator and retaining its identity
violates this trusted-authority model. Loss of authoritative state stops progress.
It does not authorize reconstructing absence or refunding outstanding locks.

| Transition | Required evidence / actor | Durable location | Irreversible point |
| --- | --- | --- | --- |
| Source issue | Exact owner-authorized lock, source policy and finality | A's canonical block/state/outbox | A's atomic commit removes source availability |
| Import | Full pinned source proof, schema/authorization and required stages | B's inbox | Durable import only; no target credit |
| Target prepare | Exact original command, base head, proposed post-state and candidate | B's pending candidate/reservation | No credit; conflicting target progress must be fenced while the decision is unknown |
| Authorize consumption | Authenticated exact target candidate; global claim still Issued | Coordinator's synchronous CAS | One ConsumeAuthorized(candidate) decision permanently excludes cancellation |
| Target commit | Matching consumption certificate, base head and exact candidate | B's state/nullifier/receipt transaction | Canonical target credit; another clone/candidate cannot obtain a second authorization |
| Source completion | Consumption decision and final target evidence | A's retained foreign consumption proof, then canonical retirement | The backing lock can retire without returning source availability |
| Authorize cancellation | Fresh owner cancellation for the exact source lock; global claim still Issued | Same coordinator CAS | Cancelled permanently excludes any future consume authorization |
| Cancel source | Exclusive cancellation certificate plus owner authorization | A's canonical cancellation record | The source lock is cancellable once, not yet refunded |
| Refund source | Canonical cancellation and the still-unreclaimed original lock | A's canonical reclaim transaction | Source availability returns exactly once |
| Unknown / partition | No matching terminal evidence | Original journal/candidate/reservation remains | No refund, new command identity, forwarding or replacement candidate |

ConsumeAuthorized is an irreversible reservation for one exact target history.
It must not be reported as a successfully committed game result before B's
canonical commit. Pending target bytes must survive a crash before the
coordinator is contacted, so recovery can finish the original candidate rather
than ask for another credit. If cancellation wins, that same candidate can never
be committed; reservation release requires the actual cancellation certificate.

Source expiry constrains the declared target block time. Wall-clock passage,
old timestamps, missing local records and delayed ACKs are not exclusion proofs.
A delayed proposal competes with explicit cancellation at the coordinator; only
one terminal decision can win. Disconnected recipients may remain locked.

Two target replicas may hold the same history and original command. This is one
canonical right, not two assets to sum. Different proposed heads/receipts compete
for the same irreversible authorization. Authority/writer keys must remain under
their configured canonical control, following the fencing contract; raw cloned
signing authorities are not quorum HA.

## Alternatives and failure boundaries

| Uniqueness owner | Trust and availability | Cost / missing protocol |
| --- | --- | --- |
| Shared terminal authority (reference) | Trust its signing policy and durable non-rollback state; outage blocks new decisions | Bounded RPC/verification and durable CAS; no cross-store database transaction |
| Canonical target consensus | Trust the target's finality/writer model; cancellation needs a verifiable terminal exclusion in that same model | Consensus plus remote proof retention; ordinary absence/ACK is insufficient |
| L1 claim/escrow Cell | Trust the chosen chain/confirmation model; both consume and refund must spend/constrain the same custody/claim domain | Actual scripts, fees/capacity, availability, reorg and exit handling; a nullifier label alone is insufficient |

The reference must reject unsupported remote multi-target policy competition,
forwarding, pending-lock continuation and automatic timeout refunds. Exchanges
involving two separate transfers need their own atomicity/failure protocol;
ordinary compensation is not trustless atomic exchange. Malicious relays can
delay, duplicate, reorder or withhold bytes. They cannot create owner/finality/
terminal signatures. Malicious configured authorities remain explicit trust
failures; observed conflicting declarations must halt affected progress.

Retain permanent terminal/nullifier identities and all source/target/policy/
certificate dependencies for the lifetime of the claim domain. Archive compaction
must preserve recognition and proof closure, rather than create fresh issuance.
Reorgs in an L1-backed alternative need separate confirmed-history rules; they
cannot be hidden by an ordinary transport retry.

The test-asset reference runs those failure boundaries as separate native
processes. The existing transfer acceptance command has 44 shared-store calls,
14 remote-receipt calls and 63 remote-scarce calls. Integration tests also race
two independently signed target candidates against owner cancellation in three
threads. Another regression copies genuine source history into an unregistered
target store and verifies that unsigned terminal consumption is still rejected. The original consume certificate and canonical receipt survive restart;
a competing candidate, cancellation after authorization, and consume after
cancellation are rejected.

### Runnable scarce reference

```bash
cargo build --locked -p myelin-session-transfer -p myelin-session-authority \
  --examples --features myelin-session-authority/test-fixture
target/debug/examples/remote_transfer init /owned/fresh-scarce
target/debug/examples/remote_transfer issue /owned/fresh-scarce
target/debug/examples/remote_transfer export /owned/fresh-scarce /owned/lock.proof
target/debug/examples/remote_transfer import /owned/fresh-scarce /owned/lock.proof
target/debug/examples/remote_transfer prepare /owned/fresh-scarce /owned/lock.proof /owned/candidate
target/debug/examples/remote_transfer authorize /owned/fresh-scarce /owned/lock.proof /owned/candidate /owned/consume.cert
target/debug/examples/remote_transfer commit /owned/fresh-scarce /owned/consume.cert
target/debug/examples/remote_transfer record /owned/fresh-scarce /owned/target.record
target/debug/examples/remote_transfer complete /owned/fresh-scarce /owned/lock.proof /owned/target.record
```

Use `decide-cancel ROOT PROOF CANCEL_CERT` before consumption authorization,
then `cancel ROOT PROOF CANCEL_CERT` and `reclaim ROOT PROOF` for the exclusive
refund branch. `release ROOT CANCEL_CERT` releases a target reservation only
after independently verifying that cancellation. Ordinary `Cancel` is rejected
by the remote application. Mere expiry, a failed RPC, and local `Missing`/`Issued`
answers never supply this certificate. The shared-ledger claim API explicitly
rejects remote terminal domains, whose decisions belong to `TerminalAuthority`.
Keep the original cancellation authorization and command bytes across retries;
a fresh authorization is a different request, not evidence of the first outcome.

`prepare_remote` stores the stamped original request and signed candidate before
returning them. Other writes and writer rotation stay fenced while this record
is pending. On reopen an uncommitted remote reservation is preserved. A valid
consume certificate finishes precisely that candidate, even after its reply or
canonical-commit reply is lost. A foreign target certificate is rejected. Only
an actual cancellation certificate may release an uncommitted reservation.

The terminal authority uses synchronous exclusive durable decisions, retained
source/candidate/owner evidence and finite claim/byte quotas. Reopen rechecks all
retained certificates and retention counters. `open` refuses missing identity or
storage; `create` refuses an existing path. The configured trusted signer and
non-rollback database remain the uniqueness owner; copying them creates a trust
failure, rather than a supported availability deployment.

Source completion carries a `RemoteConsumptionProof` inside the canonical
`RemoteRetire` action. It verifies the separately pinned target finality,
application scope and exact consumption certificate before retiring the backing
lock without returning availability. It is a flat, bounded witness; its target
record cannot contain another retirement, issuance or continuation. No foreign
`load_block` entry is manufactured. The source can replay completion after the
target service is gone. Bundle v3 retains this witness and the target's full
execution/consensus artifact dependencies under a separate external anchor.

The current persistence formats are MREC 14, block 6, RocksDB schema 16 and
fencing metadata 2. The reference transfer uses state codec 2 and command schema
4. Only these current shapes are accepted; no compatibility readers or store
migration are provided. These breaking formats are deliberate.
