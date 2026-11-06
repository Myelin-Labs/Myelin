# Aggregate commit resource limits

Status: protocol data limits and an opt-in Linux native worker are implemented
and verified locally for P1 #9. See [native workers](native-workers.md) for CPU,
address-space, wall-time and cancellation enforcement. An ordinary in-process
executor still cannot be interrupted by an asynchronous timeout.

New sessions require `SessionConfig.commit_limits = Some(CommitResourceLimits)`.
The limits are immutable genesis data. Their version-one commitment is included
in the current v3 session configuration commitment, alongside the explicit
protocol-effects policy. Changing any limit changes that commitment; recovery
does not insert current defaults into old histories.

| Budget | Counted material | Default |
| --- | --- | --- |
| CellTx count / bytes | Existing `max_block_transactions` / `max_block_bytes` | Explicit config |
| Frame bytes | Entire postcard `FrameInput`, including command identities | 16 MiB |
| Issued / consumed handoffs | Independent counts | 64 each |
| Handoff payload bytes | Issued and resolved consumed payloads combined | 16 MiB |
| Authorization bytes | Issued and resolved consumed authorizations combined | 256 KiB |
| Data commitments | Request, executor, outbox, handoff, successor, source-context and resource commitments | 4,096 |
| Outbox messages / bytes | Count and total UTF-8 topics plus payloads | 1,024 / 16 MiB |
| Snapshot bytes | Complete executor snapshot | 64 MiB |
| Record bytes | Complete finalised record, including proof, snapshot and binary envelope | 128 MiB |

Boundaries are inclusive. Zero effect counts disable those effects. Checked
arithmetic rejects overflow. `ResourceLimit { resource, used, limit }` identifies
the failing budget; an early serialization stop reports the first known size
above the limit rather than scanning the rest of the object.

## Enforcement

Preparation checks predictable request sizes before command deduplication,
handoff-vector allocation or executor cloning. It adds consumed payloads and
authorizations to the same accumulator as issuance. Executor outbox and data
commitments are checked before constructing durable messages. Snapshots are
checked before preparing the final record; a borrowed serializer checks the
record's lower bound before requesting consensus. Commit checks the complete
record with its encoded finality proof before signature verification and writing.

RocksDB independently checks the genesis limits before cloning record buffers.
It resolves consumed handoffs inside the storage transaction and rechecks their
combined budget. A failure commits no head, checkpoint, command receipt, handoff
marker or outbox message. Source evidence and target checks continue to apply.

Recovery, range replay and `FinalisedBlockRecord::decode_for_config` apply the
same protocol budgets. Each budgeted block ends its data commitments with a
domain-separated commitment to both the limits digest and original snapshot
length. Changing a stored genesis budget therefore breaks the block commitment,
even if its checksum is recomputed. Eliding a snapshot
does not reduce the record's charged size: validation restores its encoded length
arithmetically, including the postcard length prefix. Replay regenerates the
snapshot and checks its length even for a pruned record.

The record decoder also has local ceilings: 128 MiB for an encoded record or
byte buffer, 65,536 items per typed sequence, and 4,096 bytes per string. Nested
transaction buffers have explicit bounded deserializers. These ceilings bound
imports; they are not measures of actual process RSS or protocol VM cycles.
Checksums, exact envelope lengths and trailing-byte rejection remain required.

## Current storage format

Only MREC envelope 13, session block 6 and RocksDB schema 15 are accepted.
Every config binds resource limits and an explicit protocol-effects policy.
Every block records its original snapshot byte count, and every successor carries
its typed transition receipt. Historical record/store decoders and schema-marker
upgrade APIs have been removed. Older data must remain with its original binary;
the current executable does not import or upgrade it.

## Operational execution and storage integration

Use `myelin-session-worker::ProcessExecutor` for trusted native executors that
require terminable execution. Its settings are separate from protocol validity;
its report distinguishes VM cycles, host wall/CPU time, lifetime peak RSS and
sampled post-exec RSS. Fault tests verify no durable state advancement and a
successful single retry after worker failure. The core profile includes these
tests on supported Linux hosts; other platforms do not acquire an isolation claim.

Whole-database retention and bounded archive/recovery work are implemented in
[session bundles](session-bundles.md); `decode_for_config` supplies their
per-record resource check. The bundle runner separately isolates unknown native
programs. [Workload measurements](../operations/workload-measurements.md) retain
actual filesystem-capacity failures and recovery in both process layouts. Disk
headroom and deployment quotas remain operator policy: a record limit alone
does not bound the lifetime database size.
