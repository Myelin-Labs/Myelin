# Session succession and migration

A successor seals its source permanently and creates one target genesis in the
same RocksDB transaction. New successor blocks distinguish state preservation
from state conversion. Equal state codecs alone never authorize new rules.

## Support matrix

| Request | Current mechanism | Application responsibility |
| --- | --- | --- |
| Same program and semantics | `SuccessorState::Continue` copies exact snapshot bytes; complete application profile and protocol-effects policy must match | Authorize the complete target config; retain economic nullifiers and outstanding obligations |
| Authority or committee rotation | Same continuation mechanism, with a new target consensus config | Source policy authorizes the rotation; old history retains its old verifier |
| Changed rules with the same codec | `Compatible` requires the source `Application` policy, declaration and vector artifact hashes; root and snapshot remain exact | Check the declared compatibility policy and vectors in the source executor |
| Changed state schema or representation | `Migrate` carries a versioned program identity, input schema, exact inputs and converted snapshot/root | Execute the approved migration artifact, validate the target root and preserve required rights/obligations |
| Seasonal reset | An explicit migration, never an identical-state continuation | Identify reset fields and preserve balances, reward nullifiers and external obligations |
| Arbitrary program switch or opaque metadata-only conversion | Rejected by the default executor | Implement and authorize a specific policy before enabling the path |

The policy hook is `TransitionExecutor::prepare_successor_state`, called after
source block execution. Source execution still receives the complete target
config through `ProtocolEffectsContext` and must authenticate it. Migration
inputs and artifact choice must be determined by that authenticated request and
the approved source program, not by untrusted mutable services. The hook returns
a computed target root; the runtime checks it against the requested genesis.
The generic runtime cannot infer the semantics of an application snapshot.

## Record and commitment boundary

All blocks use version 6 and MREC envelope 14. Successor declarations always
include a typed transition receipt in the canonical record. Store schema 16
rejects earlier databases. Application-authorized state conversion is a current
protocol operation; it is independent of storage format upgrades, which are unsupported.

`myelin:successor-declaration-v2` commits, in order: source session id, u64-LE
height, source root, source profile commitment, complete target config commitment,
source genesis commitment, input root, source snapshot BLAKE3, and one state tag:

- `0`: exact continuation, with no additional fields.
- `1`: compatibility policy hash, declaration hash, vector hash.
- `2`: u32-LE migration artifact version (currently 1), program digest, input
  schema hash, u64-LE input length and input bytes, target root, u64-LE target
  snapshot length and snapshot bytes.

Hashes are 32 raw bytes. Only declarations with the current transition receipt
are accepted. The enclosing source block also
commits the frame, source post-root and declaration commitment. The target
predecessor reference always names the **source** final root, even when conversion
creates a different target root. Cursor, logical time and timestamp are preserved.

Source snapshot size remains charged to the source limits. Converted snapshot
size is checked against target limits; migration input size is checked against
the source input envelope. All receipt and target snapshot bytes additionally
count toward the source's complete record budget, including finality proof.

## Replay, archives and crash recovery

Offline range replay runs the source transition and successor hook again and
compares the complete receipt and target genesis. A signed receipt passing
structural/finality audit is not a claim of successful migration execution.
Recovery checks both directions of lineage and compares the target snapshot with
the source checkpoint or the committed converted snapshot. The target cannot
reinterpret source history with its own profile or authority.

Bundle exports derive mandatory artifact edges from the receipt: compatibility
declaration/vectors or the migration program. Their identifiers are BLAKE3 hashes
of the exact artifact bytes. Export requires them in `ArtifactSource`; transitive
build/runtime inputs use the existing dependency graph. Import checks these
edges before replay. Target and source each require their own recognized trust
anchor, historical application artifacts and finality configuration.

Historical interpreters use `BundleStore::read_artifact(id, max_bytes)` to obtain
the migration bytecode or other declared rule inputs. Every read checks the
inventory, caller's byte ceiling and exact content hash, even after a successful
catalog audit. The API returns bytes; it never loads native plugins into the
verifier. Unknown native application code runs only through the pinned isolated
replayer, whose normal successor hook reads and executes those archived bytes.

Source block/checkpoint/head and target genesis/checkpoint/head commit together.
Tests terminate processes without destructors immediately before or after this
transaction, then reopen the real database. Before commit neither source sealing
nor target creation survives; after commit both do and the source cannot create
another successor.

## Application ownership and acceptance evidence

Command indexes remain session-scoped. Source signatures bind source genesis;
pending work needs new target authorization or an explicit transfer protocol.
Application reward nullifiers must survive continuation/conversion even when the
command namespace is new. The regression reward ledger preserves balances and
nullifiers while changing schema and resetting only seasonal score.

Outbox records retain their source session and deterministic message ids. Sealing
does not move or acknowledge them; workers must continue draining that source.
Handoffs retain their original IDs and source signatures. The reference transfer
application now supports exact continuation with all rightsholders' authorization.
The unique sealed predecessor chain proves ownership of inherited locks; new
cancellation signatures bind the current session. Nonces and receipt nullifiers
are copied unchanged. Pending locks must be reconciled before another continuation
would exceed the 32-link ownership bound. Changed-rule/schema migration remains
unsupported in that application and needs an explicit obligation policy.

The signed reward ledger and bounded migration bytecode currently exercise the
live/store/replay boundary as regression fixtures. The reference transfer CLI
demonstrates successful continuation and a rejected rule upgrade; process exits
cover atomic sealing followed by inherited cancellation/refund.

The archive suite now replaces its process-local append fixture with isolated
execution of separately archived, current-format migration bytecode. After the
source database and original input artifacts are removed, the sandbox replays the
source conversion and restores the successor under its different state codec.
The ordinary replay CLI returns the same result. A second, validly signed history
with a forged converted root passes structural/finality audit and fails actual
migration execution. Post-audit artifact mutation, missing bytes, excessive reads
and substituted dependency edges fail explicitly. This fixture has no economic
state; balance/nullifier preservation remains covered by the signed reward ledger.
It does not claim that arbitrary application migrations are authorized or that
synthetic test build packages reproduce production binaries.

Core validation retains both histories, external trust/policy inputs and reports
under `isolated-migration/`. It runs inside the existing workspace suite; there is
no separate migration gate. Source migration execution and target genesis
restoration are separate reports, so a zero-block target cannot stand in for
reexecuting its source conversion.
The independent fixed commitment corpus is `fixtures/protocol/successor-transition-v1.json`; its preimages were assembled separately from the Rust implementation.
