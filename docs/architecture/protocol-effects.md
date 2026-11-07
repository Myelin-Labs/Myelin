# Replayable source-effect authorization

Status: the execution boundary, explicit trust classes, authorization digest
helpers and adversarial integration checks are implemented locally for P1 #10.
A [reusable reference application](reference-transfer.md) now supplies scarce
issuance/intake and copyable receipts, cancellation/refund, consumed-lock
retirement and exact continuation with inherited obligations.
[Offline bundles](session-bundles.md) and the
[compatibility/migration contract](session-succession.md) implement their
separate verification boundaries. These local reference paths are not a deployed bridge.

## Execution interface

New sessions select `SessionConfig.protocol_effects` at genesis. New blocks call
`TransitionExecutor::execute_block_with_context`, passing a
`ProtocolEffectsContext` containing:

- the exact source genesis configuration commitment and session ID;
- the genesis-bound source policy and policy hash;
- height, block timestamp, pre-state root and canonical input root;
- the complete outgoing handoff intents, including authorization bytes;
- the complete versioned successor configuration, when requested.

The default executor method rejects handoff issuance and successor declarations.
Applications must explicitly implement the new method to support them. It then
runs on the candidate clone, before finality or durable mutation. Existing
executors continue to execute ordinary blocks through the default method.

This interface was chosen because the application owns authorization, object
versions, nonce consumption and state disposition. It can validate an intent,
apply the source lock/debit and check the resulting state in one deterministic
transition. The session layer cannot infer those semantics from opaque bytes.

The framework derives handoffs from the accepted intents and post-state root.
A new block commits the reconstructed source context immediately before its
resource-usage commitment. The block/frame already commits the post-state root;
changing the source policy, request or successor config therefore changes the
signed block. The two protocol commitments both count against the data budget.

## Trust classes

| Genesis policy | Meaning | Source issuance and migration | Target intake |
| --- | --- | --- | --- |
| `Disabled` | No cross-session effects | Rejected | Rejected |
| `AuthorityMetadata { policy_hash }` | Operator-attested statement | Requires explicit executor opt-in | May interpret statements under its own rules |
| `Application { policy_hash }` | Identified application policy must authenticate effects and enforce disposition | Requires that policy's implementation | Rejects authority-only sources; still checks its own allowed source policies and payload semantics |

`require_authority_metadata(expected_policy_hash)` is an explicit helper for
statement-only applications and non-economic fixtures. It is not an escape hatch
for an `Application` policy. Target preparation, durable consumption, recovery
and replay independently check the source trust class and the exact source
record/genesis/context linkage. Changing a source genesis descriptor cannot
promote its already signed metadata history.

The descriptor is encoded in genesis and can be exported as a typed value. An
`Application` label or a nonzero policy hash does not prove a correct program,
owner authorization, an actual debit, a trusted deployment, or CKB adjudication.
A verifier must obtain the intended program/policy and reproduce its transition.
Applications must bind their allowlists and interpretation to immutable profile,
policy and state data. Complete portable manifest/artifact handling is #13.

## Owner authorization and disposition

`EffectAuthorizationScope` carries actor, exact object, object version and nonce.
Its `handoff_digest(source_genesis, intent)` additionally binds the exact source
genesis, target session/policy, payload, expiry and evidence requirement. The
payload must canonically encode recipient, amount/components and intake schema.
The signature bytes themselves are excluded from the digest.

`successor_digest(source_genesis, target_config)` binds the scope and the entire
target configuration commitment. That includes the target program, consensus
module and authority configuration, resource limits, time rules, root, cursor
and predecessor reference. Matching a codec alone does not authorize an upgrade.
A supported migration policy must separately validate semantic compatibility.

These helpers do not prescribe a signature scheme. The application must verify
the signature, map the actor to the current owner, check the current object
version, reject duplicate/reused nonces and enforce the intended state change.
A platform login is not authorization for an asset transfer.

For scarce assets, a valid signature is insufficient without the exact source
lock/debit. For copyable receipts, a separately identified application policy
must explicitly permit copying; intake must not reinterpret that policy as
scarce-asset conservation. `myelin-session-transfer` implements those distinct
source/intake policies and the separately authenticated cancellation/refund path.

The current helper domains are
`myelin:session-effect-handoff-authorization:v1` and
`myelin:session-effect-successor-authorization:v1`, using canonical postcard
serialization of their typed tuples after the literal domain. They are distinct
from ADR-008's proposed cross-deployment envelope/vector encoding. The core
context and trust-policy domains likewise have explicit v1 identities. An
application must version any different encoding or authorization interpretation.

## Replay, worker and recovery boundaries

Live execution and replay reconstruct the same context and call the same
executor method. The test suite includes a deliberately incorrect producer
that signs a structurally consistent block with no source lock, and another
with a bad owner signature. Replaying with the intended application rejects
both, despite valid block finality and matching record commitments.

The native worker's current `MWK2` protocol uses one `Block` operation for the
complete bounded context and lifecycle actions. Live execution and replay check
the same pre-root/input binding and invoke the same child policy method. Older
worker packets are rejected; there is no compatibility operation or downgrade.

Ordinary restart audits record linkage, trust classes and finality, then restores
the checkpoint. It does not re-execute every application's history and must not
be reported as owner-authorization verification. Full range replay re-executes
the requested range. Offline verification must also obtain and validate source
dependencies, artifacts and trusted genesis anchors. A future court would need
to implement the same policy; this code does not deploy one.

## Current storage format

Only MREC envelope 14, session block 6 and RocksDB schema 16 are accepted.
Every config binds resource limits and an explicit protocol-effects policy.
Every block records its original snapshot byte count, and every successor carries
its typed transition receipt. Historical record/store decoders and schema-marker
upgrade APIs have been removed. Older data must remain with its original binary;
the current executable does not import or upgrade it.

## Acceptance evidence

`session-store-rocksdb/tests/support/effect_regressions.rs` checks owner signature
and disposition failures, duplicate intents, changed recipient/amount/object/
version/expiry/target/evidence/nonce/source genesis, default-policy rejection,
metadata promotion rejection, full successor configuration authorization, real
source lock and target credit with restart/replay, and version/trust-class import
boundaries. The worker process test checks that the child's policy sees and
rejects a mismatching genesis policy.

The test ledger uses actual secp256k1 Schnorr signatures and checks source
available/locked amounts and target credit. It intentionally supports a narrow
fixture policy and does not implement cancellation, refunds, multi-object
inventory, portable artifact validation, or migration policies. The reusable
[reference transfer](reference-transfer.md), [session bundles](session-bundles.md)
and [succession contract](session-succession.md) provide those separate paths and
their acceptance evidence; the narrow fixture does not stand in for them.

Lifecycle operations require the `execute_block_with_lifecycle` policy opt-in.
The unified worker block message always carries its explicit actions; an existing
source-effect override cannot silently drop nonempty actions.
See [the lifecycle contract](handoff-lifecycle.md) for record commitments, CAS,
resource charging and replay.


Remote scarce applications use `RemoteApplication { policy_hash, terminal }`.
The kernel recognizes this genesis-bound coordination class independently of an
inbox/transport adapter. Source and target must bind the same terminal policy and
its exact roles. Every consumption requires the exclusive certificate for the
exact target candidate, including when both histories are colocated or a source
history has been copied into another store. A normal `Application` policy cannot
import or downgrade that domain. Forwarding, continuation and policy targets are
rejected; owner cancellation/refund semantics remain application-verified.
