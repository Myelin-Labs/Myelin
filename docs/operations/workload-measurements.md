# Workload measurements

Run the existing inventory, board-game and fixed-tick rules through authenticated
original commands, the command journal, `SessionChain`, development PoA finality
and synchronous RocksDB commits. This is a measurement command, not another copy
of the correctness gate. It requires Linux, Python 3.11 and the workspace build
toolchain. Keep other builds and heavy workloads off the host during sampling.
The same entrypoint also measures strict CKB-VM Cell transactions with `--kinds
cells`; these are a separate execution class from host game commands.
`--kinds rewards` selects the existing authorized scarce-transfer policy for
match-completion rewards. It starts with supplied authorized completion events;
it does not execute or certify the upstream game's win/finish condition.

```bash
CARGO_BUILD_JOBS=4 python3 scripts/myelin_workload_measurements.py \
  --out /absolute/path/to/new-measurement-directory
```

The script builds `workloads` once with the normal release profile, records the
exact source inventory, executable digest, compiler, CPU, memory and environment,
and runs the selected cases. Outputs must be new and outside the source tree or
under `target/`. A missing tool, failed case, changed source/executable or exceeded
timeout fails the run. Each completed case keeps raw samples and its own report;
a failed overall run does not erase earlier results.

| Scale | Active sessions | Inputs per session |
| --- | ---: | ---: |
| small | 1 | 32 |
| medium | 4 | 128 |
| large | 8 | 512 |

The board game uses nine legal moves ending in a draw at every scale; larger
scales add concurrent matches rather than continuing to send moves to a finished
board. Inventory buys one potion and then saves revisions; fixed ticks include
seeded movement, disconnect, delayed AI takeover and reconnect events. The input
seed defaults to 17. No generated command is silently dropped or expected to
fail. The default offered rate is one command per millisecond per session, a deliberate
load test, not a claim that the game normally receives that rate. Fixed-tick
logical time still advances by its declared 20 ms simulation step.

Every normal case compares Instant, Interval, Open and explicit manual calls.
Default Interval/Open/manual windows are 5 ms and the batch ceiling is eight commands.
The real `ProductionSchedule` decides automatic triggers. The manual driver
issues explicit calls at its configured cadence; it is not an automatic Never
producer. Shared cases use one OS thread per session and one shared canonical
RocksDB; services cases use separate processes, journals and canonical databases
on this same physical host. Each session starts its input clock after its own
initialization; these are concurrent workloads, not a globally synchronized
arrival trace.
Use `--arrival-us`, `--batch` and `--window-us` to reproduce another explicit
arrival/batch/window configuration; the selection and exact worker plans retain
all three values. Arrival can be zero for an immediate burst, batch is 1–64, and
window is 1–1,000,000 microseconds. Long-history and process-exit drills explicitly
use immediate arrivals and batch one regardless of normal-case settings.

The `cells` workload consumes and recreates one live Cell per transaction with
constant capacity and increasing sequence data. The existing absolute-timelock
RISC-V fixture runs under `VmSemantics::CkbStrict`, resolved from an immutable
embedded code dependency by its CKB data hash (`data2`). Inputs come from the
actual live-cell state. Each transaction is limited to 100,000 cycles; successful
frames must report nonzero cycles. The script checks the supplied `since` field,
not a wall-clock/CKB-chain timestamp or game rule. No lock groups are skipped.
The workload uses scheduled in-memory CellTx inputs, without a command journal,
client ACK or remote admission. It shares the scales, batching, trigger policies,
process layouts, durable commit and archive/replay implementation above.

Before timing production, each Cell session prepares a two-transaction candidate
whose second transaction violates the lock. The VM must reject it with exit code
3 and preserve the canonical head. Normal production then starts from the same
genesis input. Final validation checks the exact live-cell snapshot, remaining
capacity, outpoint, sequence and creation height. Restart and archive replay must
reproduce both the exact head/state and the recorded total VM cycles. Rejection
timing is retained separately and is not mixed into successful transaction tails.

For `rewards`, the scale counts source/inventory **pairs**: 1/4/8 pairs and
32/128/512 rewards per pair. Thus the large case has 16 sessions. Shared layout
uses one pair worker thread per pair and one canonical RocksDB; services layout
runs each pair in its own process and database. The two sessions inside a pair
share one ledger in both layouts. This measures process isolation between pairs,
not a cross-ledger transfer protocol or networked source/target services.

Each completion has a deterministic match nonce, an owner-signed one-unit reward,
and a pinned source/target genesis. The driver commits source Issue, target
Intake, then source Retire using the real `TransferExecutor`. Only source issuance
waits on the selected production trigger; intake/retirement follow immediately
for that batch. Candidate simulation, local PoA signing, synchronous commit and
local outbox ACK are timed independently for each stage. Normal cases acknowledge
these notifications locally without contacting an evidence provider.

Use explicit selections for a shorter experiment:

```bash
python3 scripts/myelin_workload_measurements.py \
  --out /absolute/path/to/new-small-run \
  --scales small --kinds inventory turns ticks --triggers instant \
  --layouts shared --no-drills
```

`--no-drills` omits the additional fault/history cases for the selected kinds.
Use `--faults` to select named faults without repeating unrelated drills;
normal cases still follow the scale/trigger/layout selection. The two options
are mutually exclusive. An explicitly selected fault must apply to the selected
workload kinds. Omitting both options selects all applicable drills.
When ticks are selected, drills include
4,096 one-command fixed-tick blocks, process exit immediately after the first
durable commit, delayed CPU-bound execution in one session, and unknown commit
results followed by original-command/ACK retry. Process exit is a real exit
without Rust destructors. Slow execution is an explicitly injected busy wait;
unknown results are a controlled journal fault, not a simulated network success.

When rewards are selected, drills include cancelling every eleventh reward and
refunding its source lock in both selected layouts, plus a real process exit
after the first durable inventory credit and before source retirement. That exit
uses one pair and batch one. A new process audits both ledgers, recovers the exact
original intake receipt, rejects duplicate credit, retires the existing lock and
finishes the remaining supplied completions. Its reconciliation timing is kept
in `reconciled-credit-<pair>.json`; whole recovery time also includes completing
the remaining workload and verifying/replaying it. This is a controlled process
fault, not a lost network packet or an authority-fencing measurement.

The reward drills also run a slow and an offline evidence provider in each
selected layout, with four pairs and 128 rewards/pair. Each pair has its own
loopback TCP provider process and evidence worker. They use the real RocksDB
outbox and `EvidenceRuntime`, with two locally verified stages: `provider-stored`
and `retrieval-verified`. The provider fsyncs the exact notification payload,
signs a receipt with a fixed development key, and rereads the stored bytes for
the second stage. Signatures bind the message commitment, payload digest/length,
stage and previous receipt. This controlled provider is not a public DA network
or a quorum certificate. These notification stages are asynchronous side effects;
the reward policy does not require DA evidence before allowing intake.

Only pair zero is impaired. Slow mode adds 10 ms to each provider request.
Offline mode exits the provider process with code 77 after its first durable
write but before replying. The worker verifies that evidence and the pending
message remain unchanged, observes `ConnectionRefused`, waits 500 ms and restarts
the provider against its original blob directory. The retry must reuse that exact
blob. Canonical reward heads must advance during the outage. One bounded request
runs at a time per pair, with a 250 ms network timeout and a 60 s worker deadline;
no canonical transaction/global lock is held while contacting the provider.
Production completion rechecks the durable outbox before terminating the worker.
Every stage must complete before the case succeeds.

The `disk-quota` reward drill requires Linux FUSE, `mke2fs`, `fuse2fs` and
`fusermount`. It mounts a fresh 32 MiB file-backed ext4 image for the canonical
database and fills only that owned volume. Real reward commands continue through
any preallocated WAL space until RocksDB itself reports `ENOSPC`. Every successful
receipt is retained; a failed write must leave the complete canonical key inventory
unchanged. In shared layout every neighboring pair must also receive the write
error. Independent services wait for the actual error marker, then finish their
entire workload before the filler is removed. This measures finite filesystem
capacity, not a user quota policy or loss of the host disk.

After releasing the filler, the driver unmounts and remounts the same image.
A new process checks every acknowledged receipt, reconciles the failed command
using its original identity, rejects a duplicate and completes the remaining
rewards. Ordinary reopen and full archive replay follow. Before cold replay,
both the original database and its backing image are deleted. FUSE daemon RSS is
included in resource samples. `quota-fault.json`, `quota-commits.jsonl`,
`quota-reconciled.json` and `quota-filesystem.json` retain the actual errors,
successful prefix, recovery outcome, tool identities, mount data and cleanup.
Storage totals before release include the separately reported filler size;
`storage_after_quota_recovery` excludes it. Post-restart timestamps use a fresh
origin and are not merged into normal end-to-end latency distributions.

## Interpreting the artifacts

Each worker retains `commands-<session>.jsonl` and `batches-<session>.jsonl`:

- Command timestamps use a monotonic clock relative to that session. They record
  scheduled arrival, enqueue start/end, batch simulation completion, durable
  canonical commit and durable local result ACK. Queue overload remains visible
  in input-to-admission latency. Earlier commands in a batch share the time when
  the whole batch finishes simulation.
- Batch phases separate reservation, candidate preparation, actual execution,
  outer executor cloning, snapshot copying, local PoA signing, proof verification
  plus synchronous commit, and result reconciliation/ACK. Preparation includes
  execution and must not be added to its nested costs. Execution includes the
  adapter's own internal cloning, authentication and state encoding. This does
  not isolate raw device fsync latency from the rest of the store commit.
- Result timing excludes JSON log writes; end-to-end command timing still
  includes instrumentation overhead between commands. Process throughput also
  includes initialization and final result validation. Reports separately retain
  per-session production and validation times.

Cell workers instead retain `transactions-<session>.jsonl`, with scheduled,
in-memory admitted, simulation-finished and committed timestamps, plus the actual
transaction ID. Their batch rows omit journal/ACK phases and include strict VM
cycles. Both execution classes share the clone/execute/snapshot instrumentation.
The current report schema is `myelin-workload-measurements-v2`, with `inputs`,
`input_kind`, `batch_inputs` and explicit aggregate/per-batch `vm_cycles`; there is
no legacy report reader or conversion path. Historical v1 reports below remain
evidence for their original source snapshots.

Reward workers retain `rewards-<pair>.jsonl`: admission, source simulation/commit,
optional inventory credit, and final retirement/refund timestamps, with exact
match and handoff IDs. `input_to_inventory_credit_ns` excludes refunded rewards;
`input_to_terminal_ns` includes both outcomes. The summary reports outcome counts
and per-stage phase distributions. Each reward commits three canonical commands:
Issue/Intake/Retire or Issue/Cancel/Reclaim. Resumed work after process death uses
separate `recovered-*` logs and a fresh arrival origin; those timings are not mixed
with normal samples from the exited process.

Each reward row also names its three exact outbox message IDs. Evidence cases
join these with `evidence-<pair>.jsonl`, reporting input-to-inventory-notification
and input-to-all-three-notifications separately from canonical credit/retirement.
Each attempt records provider proof collection, local verification and residual
local pipeline work (record lookup/validation, durable stage update and final
ACK). That residual is not an isolated device-fsync timing. Failed attempts are
retained. `provider-<pair>/requests.jsonl` records actual storage/retrieval and
idempotent reuse; `provider-restart-<pair>.json` retains exit/refusal, outage timing
and canonical heads before/after. The normal local ACK timing is zero in these
cases; publication work is measured in the separate evidence worker.

The case summary computes nearest-rank p50/p95/p99 from retained raw observations,
without discarding startup samples or failures. Nine board moves or 32 small-run
inputs are small samples; their percentiles are observations, not estimated
production tail guarantees. Each session is reported separately so a slow worker
cannot be hidden in a pooled average.

Every phase is a new subprocess. Linux `wait4` provides that child's CPU times
and lifetime peak RSS; the sampler also records RSS every 10 ms and the observed
sum for concurrently running services. Sampling includes live descendant
processes, including providers spawned from worker threads. Sampled peaks can miss shorter spikes.
Lifetime peaks include process startup/inherited memory. File counts, logical
lengths and allocated file blocks are recorded separately for canonical storage,
journals and archives. Both execution classes also fingerprint all database keys.
Allocated file bytes are a filesystem observation, not total device writes or
write amplification; retained WAL/SST files are captured after orderly process
exit. `file_types` separates numeric RocksDB `.log` WALs, `.sst` tables and other
files, retaining both logical length and allocated blocks for each class.

Normal production reports also retain `storage.record_storage`: per-session
counts and current encoded value bytes for blocks, exact stored command receipts
and any consensus WAL value. This uses public store reads and the same current
codecs used for persistence; it excludes keys, compression and RocksDB framing.
The timed read/encode scan runs after producers join, outside command-latency
samples, but contributes to whole-process duration. The consensus WAL value is
distinct from RocksDB's physical write-ahead-log files. Cell transactions in this
workload have no host command receipts. Archive reports retain actual manifest
file bytes, encoded blob-index/session-catalog field sizes and counts of dependency,
block and handoff references. These describe the exported dependency index rather
than estimating its size from a source-level formula.

## Recovery and fault boundaries

After a successful workload, a fresh process audits the complete stored history,
restores the executor, reconciles original journal entries and reexecutes from
genesis. The driver compares exact head-record commitments and game states with
the original result. First/last original-command retries must retain the same
receipt/result; all originals and committed identities are checked. The result
is also compared with an unbatched fold of the same rules and input trace. That
is a storage/batching/retry check, not an independent implementation of game rules.

Reward checks verify every original command receipt and completion nonce, exact
source/inventory balances and versions, zero remaining locks, and the matching
Retired/Reclaimed handoff terminal and inventory receipt. Source available plus
inventory available must equal the initial reward budget; refunds cannot also
have an inventory receipt. The same checks run after database reopen and after
the original databases are deleted. Exports retain each session's own profile,
including the inventory's source allowlist, before full application replay.
Evidence cases also check both signed stages for every original outbox message
after database reopen and archive recovery. Replay verifies the archived bytes
locally; it does not contact the provider. Provider processes have already exited,
and their raw logs/blob files remain measurement artifacts outside the bundle.

The export phase retains actual historical records, the exact executable and
current typed parameter packages. Source/toolchain reproduction and a sandbox
runner are **not** claimed by this measurement capsule. The driver removes only
the newly created workload's canonical database and journals, then invokes the
same pinned executable in another process to audit and replay the archive. The
original service and databases are absent. OS page cache is not purged: call this
fresh-process archive recovery, not a cold-device measurement.

Archive dependency paths include consecutive blocks. The runner explicitly sets
the depth allowance to the selected command count plus eight metadata nodes;
other blob/node/edge/byte limits remain enforced. The generic default depth of
1,024 is too small for the 4,096-block drill. The iterative bundle traversal
accepts explicit depth allowances up to its absolute node ceiling (262,144).

Archive work starts with 100,000 steps, 64 GiB of conservative accounted I/O and
a 30-second slice. A final publication check must fit one grant; every opaque
record read reserves the codec's 128 MiB ceiling, not its actual on-disk length.
The measurement driver uses the cursor's explicit `next_operation` to grant the
needed finite step/byte reservation and retains every grant/pause in
`archive-work.jsonl` or `cold-work.jsonl`. This does not enlarge blob, snapshot or
resident-memory limits. For example, 512 rewards across four pairs require 528
final reads (512 claims plus eight heads/checkpoints pairs), or 66 GiB accounted
I/O. The former fixed 64 GiB driver could never finish that final sweep. The
exporter now parks before that sweep without repeatedly reading a prefix, and
the driver supplies the declared requirement. Deadline and cancellation checks
still invalidate and restart any interrupted final sweep.

For these local drills, observed RPO is the number of acknowledged/committed
commands that cannot be recovered; the run fails on any loss or changed result.
The process-exit case must recover exactly its one committed command, retaining
later queued originals. Recovery phase duration measures invocation through
verified result and full replay; nested timings distinguish database opening,
history audit, executor attachment, journal reconciliation and application replay.
This defines the drill's observed recovery time, not a deployment RTO promise.
The reference deployment objectives below distinguish the measured recovery phases
from an end-to-end outage target. Qualify them on the intended host, storage and
failure configuration before making a service commitment.

This runner uses public development keys. Host games report zero VM cycles;
the strict Cell workload reports the actual nonzero cycles separately.
Public-network DA latency and remote consensus quorum proof collection are outside
the controlled provider's scope. Platform-index write outage is covered by the
downstream Chess online flow described below; this runner does not duplicate it.
Physical multi-host timings are not claimed. Cross-worker fencing is covered
by the existing authority/network drills mapped in the
[P1 acceptance record](../plans/p1-delivery.md#requirement-mapping--writer-fencing-and-migration),
not by an additional measurement gate. The report lists its exclusions and
marks Teeworlds explicitly skipped; use the existing external integration lane
for its acceptance. A local matrix pass does not complete those P1 requirements.
The additional `game logic inside strict CKB-VM` exclusion describes the Cell
fixture's scope; #20 requires real strict VM measurement, not a new VM game port.

## Storage growth observations — 2026-11-07

The existing runner passed 30 Instant cases: all five workload kinds at three
scales in both layouts. It covered 37,354 inputs, 46,634 canonical host command
receipts, 10,026 blocks and 9,985,280 strict CellTx VM cycles. All cases retained
exact heads/results after reopen and archive replay with original databases
removed. Every reported block/receipt count matched its production trace; every
dependency count and manifest length matched the retained archive, and physical
file-class totals matched their database/journal/archive totals.

Large shared-layout observations, in KiB (1,024 bytes):

| Workload | Encoded block values | Encoded receipt values | Retained RocksDB WAL files | Archive manifest | Dependency references |
| --- | ---: | ---: | ---: | ---: | ---: |
| Inventory, 4,096 inputs | 3,241.52 | 835.00 | 7,217.78 | 189.00 | 1,069 |
| Turns, 72 inputs | 68.54 | 14.62 | 151.94 | 31.87 | 73 |
| Fixed ticks, 4,096 inputs | 4,009.35 | 835.00 | 8,832.66 | 189.35 | 1,071 |
| Strict CellTx, 4,096 inputs | 2,956.13 | 0 | 3,570.34 | 398.19 | 2,395 |
| Rewards, 4,096 rewards / 12,288 commands | 14,870.74 | 2,506.11 | 51,970.68 | 2,452.06 | 16,529 |

For shared rewards, small/medium/large scales used 32/512/4,096 rewards.
Encoded block values grew from 125.44 to 1,874.11 to 14,870.74 KiB; receipts
from 19.50 to 312.50 to 2,506.11 KiB; retained WAL files from 399.76 to
7,530.85 to 51,970.68 KiB. Blob-index JSON grew from 17.67 to 229.60 to
1,772.61 KiB, with 155/2,121/16,529 dependency references. These include required
source/terminal causal links, not only block counts. WAL bytes are retained file
lengths at the observed phase, not cumulative writes or a write-amplification
factor. Local development PoA produced no separate `ConsensusWal` store values;
this is not a Tendermint WAL measurement. Scheduling can change batch grouping,
so the fixed inputs do not imply identical block/file sizes between runs.

Report: `/home/arthur/.cache/myelin-p1-storage-growth-v1/report.json`, SHA-256
`e9afe7d70e83117cfdb136550f5a87de42adc183d400ef3a640115d26a6effd2`.
Source: `e86ed3281248ac61a92fb500409a52c0873b14c3cf2ed057b4c6b224fadb3fba`.
Executable: `2fc363718f9d26e469c9b2f9884235fb1bde644e4159843e99ec8f03c4eb32db`.
All 22,523 artifacts were independently rehashed. The per-case extracted totals
are retained in `myelin-p1-completion-audit-v1/storage-growth-verification.json`
alongside the reports. Reproduce with:

```bash
CARGO_BUILD_JOBS=4 python3 scripts/myelin_workload_measurements.py \
  --out /absolute/path/to/new-storage-run \
  --kinds inventory turns ticks cells rewards --scales small medium large \
  --triggers instant --layouts shared services --no-drills
```

The measurement changes add no storage format, decoder, production-library API,
test suite or gate. They reuse the original producer/recovery/archive paths.

## Platform index observations — 2026-11-06

The existing Chess `tests/chess-online-flow.mjs` replaces one repeated lost-reply
scenario with an actual SQLite write outage. After native room creation commits,
a second SQLite connection takes `BEGIN IMMEDIATE` on the room-discovery database.
The host's projection fails with `SQLITE_BUSY` (code 5) and HTTP 503. Its client
keeps the complete original request. While that writer lock remains held, another
account completes four native game moves through the same HTTP host and verifies
its signed export. This is a write outage of the routing/discovery index; SQLite
reads and independent canonical RocksDB databases remain available.

Observed lock duration was 381.11 ms. The create HTTP request returned 503 in
49.83 ms, including its successful native commit; only 0.845 ms elapsed between
acquiring the SQLite lock and observing that failure. Independent game commands
took 45.97–47.03 ms each and advanced their canonical action count from zero to
four. After lock release, killing/restarting both host and native service and
recovering the original room command took 83.29 ms. The receipt and head matched
the pre-projection committed result exactly (one action), the seat projection was
repaired, and all four independent receipts and their head survived unchanged.
The rest of the existing admission, competing-seat, lost-reply, restart and
offline replay flow also passed; room and independent exports replayed eight and
four frames respectively. These are one-run phase observations, not availability
or throughput targets. SQLite membership projections never grant canonical rights.

Report: `/home/arthur/.cache/myelin-p1-platform-index-v1/report.json`, SHA-256
`73f51ceb1fa3b5073c9a1202f235629cafbdbe8bffaebd24d6dab25416be12fa`.
Chess source: `06b29febf990631c29029f69ad6ebf7a9aefbb98a34e419486b6d9896a921d2f`.
Native executable: `580e6a6979038de0cd6efc38112903f47763baeda9dcd974cb2c82fcc2dc1b67`,
the exact binary extracted from the previously verified independent Chess release.
Only the online-flow script differs from that release's game source; host and
native application code are unchanged. Node 22.23.1 ran the complete flow in
4.022 s. The report binds before/after source inventories, executable, command,
raw stdout/stderr, original requests/receipts, signed exports and phase timings.
From the independent Chess repository, after building its native binary:

```bash
CHESS_BINARY="$PWD/target/release/myelin-chess" \
  node tests/chess-online-flow.mjs > chess-online-flow.json
```

This reuses the existing integration entrypoint and removes a duplicated
creation-response-loss injection. Join and move response-loss coverage remain.
It adds no platform service, test suite, production fallback or acceptance gate.

## Filesystem capacity observations — 2026-11-06

Four cases passed: two small normal cases and both medium quota layouts,
covering 1,088 rewards / 3,264 commands with zero VM cycles. The 32 MiB images
provided 31,448,064 filesystem bytes. Shared mode acknowledged twelve commands
before the next Issue failed; services mode acknowledged Issue and Intake before
Retire failed. All four shared pairs received real RocksDB `ENOSPC` errors in
0.098–0.294 ms; the isolated pair's failed Retire took 0.426 ms. The three healthy
services completed 384 rewards / 1,152 commands while the filler remained present.
Their inventory-credit p99 was 416.79 ms under this finite 1 ms arrival burst.

| Layout | Exact failed-command reconciliation | Resume and complete remaining rewards | Full archive replay after original image removal |
| --- | ---: | ---: | ---: |
| Shared | 67.35 ms | 2.718 s | 1.219 s |
| Services | 35.59 ms | 0.695 s | 0.365 s |

Both failed commands were absent after reopening and were retried once. All
twelve/two prior receipts survived exactly, duplicate operations were rejected,
and final heads, balances and handoff conservation matched after ordinary and
archive recovery: observed acknowledged-command loss was zero. These durations
exclude operator detection and disk provisioning; they are not deployment RTOs.
Aggregate sampled RSS including FUSE peaked at 103.40 MiB across shared phases
and 277.15 MiB across services phases. Both mounts and their daemon processes
were removed cleanly; cold replay had no original image or canonical directory.

Report: `/home/arthur/.cache/myelin-p1-quota-measurements-v3/report.json`, SHA-256
`98c84cc1969524320bb043f0d6697dedaabab31bd5b207069fc42f052f7fb5df`.
Source: `1765b649524dc7b815328bb573af8106c3e4057d6a206317227edce2088334d9`.
Executable: `402a3946bfeb5dcca59c3e8101d52b095a3a6e0c12901d37ebf3ab8f6bb89d45`.
All 1,916 artifact identities were independently checked. Reproduce with:

```bash
CARGO_BUILD_JOBS=4 python3 scripts/myelin_workload_measurements.py \
  --out /absolute/path/to/new-quota-run \
  --kinds rewards --scales small --triggers instant --layouts shared services \
  --faults disk-quota
```

The first two quota reports remain failed under `myelin-p1-quota-measurements-v1`
and `-v2` in the same evidence directory. The first incorrectly assumed that a
full filesystem makes the next WAL append fail; preallocated WAL space allowed
valid commits. The second incorrectly required zero available blocks after
RocksDB closed, when closing released 1 KiB. The current drill uses actual write
failure as its fault evidence and records post-close space without that redundant
gate. It does not disable WAL allocation or substitute injected storage errors.

The same source/executable also passed nine existing reward/provider regressions
and eight inventory/turn/tick/CellTx regressions. Their reports are
`/home/arthur/.cache/myelin-p1-quota-reward-regression-v1/report.json` (SHA-256
`f7e3c7bc19bc544ef2be60e4e4ad2ab70de642ef03a2ba3a2dd9bd3d8f8ea4de`,
17,974 checked artifacts) and
`/home/arthur/.cache/myelin-p1-quota-class-regression-v1/report.json` (SHA-256
`36ae7d5cadc40a6f58e580c26d74c721c7116b2a813e98cc2d1ec60a0d910234`,
433 checked artifacts). The affected package's 28 Rust tests, strict Clippy and
eleven existing Python checks passed. This extends the existing measurement
entrypoint; no correctness lane or parallel test suite was added.

## Evidence provider observations — 2026-11-06

The current provider campaign passed nine cases: two small normal cases, two
cancel/refund cases, post-credit process exit, and slow/offline providers in both
layouts. It covers 3,168 rewards and 9,504 canonical commands. The four provider
cases cover 2,048 rewards, 6,144 distinct outbox messages, 12,288 accepted stages
and two intentionally failed requests. Both offline cases reused the original
durable blob after process exit. During the measured 502.86/502.50 ms outages,
source/inventory input positions advanced from 1/1 and 2/1 respectively to
256/128. Full reward conservation and every signed evidence chain passed after
restart and after original database removal/archive replay.

Report: `/home/arthur/.cache/myelin-p1-evidence-measurements-v2/report.json`, SHA-256
`9dee83cf838b4e774e7d292877e90488c557b8fea224802141501b9053a14384`.
Source tree: `95849e92c4f6b79806043d9d66a61fef7e48cd53b7cb53d6ff83a85e1b5b2e0e`.
Executable: `e82fdaf836991cc67a792ee4aeb931440612287eb72c2059f7bb277b0248d660`.
All 17,965 artifacts were independently rehashed. Reproduce on Linux with:

```bash
CARGO_BUILD_JOBS=4 python3 scripts/myelin_workload_measurements.py \
  --out /absolute/path/to/new-provider-run \
  --kinds rewards --scales small --triggers instant --layouts shared services \
  --faults cancel-refund reward-process-exit evidence-slow evidence-offline
```

These selected drills include the four medium provider cases in addition to the
selected normal cases. Results at 1 ms arrivals and batch ceiling eight:

| Provider fault / layout | Canonical inventory credit p99 | Pair 0 input → all notification evidence p99 | All evidence workers finished |
| --- | ---: | ---: | ---: |
| Slow / shared | 111.42 ms | 9,839.54 ms | 9.995 s |
| Slow / services | 113.10 ms | 9,843.53 ms | 9.998 s |
| Offline / shared | 277.53 ms | 3,522.28 ms | 3.655 s |
| Offline / services | 102.99 ms | 3,802.60 ms | 3.935 s |

Slow pair zero's proof-collection p95 was 13.12/13.39 ms; the healthy pairs were
3.28–3.55 ms. Healthy pairs completed publication while the slow provider still
had work; their input-to-all-evidence p99 was 2.81–2.89 s. Both layouts therefore
kept canonical work and unrelated providers progressing, while evidence still
accumulated a backlog under this short high-rate burst. Shared-mode aggregate
sampled RSS, including providers, was 126–127 MiB; separate services used about
298 MiB. These one-run observations do not establish a layout speedup.

For this measured provider configuration, use separate evidence workers per pair
and bound an admitted burst to four pairs × 128 rewards (384 notifications/pair).
A provisional qualification target is to drain that entire burst within 15 s,
including the injected 10 ms provider delay or 500 ms provider outage, while
recovering all original receipts and preserving reward conservation. This is a
burst/recovery target derived from the observed maximum of about 10 s, not a
sustainable arrival-rate claim. The workload's finite plan is not a production
admission controller. The earlier 100 rewards/s/pair credit result below measures
local notification ACKs and must not be treated as external publication capacity.

The first provider campaign also passed, before a review tightened the worker's
completion race and removed repeated adapter setup. Its report remains under
`/home/arthur/.cache/myelin-p1-evidence-measurements-v1/`; only v2 supports the
current observations. The affected package's 28 Rust tests, strict Clippy and
the existing eleven Python checks passed. Eight small Instant inventory/turn/tick/
CellTx regressions also passed on the v2 source/executable, with 146 host commands,
64 Cell transactions and 68,864 VM cycles. Their report at
`/home/arthur/.cache/myelin-p1-evidence-regression-v1/report.json` has SHA-256
`ac0056f1f7a701f3138b6e9fb3bfd5c9ee4fdb41d0df967c0e847d6b3350daa9`;
all 434 artifacts were rehashed. This extends the existing workload entrypoint;
it adds no acceptance lane or duplicate test suite.

## Reward observations — 2026-11-06

The reward matrix passed all 27 cases: three scales × four triggers × two layouts,
cancel/refund in both layouts, and process exit after durable inventory credit.
It committed 38,176 rewards as 114,528 commands: 38,088 inventory credits and 88
refunds, with zero VM cycles. All original command receipts, balances, terminal
handoffs and exact heads survived restart and replay after database removal.
Each cancellation case credited 468 rewards and refunded 44. The process-exit
case recovered the original intake receipt, rejected duplicate credit without
changing the inventory head, and retired the original source lock. That one
reconciliation took 2.261 ms; the recovery invocation including the remaining
31 rewards and audit/replay took 0.276 s. This is a local process fault, not a
network or independent-ledger migration drill.

The retained report is
`/home/arthur/.cache/myelin-p1-reward-measurements-v2/report.json`, SHA-256
`e52ce6a98d4fc8512c7d330a500f0fa80c4e928736665990c723d0ae769bc04f`.
It binds source tree `97305c581a96ac642137f460bd66db8c37ea9eba79bd6edad422d483eda9acd9`
and executable `d1887c619e9b5b76b53a85af982cc6f3a84455af264ed537064c0f712ac3ea10`.
All 56,418 artifact identities and 226 session-specific archived profile blobs
were independently checked. Reproduce with:

```bash
CARGO_BUILD_JOBS=4 python3 scripts/myelin_workload_measurements.py \
  --out /absolute/path/to/new-reward-run \
  --kinds rewards --scales small medium large \
  --triggers instant interval open manual --layouts shared services \
  --faults cancel-refund reward-process-exit
```

At the default 1,000 rewards/s/pair, the large case has eight pairs and develops
a backlog under every trigger. Input-to-inventory-credit p95 in milliseconds:

| Trigger | Shared process | Separate processes |
| --- | ---: | ---: |
| Instant | 2,100.78 | 3,053.10 |
| Interval 5 ms | 2,282.49 | 2,273.28 |
| Open 5 ms | 2,153.21 | 2,297.49 |
| Manual 5 ms | 2,175.22 | 2,342.61 |

Changing the trigger alone did not absorb this offered load. A second experiment
kept eight pairs, 512 rewards/pair, Instant and a batch ceiling of eight, but used
10 ms fixed arrivals: 100 rewards/s/pair, 800/s in aggregate. Both cases passed,
with 8,192 rewards, 24,576 commands, zero VM cycles and exact restart/archive
replay. Its source tree is
`cdb8d2df458232d3510717666b7e498ccb230a4d7253f6e717bd7509363cab37`;
only the configurable runner parameters and their documentation changed from
the matrix source. The executable is identical. The report at
`/home/arthur/.cache/myelin-p1-reward-capacity-v1/report.json` has SHA-256
`80e486d08cd360e638ec5e86e31ce768841926c8fa2714d3f3fcd7003c871778`;
all 29,840 artifacts and 32 archived profile blobs were checked.

```bash
CARGO_BUILD_JOBS=4 python3 scripts/myelin_workload_measurements.py \
  --out /absolute/path/to/new-reward-capacity-run \
  --kinds rewards --scales large --triggers instant --layouts shared services \
  --arrival-us 10000 --batch 8 --window-us 5000 --no-drills
```

| Observation | Shared process | Separate processes |
| --- | ---: | ---: |
| Input → inventory credit p95 | 20.52 ms | 19.84 ms |
| Input → inventory credit p99 | 22.25 ms | 22.12 ms |
| Worst individual pair's credit p99 | 22.78 ms | 22.78 ms |
| Input → retirement p99 | 27.73 ms | 27.97 ms |
| Sampled aggregate production RSS | 312.91 MiB | 975.77 MiB |
| Full archive recovery group | 11.438 s | 2.605 s |

Use Instant, a batch ceiling of eight and 100 rewards/s/pair across eight pairs
as a starting configuration for this reference host. The observed batches
contained only one or two commands at this arrival rate. A provisional next-host
qualification target is p99 input-to-durable-inventory-credit below 50 ms for
**each pair**, with exact receipt recovery and zero duplicate/lost rewards.
This target leaves margin above the observed 27.25 ms maximum credit latency;
it is not a production SLA or a sustained capacity limit. Qualify longer histories,
bursty input and the actual host/storage before adopting it. These measurements
use the same reference desktop and seed 17 as the Cell runs; they exclude game
judgment, RPC and external evidence delivery. Each separate process owns a whole
source/inventory pair with one shared ledger.

The initial reward campaign at
`/home/arthur/.cache/myelin-p1-reward-measurements-v1/report.json` remains failed:
it was interrupted after eight successful small cases when the medium export
repeatedly exhausted its final-publication grant. The reservation fix described
above passed the existing publication/source-change regression and the complete
core profile. The corrected matrix retained finite grants, including the large
shared sweep's 516 GiB of conservative accounted I/O; that number is neither
physical bytes read nor memory allocated.

The shared execution instrumentation and archive changes also passed eight
small Instant cases across inventory, turns, ticks and strict CellTx in both
layouts: 146 host commands, 64 Cell transactions and 68,864 actual VM cycles.
The report `/home/arthur/.cache/myelin-p1-reward-regression-v1/report.json` has
SHA-256 `0fa3776e54d2565cc58c5a9b4488234b964819a5a32220e76e8c9f78ad23a95b`;
all 435 artifacts were checked. It uses the reward matrix's source and executable.
The existing entrypoint selects these workloads and drills; no extra gate or
parallel test suite was added.

## Strict CellTx observations — 2026-11-06

The 24 Cell cases passed on source tree
`e24e574e362f52286ff2c389351afbe003fa63d8b64c906e526c900662a77fd6`:
three scales × four triggers × two process layouts. The report at
`/home/arthur/.cache/myelin-p1-strict-cell-measurements-v1/report.json`
has SHA-256 `c7704027668ca636df90d20af3ef1b292ee87093a849d919c64d3a56e7a663f9`.
All 11,751 artifact identities were independently rehashed. The exact native
executable has SHA-256
`f57a15d72f3dcb3aa54a3962bf15a5527b49e294102f038ea3975e7bcdbe48f2`
and is retained inside each case's archive.

```bash
CARGO_BUILD_JOBS=4 python3 scripts/myelin_workload_measurements.py \
  --out /absolute/path/to/new-cell-run \
  --kinds cells --scales small medium large \
  --triggers instant interval open manual --layouts shared services --no-drills
```

The run committed 37,120 transactions in 9,342 blocks: 39,941,120 actual VM cycles,
or 1,076 cycles per transaction for this small timelock fixture. All 104 sessions
passed the rejected-candidate probe. Restart and archive replay reproduced the
original head, full live-cell state and cycle total; the original RocksDB was
removed before archive replay. There were no production-process crashes in this
selection. Machine/toolchain/seed were the reference i9-13900H, 20 logical CPUs,
Linux x86_64, Rust 1.92.0 and seed 17. These are one-run observations on a desktop
host, not controlled repeated tail estimates or production throughput limits.

Large cases have eight concurrent sessions with 512 Cell transactions each.
Input-to-durable-commit p95 in milliseconds:

| Trigger | Shared process | Separate processes | Shared blocks | Separate blocks |
| --- | ---: | ---: | ---: | ---: |
| Instant | 3.24 | 14.90 | 2,655 | 893 |
| Interval 5 ms | 6.95 | 17.35 | 808 | 608 |
| Open 5 ms | 8.70 | 8.65 | 593 | 594 |
| Manual 5 ms | 6.78 | 6.54 | 808 | 808 |

Across large shared cases, sampled production RSS was 81–85 MiB; the sum across
eight separate processes was 483–484 MiB. Large shared Instant stored 2,696
canonical key/value entries and about 6.51 MB of database files; Open stored 626
entries and about 2.59 MB. Its increased waiting time accompanies fewer commits.
Shared Instant full archive recovery took 1.131 s; independent services completed
their parallel recovery group in 0.160 s. They also produced different block
counts, so this is not an equal-history speedup. Separate archives repeat the
14.6 MB executable per service; their larger total size is not ledger growth.

For this cheap Cell script, Instant is a useful low-latency baseline in one
process; a 5 ms Open or explicit manual batch is a candidate when reducing commit
count matters. Separate processes did not universally improve latency and used
substantially more memory. Keep the measured batch-eight, 100,000-cycle-per-tx
ceiling as an experiment configuration; rerun real scripts with their actual
cycle/data sizes before choosing a production capacity budget. None of these
timings predicts the host games' journal/ACK latency or a VM game's performance.

The common archive/instrumentation refactor also passed six small Instant cases:
inventory, turns and ticks in both layouts, 146 commands and zero VM cycles.
The report at
`/home/arthur/.cache/myelin-p1-workload-host-regression-v3/report.json`
has SHA-256 `357148d419ac605cddf2d1c6fee97adaf30168855607923f5d8ddc7d717a8d29`;
all 271 artifacts were rehashed. It uses the same source/executable as the Cell
matrix. The affected package's 28 Rust tests, strict all-target/all-feature
Clippy, and the existing 11 Python checks passed. No new acceptance lane,
compatibility reader, or duplicate test suite was added.

## Reference observations — 2026-11-06

The first run completed all 72 normal cases, then failed the long-history export
because its default dependency-depth allowance was insufficient. That overall
report remains **failed** at
`/home/arthur/.cache/myelin-p1-workload-matrix-v1/report.json`, SHA-256
`c9bdad032cb1fcddf3606b7307942ba2bdd0e8f165bcb15d553bbe6720c00b77`.
The normal-case observations below belong to source tree
`1f2ac07f197ef267859841bfcbe23d854bb2beebaa648303d877002292e769c6`.
Each completed normal case passed restart, archive export, removal of its original
databases and exact archive replay. Its 20,592 retained artifact identities were
rechecked; the later failure is not hidden by those successful cases.

After correcting the explicit depth allowance and removing the unrelated duplicate
serialization interfaces/tests, the two small tick cases and all six additional
history/fault cases passed at
`/home/arthur/.cache/myelin-p1-workload-drills-v2/report.json`, SHA-256
`9d8e78e0071d0f06acb4612daaa506b1e88f4548aa1e723930e62601819b143e`.
This report binds source tree
`9db03a1aa1bb543899f89ab54109786980d8af8ca841ea30cc6d9678cc840099`
and 6,947 rechecked artifacts. The normal matrix was not repeated on this newer
snapshot. Reproduce that focused selection with:

```bash
CARGO_BUILD_JOBS=4 python3 scripts/myelin_workload_measurements.py \
  --out /absolute/path/to/new-drill-run \
  --scales small --kinds ticks --triggers instant --layouts shared services
```

Both runs used Linux x86_64, an i9-13900H with 20 available logical CPUs, about
31.1 GiB RAM, Rust 1.92.0, the ordinary release profile and seed 17. These are
single runs on one host. Case order was fixed, OS cache was not purged, and the
small samples do not establish stable tail distributions or an optimal trigger.

Input-to-durable-local-response p95 in milliseconds, shared-process normal cases:

| Game | Scale | Instant | Interval 5 ms | Open 5 ms | Manual 5 ms |
| --- | --- | ---: | ---: | ---: | ---: |
| Inventory | small | 56.11 | 58.68 | 98.64 | 59.11 |
| Inventory | medium | 567.71 | 566.75 | 579.36 | 574.77 |
| Inventory | large | 13231.08 | 15268.07 | 13869.46 | 14631.44 |
| Turns | small | 13.07 | 16.58 | 22.97 | 17.97 |
| Turns | medium | 23.43 | 33.94 | 37.01 | 30.85 |
| Turns | large | 88.91 | 41.79 | 47.21 | 113.82 |
| Ticks | small | 58.40 | 63.85 | 60.82 | 57.53 |
| Ticks | medium | 607.50 | 634.22 | 668.91 | 804.05 |
| Ticks | large | 16779.63 | 15987.48 | 16714.92 | 15003.91 |

For large tick/Instant cases, separate processes reduced observed p95 from
16.78 s to 13.33 s, while sampled aggregate RSS rose from 890.4 MiB to 1268.9 MiB.
Neither layout kept up with the offered 8,000 commands/s. In the shared case,
per-batch reconciliation/ACK p95 was 318.67 ms, versus 18.65 ms for proof
verification plus synchronous canonical commit and 2.12 ms for execution.
Journal updates currently clone, validate and persist the complete bounded
journal. These observations identify journal/reconciliation work as the first
profiling target; they do not isolate raw fsync cost or prove causality for every
latency spike.

Settings suggested for the next deployment experiment:

- **Turn-based play:** start with Instant for isolated low-rate moves. Compare a
  5 ms Interval under bursts: eight simultaneous matches measured 41.79 ms p95
  with Interval versus 88.91 ms with Instant, while one match favored Instant.
  Keep batch eight as the measured baseline, not a proven optimum.
- **Inventory saves:** Instant is a reasonable baseline at low concurrency, but
  changing the window did not resolve the large-case backlog. Keep admission
  bounded and establish a rate limit from repeated target-host measurements
  before increasing sessions or journal history. A longer batching window alone
  is not a demonstrated capacity improvement.
- **Fixed ticks:** keep the declared 20 ms simulation clock separate from commit
  timing. None of these configurations supports the offered one-input/ms/session
  stress rate with realtime response. Measure the intended input rate and journal
  lifetime after reducing update costs; moving to separate processes alone is
  insufficient and consumes more aggregate memory.

The corrected long drill retained 4,096 one-command blocks. Production took
458.68 s; fresh-process database recovery plus full replay took 2.45 s; export
took 5.21 s; archive audit/replay after original-database deletion took 2.19 s.
Sampled export RSS peaked at 74.2 MiB. The archive has 4,104 blobs and 23,718,512
logical file bytes including its manifest (31,318,016 allocated file bytes).
The export's roughly 1.10 TB accounted I/O is conservative work-budget accounting,
including record reservations; it is not measured disk traffic. Reopening RocksDB
also changed allocated/logical sizes through its normal maintenance, so the
before/after file sizes are not a retention or garbage-collection saving.

Process exit immediately after canonical commit recovered exactly that one
commit and retained all 32 original commands. Unknown-result drills recovered
40 controlled unknown outcomes in each layout, with 4,096 commands per layout.
The slow-session drills completed all eight sessions and preserved their exact
heads/states. Their 2 ms injected execution delay is not evidence of isolation
under arbitrary CPU load, stalled native code or a failed physical host. These
drills observed no loss of committed commands; they do not themselves establish
an end-to-end deployment RPO/RTO result. Multi-epoch dependency measurements and
reference objectives follow below. The later CellTx, reward, provider, quota and
platform-index campaigns above retain their own evidence and source identities;
none is retroactively attributed to this earlier run.

## Dependencies across authority epochs

The archive suite replaces its single-successor refund case with one parameterized
case covering both scarce-claim terminal paths. Two locks of 25 units cross a
source lineage with a different PoA authority in each successor. The fixed target
consumes one claim; the source cancels the other after the first rotation. The
final source retires the consumed lock and refunds the canceled lock. All epochs
are audited and replayed before export and again after deleting the original
database. Exact head records, state roots and full-range receipt commitments must
match. The final source has 75 available, the target 25, and no locks remain.
Removing a declared intermediate seal dependency must fail even when its bytes
and all independently signed sessions remain in the archive.

This is the existing archive test and core lane, with three rotations by default;
there is no additional gate. Core retains its report under `epoch-dependencies/`
on Linux. For explicit release measurements, use a new output parent:

```bash
measurement_root=/absolute/path/to/new-epoch-run
mkdir "$measurement_root"
for rotations in 2 8 32; do
  MYELIN_DEPENDENCY_EPOCHS="$rotations" \
  MYELIN_DEPENDENCY_EVIDENCE_OUT="$measurement_root/epochs-$rotations" \
  CARGO_BUILD_JOBS=4 cargo test --locked --release \
    -p myelin-session-bundle --features test-fixture --test archives \
    dependency_measurements::multi_epoch_dependencies_replay_without_the_original_store \
    -- --exact --test-threads=1
done
```

Rotation counts are 2–32, with 32 reaching the existing handoff-owner depth limit.
There are `rotations + 1` source sessions and one fixed target. Each grant allows
64 GiB of **accounted** I/O and otherwise uses normal archive budgets; exhaustion
resumes the existing cursor. The fixture does not relax protocol or library
defaults. It measures a deep, short-history lineage separately from the earlier
4,096-block and many-active-session workloads.

The retained release run is
`/home/arthur/.cache/myelin-p1-multi-epoch-v3/report.json`, SHA-256
`df192ebe5edcb5ce29222cd8fc8a8450f8c724a476961d179d8ab524bbd04ba6`,
source tree `98eb01069bc8c065061d27e33fe32c865288471123a0fcb0ce535afe272b83de`.
Its 296 artifact identities and unchanged source inventory were independently
rechecked. `run.py` records the exact build/commands, compiler and machine, and
retains the executable with SHA-256
`0a928ccd04de4230be87dca075f0f480bf04fc75bc10d3b59ab53a7aa8fd9324`.
Each case runs in a fresh process on the same reference host as the earlier matrix.

| Authority rotations | Sessions / blocks | Closed database logical bytes | Archive logical bytes | OS-reported peak RSS (MiB) | Export (ms) | Archive audit (ms) | Restore and replay (ms) |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 2 | 4 / 7 | 95,689 | 14,980,931 | 58.05 | 42.15 | 11.11 | 6.21 |
| 8 | 10 / 13 | 152,449 | 15,047,209 | 57.80 | 64.92 | 20.56 | 17.72 |
| 32 | 34 / 37 | 379,519 | 15,312,673 | 59.01 | 166.25 | 40.10 | 106.52 |

The 32-rotation archive has 178 blobs, 386 dependency edges and depth 72.
Allocated database/archive file bytes are 393,216 / 15,794,176. Archive sizes
include the exact executable, which dominates this small fixture. Raw reports
also retain production/live-audit times, file counts, allocated bytes, work
counters, CPU usage and per-phase RSS samples. Export's roughly 34 GB of I/O
reservations is not physical disk traffic; none of these runs exhausted a grant.

Peak RSS comes from Linux `wait4` for that individual process, while phase RSS
uses `/proc/self/status` once after each phase. Kernel RSS accounting is
approximate; those samples can miss spikes and are not per-phase peaks. Within a
case, archive restoration shares the producing process and OS page cache, after
closing and deleting the original RocksDB. This is trusted compiled host replay
with zero VM cycles and synthetic build packages. It establishes neither a
sandbox/complete-build result nor a fresh-process or cold-device recovery time.
The separate clean-container/offline-build evidence covers those other archive
contracts. These are single observations, not deployment RPO/RTO targets.

## Reference deployment recovery objectives

The initial local deployment objective is **zero lost acknowledged commands and
recovery within 60 seconds after a worker-process failure**, provided its canonical
store, ownership DB and original-command journal survive. This is a configured
operational target, not an availability guarantee supplied by the library.

The measured sizing envelope is the reference Linux x86_64/i9-13900H host above,
one host-executed fixed-tick session with at most 4,096 committed one-command
blocks, and a locally available archive of at most 32 MiB including the exact
interpreter. Run one full recovery at a time; larger histories, simultaneous
recoveries, strict-VM programs, remote archives and other hardware need their own
measurement. The retained reference reported 2.45 s for fresh-process DB recovery
and full replay and 2.19 s for archive audit/replay after DB deletion. The 60 s
objective leaves an operational allowance for detection, startup and readiness;
it is a policy choice informed by those observations, not an extrapolated p99.

| Failure/recovery mode | Recovery point objective | Recovery time objective and endpoint |
| --- | --- | --- |
| Worker process dies; authoritative volumes survive | Zero missing or changed acknowledged command outcomes; retain canonical but unacknowledged outcomes and unresolved originals separately | 60 s from observed termination through epoch/head audit, journal reconciliation and a successful next authorized command; reserve at most 5 s for failure detection |
| Restore locally retained evidence after removal of the producing DB | Every required record up to the independently pinned archive head is present and replays exactly; this says nothing about later unarchived live commands | 60 s from starting recovery to an independently verified root/result with the original executable; write admission still requires the live authority procedure |
| Authority or required data is unavailable, stale, or unverifiable | Never replace missing history, release uncertain economic rights or reset epochs to meet a time target | Fail the attempted recovery visibly and keep writes pending; no automatic takeover RTO is promised for lost authoritative stores/keys |

Measure the full outage with one monotonic timeline: process loss, detection,
recovery invocation, audited checkpoint, original-command reconciliation and first
successful authorized command. Retain each phase as well as total outage duration.
For archive-only verification, record invocation through verified result and label
it separately from service recovery. Existing reports measure recovery phases;
they do not yet demonstrate the complete 60 s deployment objective.

Compute RPO by comparing the exact pre-failure acknowledged command IDs and their
receipt/business results with recovered originals. Count any missing or changed
outcome as a failure even if the service answers `/health`. Check canonical but
unacknowledged commands separately: they must remain discoverable without another
charge. Archive RPO is relative to its pinned head; report the time/command gap
between that head and the last live acknowledgement instead of calling an old
backup zero-loss. Verify scarce-transfer balances and permanent nullifiers wherever
the profile supports them.

Use the existing workload, archive and writer-fencing drills for qualification;
this adds no CI lane or second correctness suite. Record every observed attempt,
including timeouts, cancellations and blocked restores, and remeasure after an
executable, hardware, storage or admitted-size change. The retained measurements
predate the later journal optimization and remain bound to their original source
and executable identities. Passing a newer correctness run does not update their
performance figures. Total-disk-loss durability additionally needs a deployment's
coordinated backup/replication and monotonic-authority design; these local
process/archive targets do not claim that protection.
