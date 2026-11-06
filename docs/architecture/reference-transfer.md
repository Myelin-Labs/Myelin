# Shared-store reference transfer policy

`myelin-session-transfer` supplies a reusable deterministic application for P1
#10/#11. It implements owner-authorized issuance, target intake, authoritative
scarce-transfer cancellation, later refund, consumed-lock retirement and exact
continuation with inherited obligations. Its atomicity is limited to a shared store.

## Identity and authorization

`TransferPayload` uses a checksummed `MTRP` version-1 envelope and canonical
postcard encoding. It binds deployment and uniqueness domain, transfer kind,
intake schema, exact source genesis/profile/consensus/policy identity, asset
namespace and object, owner and recipient, source version, amount, components,
nonce, and either an exact target genesis or a shared-store intake policy.
Components are opaque application bytes; games need not share inventory layouts.

The owner's BIP340 Schnorr signature uses the source-effect authorization domain
documented in [protocol effects](protocol-effects.md). It also binds expiry and
the complete evidence requirement. The signature-independent authorization digest
is the transfer ID. Changing a valid signature or the eventual handoff identity
does not create another transfer right. A platform login has no role here.

Source state checks ownership, exact version and components, available amount,
and an unused owner nonce. Stable command IDs bind the session genesis and typed
command. Retrying the same intent must query its command receipt; a new handoff
or nonce is a new authorization, not a retry mechanism.

## Two distinct policies

| Policy | Source transition | Target transition |
| --- | --- | --- |
| `Scarce` | Subtract available amount, add an equal pending lock, advance version and consume nonce | Add available amount, retain transfer nullifier and receipt |
| `CopyableReceipt` | Preserve available amount, create no lock, advance version and consume nonce | Create a receipt only; never create or credit an asset account |

The policy commitment fixes the kind, deployment/store domain, state caps and
sorted allowlist of exact source identities. The target intake schema cannot
reinterpret copyable receipts as scarce assets. `SourceBindings::from_store`
checks each allowlisted genesis against the selected store before executor
construction or restoration. It proves those local anchors, not source-program
execution, remote finality or external uniqueness. Source policy artifacts and their intended semantics remain trust inputs. The
[offline bundle verifier](session-bundles.md) obtains pinned artifacts and
reexecutes the source policy under independently recognized anchors.

Execution works on an isolated state clone. Every command must match one issued
or consumed handoff in order; unmatched effects, extra CellTxs, invalid commands
and unsupported successor declarations reject the complete candidate. Limits
bound accounts, pending locks, receipts, nonces and component bytes. Checked
arithmetic and canonical sorted indexes apply on execution and snapshot import.

## Atomicity and lifecycle

The source lock, issued handoff, command receipt and outbox commit in the source
block's RocksDB transaction. Target credit, consumption marker, receipt and
outbox commit in the target block's transaction. Competing policy targets must
use the **same physical store**; its handoff-key CAS admits only one consumer.
An equal `uniqueness_domain` value on independent stores does not coordinate them.

Source pending locks remain after consumption until explicit retirement. They are nonspendable
bookkeeping backing the transfer, not another balance available to the owner.
In a closed test with no other funding, source available plus target available
is conserved. Do not count consumed backing locks as a second spendable supply.
The current command profile can remove that backing after verifying the
consuming block. It never credits source available balance; cancellation cannot
refund a consumed transfer.

Expiry rejects late intake but never unlocks the source. There is deliberately
no timer-based refund. Cancellation now races consumption on the same indexed handoff key and commits
before a separate refund block. The owner signs the exact source genesis,
transfer ID and handoff ID. Refund always returns to the original owner, so no
caller can choose another beneficiary. Exact continuation preserves pending
locks and nullifiers; cancellation requires a fresh signature bound to the
current owning session. See [lifecycle protocol](handoff-lifecycle.md).
Nonce and receipt caps fail closed; unsafe nullifier pruning is not provided.

## Run and replay

Build once and retain that exact binary: its digest is part of the application
profile, so rebuilding may make an existing example store incompatible.

```bash
cargo build --locked -p myelin-session-transfer --example shared_store_transfer
target/debug/examples/shared_store_transfer resume /tmp/myelin-transfer-demo scarce
target/debug/examples/shared_store_transfer replay /tmp/myelin-transfer-demo scarce
target/debug/examples/shared_store_transfer resume /tmp/myelin-transfer-demo scarce
```

Use `copyable` and a different store directory for the receipt variant. A fourth
argument selects `exact` (the default) or `policy`; for example:

```bash
target/debug/examples/shared_store_transfer resume /tmp/myelin-policy-demo scarce policy
target/debug/examples/shared_store_transfer replay /tmp/myelin-policy-demo
```

Changing the target mode when resuming an existing transfer fails without
reissuing it. The integration test also races two policy-compatible targets;
this single-target CLI example does not simulate a distributed network. The
example contains public development keys. `resume` reconciles existing heads
before acting, and never issues another handoff after an ambiguous retry.
Each invocation closes and reopens the store, audits both sessions and fully
replays their histories and any source continuation. JSON reports expose exact block/consumer references,
policy, balances, replay receipts, the authoritative lifecycle with exact issuing/consuming/cancelling/refunding
blocks, and explicit capability booleans. This is a
local example, not the portable offline bundle required by #13.

The same binary demonstrates an owner-authorized continuation and an explicitly
rejected rule change, without a new gate or compatibility selector:

```bash
target/debug/examples/shared_store_transfer issue /tmp/myelin-continuation-demo scarce
target/debug/examples/shared_store_transfer unsupported-upgrade /tmp/myelin-continuation-demo scarce # fails
target/debug/examples/shared_store_transfer continue /tmp/myelin-continuation-demo scarce
target/debug/examples/shared_store_transfer cancel /tmp/myelin-continuation-demo scarce
target/debug/examples/shared_store_transfer refund /tmp/myelin-continuation-demo scarce
```

Continuation is one standalone `Continue` command under command schema v3. Every
account owner and receipt recipient must sign, once in sorted order, the digest
of `(source_genesis, current_state_root, complete_target_config_commitment)`
under `myelin:reference-transfer-continuation:v1`. The application profile,
effect policy and state remain exact. Resource or authority changes are bound by
the same signatures; rule upgrades, schema changes and resets are unsupported.

## Verification

Integration tests exercise real RocksDB issuance/intake/restart/replay, two
competing policy targets, owner signature and field mutations, exact target
genesis and source anchors, expiry, nonce/version reuse, late-batch rollback,
state caps, copy-to-scarce rejection, alternative valid signatures, and canonical
snapshot imports. A Linux process test executes this same policy in the bounded
native worker, checks native/process block equality and replays it after commit.
The worker fixture supports leaf sources; it is not a general offline artifact
loader or unknown-program sandbox.

The crash matrix runs seven real child-process termination boundaries for both
explicit and policy targets: before/after source commit, before/after target
commit, before delivery, after durable delivery but before ACK, and after ACK.
Every case reopens the real store, checks exact heads, locks, receipts and marker,
resumes twice, fully replays and verifies conservation. Its separate idempotent
delivery sink has one effect after an ACK-loss retry. Missing commands become
retryable in this controlled drill only after the exact old writer process is
confirmed dead; this is not a cross-host fencing proof. A further matrix tests eight real exits before/after cancellation and refund
for exact and policy targets. [Continuation tests](../../session-transfer/tests/succession.rs)
cover sealed-source obligations, and the separate
[authority/network drills](writer-fencing.md) cover canonical writer fencing.
The local process-exit matrix does not substitute for that authority protocol.

`fixtures/protocol/reference-transfer-v1.json` fixes the payload bytes,
authorization preimages, signatures and transfer IDs for both policies. Rust
checks byte identity and BLAKE3. The independent Python encoder additionally
checks field ordering and BIP340 signatures; it does not implement BLAKE3.

## Cancel and refund independently

Use one retained binary and a fresh store. `issue` commits only the source lock;
`cancel` requires the exact owner's authorization; `refund` requires the earlier
cancellation. `replay` audits a target that has not yet produced a block as an
unchanged genesis, without fabricating a replay receipt.

```bash
target/debug/examples/shared_store_transfer issue /tmp/myelin-cancel-demo scarce policy
target/debug/examples/shared_store_transfer cancel /tmp/myelin-cancel-demo scarce policy
target/debug/examples/shared_store_transfer refund /tmp/myelin-cancel-demo scarce policy
target/debug/examples/shared_store_transfer replay /tmp/myelin-cancel-demo
```

Repeating `cancel` or `refund` reconciles the existing terminal state; it does not
issue new rights. `refund` before cancellation and `resume` after cancellation
fail. Copyable receipts do not have refundable source locks; this reference
policy does not implement copy revocation.


## Retire a consumed source lock

All new stores use the current command profile, including scarce-lock retirement.
Retain the exact executable used to create a store for replay.

```bash
target/debug/examples/shared_store_transfer issue /tmp/myelin-retirement-demo scarce policy
target/debug/examples/shared_store_transfer resume /tmp/myelin-retirement-demo scarce policy
target/debug/examples/shared_store_transfer retire /tmp/myelin-retirement-demo scarce policy
target/debug/examples/shared_store_transfer replay /tmp/myelin-retirement-demo
```

`resume` commits the target intake; `retire` independently reconciles its exact
consumption reference and clears source bookkeeping. The example ends with source
available 90, locked 0 and target available 10. Repeating `retire` returns the
same result. Retirement before intake, a refund after intake, copyable retirement
and incompatible genesis identities all fail. The output reports retirement
capability and the exact consuming/retiring references.

`scripts/myelin_transfer_retirement_acceptance.py --binary /absolute/path/shared_store_transfer
--output /absolute/path/result.json` runs the example as separate processes for
both target modes, checks repeated operations and failed transitions, preserves
current command behavior, and writes the binary digest plus every exit status. It only uses
its own temporary local stores.
