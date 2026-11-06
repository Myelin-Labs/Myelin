# Session evidence bundles

`myelin-session-bundle` implements the local P1 #13 reference: bounded content
inventories, explicit trust bootstrap, streaming SessionStore export, mandatory
protocol dependency extraction and read-only per-session finality audit. A
Linux trusted-code hook pins the currently running replayer before restoring and
replaying archived state. The explicit `replay` command runs archived native
programs with pinned runtime libraries in a restricted Linux sandbox. A separate
acceptance script packages the trusted verifier/runtime, restores its OCI image
into an empty image store, and reproduces the result without network or host
bind mounts. Reference source/vendor/toolchain kits now also reproduce the native
replayer and CLI in two clean offline builds. Typed file packages bind complete
kits into session exports without changing historical execution mappings or
raising individual-blob limits. The original transfer and all three command/tick
example programs replay archived histories after their source databases are
removed. The [P1 acceptance record](../plans/p1-delivery.md) maps the retained
clean-container/build reports and their frozen source identities; publication
remains pending. Other applications require their own program/policy acceptance.

## Layout and exact identities

```text
bundle/
  manifest.json
  blobs/
    <64 lowercase hex BLAKE3 digest>
```

The manifest uses format `myelin-session-bundle-v2`. It is canonical compact JSON
in the field order of `BundleManifest` and its nested Rust structs, with no
trailing newline. Import decodes and re-encodes, requiring exact byte equality;
unknown/duplicate fields and alternate encodings reject. Blob identifiers are
strict lowercase 32-byte hex strings, never arbitrary filenames or URLs.
The manifest commitment is BLAKE3 over `myelin:bundle-manifest:v2` followed by
those complete canonical bytes.

The manifest records the Myelin version and required persistence versions,
requested root sessions, sorted unique blob descriptors, and sorted unique
session descriptors. Each session identifies its network/deployment, exact
genesis commitment, complete application profile, genesis/configuration/head/
checkpoint blobs, ordered genesis-prefix block inventory, handoff/evidence
inventory, and execution artifacts. The real genesis bytes must decode under a
supported record version and match the declared session, config commitment and
complete profile. Metadata is not an alternate way to replace the genesis.

A blob has an exact length, digest, one or more roles and a sorted unique list
of content prerequisites. Multiple roles allow a program to serve as its own
replayer without duplicating content. Duplicate roles, unused artifacts, missing
dependencies, cycles and excessive dependency depth reject. The graph represents
content prerequisites, not reverse lookup or ownership links. For example, an
event depends on the block it consumes; the source block must not depend on an
auxiliary terminal index which later points back to the consumer.

`verify-inventory` checks the declared graph and derives the required chunk
edges of recognized typed file packages. `BundleStore::open` and the
`verify` command additionally derive mandatory edges from actual history:
blocks depend on genesis and the preceding block; handoff consumers depend on
source issuance and required evidence; refunds depend on cancellation; successor
genesis depends on the sealed predecessor block. Typed successor receipts also
require their compatibility declaration/vectors or migration-program artifacts;
export retains these automatically from the explicit artifact catalog. Record
envelope 13 carries all current blocks, including typed transitions.
Auxiliary terminal indexes
depend on the issuance and terminal blocks they reference. Recovery verifies the
reverse links without adding graph cycles. Both predecessor and successor must
be present, and every included session undergoes its own finality audit.

Opaque application snapshots can refer to additional app-owned restore data.
The exporter cannot infer these references: the caller must explicitly include
their sessions and artifacts in the export catalog. Successful protocol auditing
does not prove application replay or app-specific dependency completeness.

## Historical program and build identity

`ExecutionArtifacts` identifies program, replayer, rules, complete build-input
package, toolchain package, runtime dependencies, target and runner ABI. Its
commitment is BLAKE3 over `myelin:bundle-execution:v2` followed by the canonical
JSON encoding of that exact object. Runtime dependencies are sorted and unique.
Target and ABI are descriptions until checked against an actual runner.

The program blob's BLAKE3 must equal `ApplicationProfile.program_digest`.
Source-derived identifiers and legacy identity mappings are unsupported. Exact
artifact identity alone does not prove reproducible builds; the separate build-kit
workflow verifies complete inputs and compares independently built binaries.

No native program is loaded or executed by inventory or finality verification.
The explicit isolated runner below supplies filesystem/network restrictions;
the separate live native worker's CPU/memory/deadline enforcement alone remains
insufficient for unknown binaries.

`BundleStore::replay_current_process` is an explicit hook for trusted, already
compiled application code. On Linux it hashes `/proc/self/exe`, checks its exact
size and replayer digest, and only then invokes the caller's restore callback.
It validates the latest snapshot root and replays the complete genesis-to-head
range, returning the normal `RangeReplayReceipt`. An empty chain has no range
receipt. This hook never loads an archive's executable and is not an isolation
boundary for unknown code. Pinning one executable does not pin the host loader
or dynamic libraries. The isolated runner and complete reference build kits
described below bind those separate runtime/build dependencies.

## Isolated native replay

`runner::replay_isolated` and the `replay` CLI require the exact ABI
`myelin-bundle-replayer-v1` and an explicit `ExecutionArtifacts.runtime` object:

```json
{"files":[{"path":"/lib64/ld-linux-x86-64.so.2","blob":"<BLAKE3 hex>"}]}
```

The file list is sorted by destination, has at most 128 entries and names exactly
the `runtime_dependencies` set. Only regular files under `/lib/`, `/lib64/`,
`/usr/lib/` or `/usr/lib64/` are admitted. Paths cannot contain traversal,
empty components, NULs, overlapping files/directories or arbitrary mount options.
Empty file lists support static programs. Current canonical execution descriptors
serialize `runtime` explicitly, including `null` for non-isolated profiles.
Isolated replay requires the layout object. Changing its mappings changes the
execution commitment and requires independent recognition; no historical
manifest preimage or omitted-field compatibility is preserved.

The operator supplies a separate `SandboxPolicy`: absolute installed bubblewrap
path and BLAKE3, wall milliseconds, CPU seconds, address-space bytes, stdout bytes
and stderr bytes. The launcher is trusted host infrastructure, never selected
from the downloaded archive. It is copied and hash-checked before execution.
There is no fallback to direct execution if the launcher, kernel support, runtime
library, target architecture or isolation setup is unavailable. The implementation
has Linux x86_64/aarch64 paths; acceptance here ran on Linux x86_64 with bubblewrap
0.13.1. Other platforms return unsupported-platform errors.

Before launch, all archive blobs are streamed into a new private staging
directory with fresh digest checks. The sandbox receives read-only copies of
the bundle and public trust/limit/session inputs, `/app/replayer`, and only the
declared runtime library files. No host home, `/usr`, source checkout, database,
socket or inherited environment is mounted. Its root, `/proc` and `/dev` are
read-only; `/tmp` is an empty read-only directory. There is no writable scratch
space in this ABI. The actual executable must be a matching 64-bit Linux ELF.

The runner requires user, mount, PID, network, IPC and UTS isolation, drops
capabilities, sets no-new-privileges, disables further user namespaces and starts
a new terminal session. A syscall allowlist permits the single-process native
runtime and the namespace init's wait/reap operations. Argument-filtered `clone`
permits only threads sharing one thread group, address space, descriptors and
filesystem context; these inherit the syscall filter and process resource
limits. `clone3` is unavailable because its pointer-based flags cannot be checked
by classic BPF. Independent processes, sockets, namespace entry, ptrace, keyrings,
SysV IPC and io_uring remain denied.
Inherited descriptors are marked close-on-exec except the launcher's filter
descriptor, which is consumed before application execution. The parent bounds
stdout/stderr, enforces a wall deadline and cancellation, then kills/reaps the
launcher; namespace teardown terminates the application. RLIMIT_CPU, RLIMIT_AS,
zero core size, zero file growth and a 256-descriptor ceiling apply before the
launcher starts. Only descriptor flags and duplication are allowed through
`fcntl`; advisory locking and owner-directed asynchronous signals are denied.

This policy follows bubblewrap's responsibility split: the host chooses the
mounts, namespaces and syscall policy, while the locally trusted kernel and
launcher implement them. See its [security model](https://github.com/containers/bubblewrap/blob/v0.13.1/README.md#sandbox-security)
and [option reference](https://github.com/containers/bubblewrap/blob/v0.13.1/bwrap.xml).
Isolation protects the host; it does not prove that an externally approved
replayer honestly implements the application. Kernel/launcher compromise or a
malicious same-user host process is outside this runner's trust model.

The application receives the manifest hex commitment as its sole argument, reads
`/run/{session,trust,limits}.json` and `/bundle`, and writes one `ReplayOutput` JSON
object to stdout. `replayer_input` verifies these bindings; the reference transfer
binary then uses `replay_current_process` to check its own artifact, restore the
latest state, and reexecute the entire genesis prefix. The host compares every
returned session/genesis/profile/range/root/resource field with the audited
history. Missing or altered output rejects. There is no `verified` flag accepted
from the subprocess and no CKB-VM/DA/L1/court promotion.

The report marks only `replayed_session` as host-replayed. Its nested catalog
audit retains `host_replay: not-run`; other sessions need separate invocations.
Host wall/CPU and kernel lifetime peak RSS are reported separately from receipt
VM cycles, including on execution failures. Kernel peak RSS includes launcher
and fork lifetime effects, not just application memory. Preparation time is
separate: the child deadline/CPU limits do not measure the parent audit/copy.
Staging is bounded by bundle limits and cancelled between blobs; large-history
lifetime storage still requires an operator retention/quota policy. Bounded
retention and superseded archive payload collection are described below; actual
filesystem-capacity faults are recorded in the workload measurements.

## Independent trust bootstrap

The caller supplies a separate allowlist of `TrustAnchor` values:

```text
network/deployment
session id
genesis config commitment
application profile commitment
execution artifacts commitment
```

Every session in a composition needs its own exact allowlist entry. Trust does
not flow automatically from a root session to another authority or application.
The `proposed_anchor` API only constructs a candidate for out-of-band review;
it does not approve it. The CLI never generates an accepted trust file from a
downloaded bundle. A bundle's own keys establish internal consistency during
finality auditing, but cannot establish that it is an intended official game.
The network label is externally scoped; it is not claimed as a new field in old
genesis formats.

## Bounded reads and filesystem behavior

Default import limits are 8 MiB manifest, 256 MiB per blob, 8 GiB expanded blob
bytes, 65,536 blobs, 262,144 edges, depth 1,024, 256 sessions and 60,000 blocks.
Explicit hard ceilings prevent callers from accidentally disabling all bounds.
These operational limits never relax genesis protocol limits. The declared graph
uses iterative traversal, so a deep hostile graph cannot exhaust recursion.

Unix imports pin directory file descriptors and open single fixed-name children
with `openat`, no-follow, nonblocking and close-on-exec flags. Symlink members,
nonregular files and symlink roots reject. Renaming or replacing the pathname
does not redirect an already-open bundle. Other platforms return an explicit
unsupported-platform error until an equivalent implementation exists.

Integrity scanning hashes one blob at a time with a 64 KiB buffer and verifies
actual length, end-of-file and digest. `read_blob` additionally enforces the
caller's allocation cap and rechecks the returned bytes on every read. A prior
successful scan is never reused as proof that mutable files stayed unchanged.
The manifest is bounded before JSON deserialization. Missing descriptor,
missing file, digest mismatch, unknown trust, dependency cycle and unsafe path
have distinct machine-readable error codes.

`BundleStore` reconstructs bounded checkpoint-height, command, handoff and evidence indexes from
verified record bytes. It reads blocks lazily; whole-chain materialization,
writes, delivery ACKs and writer WAL operations are disabled. Recovery pages
contain at most 64 records and normally at most 16 MiB of encoded records; one
larger record may occupy a page, subject to the 128 MiB record codec ceiling.
Recovery reads until an empty page, so a short byte-budget page cannot hide the
tail of a history. Per-store command and per-session outbox indexes are capped
at 1,048,576 entries; manifest limits also bound the auxiliary indexes.
Checkpoint lookup uses the reconstructed height index and rereads the selected
content-checked record; it does not scan all preceding blocks.

## Hot retention and durable archive pins

RocksDB schema 15 supports explicit cold block references. A block is relocated
only after the configured `ArchiveRecordSource` supplies bytes exactly equal to
the hot encoding, with the pinned length and BLAKE3 digest. One synchronous WAL
transaction replaces the hot block key with a 77-byte manifest/blob/length pin.
It checks the current head and all relevant handoff/outbox keys for concurrent
changes. It never advances the head or changes rewards, balances or commands.
Schema 14 and earlier are rejected; there is no upgrade/import path.

`BundleArchives` registers already-audited `BundleStore` instances and supplies
bounded, content-checked block reads. Open the live store with `open_archived` and
that registry before relocating blocks. Normal block/page/checkpoint reads,
recovery, replay and transactional source checks resolve the explicit cold key;
absence of a provider, manifest or blob fails instead of truncating the history.
The hot/cold key pair is read through one database snapshot. A configured
provider is still responsible for availability and retaining the complete
manifest dependency/artifact closure. Its presence alone proves no application
reexecution, independent replication or permanent physical durability.

`offload_block` is a bounded, per-record operation. Its required checkpoint
callback runs before opaque store reads (128 MiB plus bounded compact metadata),
archive reads (exact
blob length) and the synchronous commit (zero bytes). A callback may park or
return an error before the mutation. `RetentionWindow` keeps an
operator-specified hot suffix and up to 256 protected recovery/dispute ranges.
The current tip and blocks with pending outbox deliveries stay hot. Blocks tied
to Issued or Cancelled handoffs remain hot; consumed scarce claims stay hot until
Retired, and cancelled claims until Reclaimed. A trusted
`HandoffRetentionPolicy` may classify a consumed effect as copyable.
`TransferPolicy` authenticates the source genesis, effect policy, signed payload
and transfer kind before doing so. `RetainUntilTerminal` keeps unknown semantics
pinned. Wall-clock expiry does not discharge a hold or permit a refund.

| Record | Retention in this implementation |
| --- | --- |
| Genesis, current head and latest complete checkpoint | Remain hot; no deletion API |
| Historical blocks and archival snapshot payloads | May move together into the pinned archive after the above checks |
| Checkpoint height/hash index | Remains hot; resolves its selected record through the configured cold source |
| Command receipts and anti-replay identities | Remain hot and permanent; log expiry never makes a command reusable |
| Handoff terminal state and source authorization | Live obligations retain full records; reclaimed/retired payloads may be replaced by permanent hot lifecycle markers bound to the exact cold issuing block |
| Pending outbox and evidence receipts | Remain hot; only the existing explicit delivery ACK removes its queue entry |
| Historical programs, profiles and package chunks | Retained with every referenced manifest and its dependency closure |
| Authority ownership/key material | Separate authority store and deployment procedure; relocation copies none of it |

`archived_blocks(after, limit)` enumerates at most 1024 durable pins per page.
Archive collection must inspect a consistent capture or quiescent store: a moving
multi-page scan cannot prove that an archive is unused. There is no pin expiry or
eviction API. Restart must configure the required manifests again. The reported
hot-record bytes are logical bytes removed, not reclaimed SST bytes; compaction,
disk quotas and physical-space measurements are separate operational work.

An existing cold block can rebind to a different manifest only when the blob
digest and length are unchanged and the replacement bytes decode for the same
session/height. The swap is another synchronous transaction with head and cold
pin conflict checks. It does not require the old archive files: an independently
audited replacement can repair their loss. Hot-retention windows apply to moving
hot records; they do not prevent relocating an already-cold manifest reference.
`AlreadyArchived` and `Rebound` distinguish those cases from a hot-storage hold.
Registering a verified copy of the same manifest may refresh its filesystem
location, while in-flight readers retain their previous handles.

### Terminal handoff payload compaction

Relocation also replaces duplicate full `StoredHandoff` entries for **Retired** or
**Reclaimed** handoffs issued by that block. A permanent compact entry stores the
exact issuance and terminal block references in at most 256 bytes. The handoff ID
remains the key; the original payload and authorization remain in the exact,
digest-checked issuing block. New compact markers and cold block pins are written
in the same synchronous optimistic transaction that deletes the hot copies.
There is no expiry or deletion operation for those identities.

Only the two immutable terminal states compact. Issued, Cancelled and Consumed
entries retain their full payloads. A trusted copyability policy can permit moving
a consumed copyable claim's block to cold storage, but does not erase or compact
its consumed marker. Current tips, hot suffixes, protected dispute/recovery ranges
and pending delivery holds prevent compaction. These holds also apply when a cold
block is revisited or rebound. A repeated pass reports zero new compactions once
the full entry is gone.

Reads atomically select either the full or compact entry and reject overlap. A
compact read loads the issuing block through its existing archive pin, checks the
exact block hash and handoff ID, then reconstructs the same public `StoredHandoff`.
Missing or changed archive data is an error. Export/replay therefore retains the
original semantic handoff format and full dependency closure; compact markers are
local storage records only. Recovery still checks the terminal references against
the actual consumption, retirement or refund history. Full backup captures both
the marker and its cold dependencies without regenerating indexes.

The retention result separates logical hot block bytes from the **net** handoff
index bytes saved after replacing each payload with its marker. It does not claim
immediate SST space reclamation. Permanent command/handoff identities still grow
with lifetime activity, and archive roots remain pinned; compaction alone cannot
establish a fixed total storage bound or authorize archive deletion. RocksDB schema
15 is required, with no decoder or upgrade path for prior store schemas. MREC and
block wire formats are unchanged.

### Bulk retention

`ArchiveTask::retain(store, archives, plan, policy)` uses the registry configured
on the live store. `RetentionPlan` contains 1–1024 explicitly trusted archive
catalogs and 1–1024 sorted, nonoverlapping session/height ranges. Each range names
its target manifest, hot suffix and protected recovery/dispute intervals.
`BundleLimits` caps bytes, nodes, edges, sessions and blocks across the combined
catalog set, including repeated content in separate catalogs. Catalog
commitments, complete dependency closure and finality are checked before any
mutation. The task then captures each session head once and checks both bounds
of every requested range. Each individual block still verifies exact bytes and
handoff/outbox holds in its own optimistic transaction.

Work pauses inside reads and before commits using the existing archive budget.
`ArchiveProgress.retention` includes requested/processed block counts, moved and
rebound counts, already-cold blocks, each hold reason, logical hot block bytes
removed, `handoffs_compacted` and net `handoff_bytes_removed`. These counters are also included in errors after partial progress.
Earlier per-block commits remain durable on pause, cancellation, source-head
conflict, or cursor drop; this is not an all-or-nothing batch. Re-running a plan
captures new heads and recognizes already-completed relocation. A head conflict
returns `SourceChanged`, never a completed report for the unfinished range.

The CLI `retain <rocksdb> <plan.json>` uses the same task. Its `work retain`
variant accepts budget grants on stdin; EOF discards only the process-local
cursor and reports the last progress. Archive paths are relative to the plan.
The CLI uses `RetainUntilTerminal`; library callers may supply an authenticated
application policy for copyable claims. Plans cannot assert a copyability flag.
The CLI opens an existing database exclusively, while the library API can share
an existing live store and detects concurrent writers at the commit boundary.

Example plan structure (replace the identifiers and trust anchors with the
actual exported manifest and independent trust policy):

```json
{
  "archives": [{"path": "archive", "commitment": "<manifest hex>", "trust": ["<TrustAnchor object>"]}],
  "ranges": [{
    "session": "<session hex>", "first": 0, "last": 100,
    "archive": "<manifest hex>", "hot_from_height": 90,
    "protected": [{"first": 20, "last": 25}]
  }],
  "limits": {
    "manifest_bytes": 8388608, "blob_bytes": 268435456,
    "total_bytes": 8589934592, "nodes": 65536, "edges": 262144,
    "dependency_depth": 1024, "sessions": 256, "blocks": 60000
  }
}
```

Rebinding alone does not authorize archive deletion. Collection checks every
database in its explicitly owned scope and the replacement dependency closures.
Measured lifetime storage bounds remain open; moving hot block bodies elsewhere
does not establish a total storage bound.

### Superseded archive payload collection

On Linux, `ArchiveTask::collect(CollectionPlan)` and `collect <plan.json>` reclaim
payload files from superseded archives. The required plan fields are
`databases`, `archives` (keepers), `candidates`, `protected` and `limits`.
Keepers and candidates use the existing `ArchiveCatalog` shape; paths are relative
to the plan file in the CLI. The plan is an explicit ownership boundary: include
every database/checkpoint that can reference these archive directories. The API
cannot discover references in an omitted database, remote deployment or external
backup catalog. Managed archive directories must remain immutable outside these
APIs. A complete self-contained backup keeps its own archive copies.

The task opens every existing database privately and holds its native exclusive
lock until collection ends. It fingerprints the complete keyspaces and checks
every cold pin against an independently audited keeper, including exact session,
height, blob and length. A pin to a candidate or an omitted catalog rejects the
plan. Use retention's atomic exact-blob rebinding first. Collection never edits
database records, command identities, handoff terminal markers or pending queues.
Database entry/byte/session budgets apply across the supplied databases; archive
budgets apply across keepers and candidates together.

Every candidate session must have the same independently recognized identity,
genesis and consensus as a keeper, with an identical complete block prefix.
All candidate blocks, claims, evidence, programs, migration inputs, build/runtime
files and their dependency edges must remain in the keepers. Only the old
indexed head and latest-checkpoint snapshots may be superseded by the keeper's
audited later snapshot. Set `protected` to retain exact archive roots required
by recovery latency or dispute windows; a protected candidate rejects. Claim or
evidence state changes that lack an exact retained copy also reject; collection
does not infer permission from age or terminal status.

`BundleDirectory` readers retain shared nonblocking locks on both the archive
root and payload directory, including derived/parked readers and separately
mounted payload aliases. Collection takes exclusive locks on all candidates;
an active reader makes the plan fail without deleting payloads. Keeper readers
remain locked throughout the task. These are advisory locks among cooperating
users of this library, not protection against an external filesystem writer.
The isolated runner permits only the shared, nonblocking `flock` operation.

Before deletion, the task validates all candidates and rejects unexpected files,
symlinks and special files. It atomically renames `manifest.json` to `gc.pending`
and syncs the directory before unlinking any declared blob through pinned directory
handles. It retains this canonical manifest and the empty `blobs` directory as
a restart marker. Normal readers cannot open a collected archive. The marker
is not a verification receipt: rerunning the plan reopens all databases,
re-audits keepers and rechecks recognition, coverage and pins before completing
remaining unlinks. Missing payloads are allowed only after that marker exists.

The task shares the existing work budgets and `work collect` stdin grants.
`ArchiveProgress.collection` reports completed candidates, removed/already-missing
blobs and unlinked file lengths, including partial errors and pauses. Unlinked
lengths are not physical disk savings because files can be hardlinked. Dropping
a cursor or killing the process keeps completed deletions; rerun the same plan
to finish. General audit/replay cursors remain in-process. The
[workload measurements](../operations/workload-measurements.md) report archive
file sizes and recovery costs separately from physical GC savings and deployment
RPO/RTO, which they do not establish. No extra acceptance lane is required.

## Consistent database backup and restore

`ArchiveTask::backup(Arc<RocksSessionStore>, BackupPlan, new_directory)` captures
one native RocksDB checkpoint while the application's live store may continue
writing. This includes every database key: hot records, cold pins, command and
handoff identities, pending outbox/network queues, sequence counters and recovery
WAL state. It then scans that fixed checkpoint under a read snapshot to commit
the complete ordered key/value fingerprint, session heads and exact cold catalog
set. This is distinct from the read-only evidence export described below.

`BackupPlan` requires `archives` and `limits`. Each `ArchiveCatalog` uses the same
`path`, independently recognized manifest `commitment` and `trust` fields as a
retention plan. Paths are relative to the plan file in the CLI. Supply exactly
those catalogs pinned by the capture, or an empty list for a hot-only store.
Missing, extra or duplicate catalogs reject. Catalogs undergo the shared bounded
inventory, dependency and finality audit; every captured cold pin must match the
audited session/height/blob/length reference. Every file in each pinned catalog,
including its declared artifact and dependency closure, is copied into the backup.

The new directory contains `db/<RocksDB files>`,
`archives/<manifest hex>/{manifest.json,blobs/<blob hex>}`, and canonical
`backup.json`. The checkpoint is closed before its engine files are copied. Each
file is bound by its relative name, length and BLAKE3 digest. The manifest records
the current storage options, full keyspace inventory and local clock bounds
around checkpoint creation. `BackupManifest::commitment` binds the canonical
manifest; retain this digest through an independent trusted channel. The temporary
checkpoint is removed before `backup.json` is published. A successfully completed
backup has no dependency on the original database or original archive paths.

`ArchiveTask::restore(backup, expected_commitment, trust, limits, new_directory)`
requires that independent digest before creating its destination. It rejects
noncanonical manifests, unsupported formats, unsafe member paths, symlinks,
nonregular files, changed lengths or digests, and existing destinations. Copied
cold catalogs are audited against the separately supplied trust anchors. The
restored current-schema database must reproduce the complete captured keyspace
fingerprint, heads and pins before `restore.json` is published. The library
returns the opened store with its cold provider configured; the report lists
its database and copied archive directories. On a later open, register those
catalogs with their independent trust anchors and use `open_archived` again.

This is a current-schema physical backup (`myelin-session-backup-v1`), with the
native engine/version requirements of this build. There is no old-schema import,
upgrade, recovery index rebuild or conversion shim. The captured sessionstore
contains no separate authority DB, command-journal DB, signing-key directory or
undeclared application runtime files. Back up those deployment components under
their own consistency and secret-handling procedures. A hot-only capture does not
supply an application executable. Restore reports `writer_authority` and
`application_replay` as `not-run`: storage identity does not establish application
reexecution or permission to resume canonical writes. Follow the
[writer migration procedure](../operations/writer-migration.md) before activation.

Both tasks use the existing `ArchiveBudget`/cancellation/cursor protocol. File
copies stream in 64 KiB chunks and publish only after all validation completes.
Failures leave an unpublished partial destination; use a new directory for a new
task. As with export, a filesystem failure after the publication link can leave a
marker whose directory sync was not confirmed; require successful completion.
`BundleLimits` bounds the final files plus backup manifest, member count, logical
keyspace and combined archive catalogs. Native checkpoint creation may flush,
copy or hardlink engine files before their sizes are known. It cannot be preempted
and its temporary disk usage is not a hard quota. Logical I/O reservations are
not measurements of physical disk, peak RSS or elapsed native-call time. A live
compaction that changes hardlinked source-file metadata can abort copying with
`SourceChanged`; retry from a new capture. The
[workload measurements](../operations/workload-measurements.md) cover long-history,
many-session and multi-epoch archive recovery. They do not measure online backup
throughput or establish deployment RPO/RTO. Cursors remain in-process.

```bash
myelin-session-bundle work backup /path/to/store backup-plan.json /new/backup
myelin-session-bundle work restore /new/backup "$EXPECTED_BACKUP_COMMITMENT" external-trust.json /new/restore limits.json
```

The standalone backup CLI must acquire the database's exclusive lock; online
capture uses the library API with the application's existing live `Arc`.
`work` grants arrive as JSON lines on stdin, as for retention/export. No additional
validation gate or automatic authority activation is introduced.

## Export and consistent capture

`export_store` accepts an owned `Arc<dyn SessionStore>` and a complete catalog of sessions, their public consensus
TOML configurations, execution descriptors and explicitly supplied local artifact
files. Every catalog session is included. Artifact files stream through a 64 KiB
buffer into digest-named blobs; records are read one at a time. Shared artifacts
are deduplicated. No store WAL, operational delivery index, authority epoch
database or private signing key is copied automatically.

The source must be quiescent. The exporter captures head/checkpoint pairs and
rechecks them and all captured handoff/evidence terminal records before
publication. Changes return `source-changed`. This detects ordinary concurrent
advancement, not malicious source rollback. The CLI opens an existing current-
schema RocksDB with its exclusive lock; applications must close that store first.
Opening RocksDB may maintain its own operational files but does not perform an
implicit schema migration.

The destination must be new. Export writes `manifest.pending`, derives causal
edges and performs a complete internal finality audit before syncing and
publishing `manifest.json` without overwriting an existing file. Internal proposed
anchors are used only for that consistency check; no trusted allowlist is
generated. Failure before publication retains staging for diagnosis without a
final manifest; retry failures with a new destination. A filesystem error after
the publication link can leave a manifest whose directory sync was not confirmed.
Paused work retains the same destination and resumes through its owned cursor.

Inventory, typed-package validation, index construction, dependency checks,
finality auditing, export, packing, assembly, retention, collection, backup and restore share
`ArchiveTask` budgets.
`advance(ArchiveBudget, &RecoveryCancellation)` returns either `Complete` with
the operation's report and usage or `Paused` with a process-local owned cursor.
`PausedArchive::resume` accepts a fresh budget and cancellation token; progress
JSON cannot recreate the cursor or confer verification/publication rights.
Dropping it wakes and aborts the parked worker at its next cooperative boundary.
Each pending task owns one worker thread, preserving streaming hash state, file
offsets, indexes and the existing call stack rather than restarting the scan.

Defaults allow 100,000 work steps, 8 GiB of logical I/O reservations and 30 seconds
per grant. Blob reads/writes reserve requested bytes, including EOF probes;
opaque source-store reads conservatively reserve the 128 MiB record ceiling.
These counters are neither physical I/O nor RSS. Each synchronous bounded read,
codec/crypto call and filesystem sync can finish after a deadline; cancellation
and time limits are cooperative between calls. Open/parse of CLI configuration
and standalone input manifests precedes the archive task; retention catalog
manifests are opened inside the task. Retained indexes still obey `BundleLimits`;
callers bound the number of concurrently parked tasks. Retention reserves opaque
store-read ceilings in addition to the archive provider's streaming I/O, so its
byte counter deliberately overestimates bytes physically read. A parked retention
transaction can retain a database snapshot until resumed or dropped.

Source files are checked for metadata changes across streamed reads, including
changes to an already-hashed prefix during a pause. Each later use still checks
the content digest. This is not an atomic filesystem snapshot. Export rechecks
captured source state after the internal audit, immediately before publication;
the whole final source pass is reserved before its first record read. One grant
must fit those steps and conservative record-read bytes. An undersized grant
pauses with `progress.next_operation` and performs none of that pass, rather than
repeatedly reading a prefix that can never fit. `next_operation.elapsed_ms` is
zero because an I/O duration cannot be predicted. Cancellation and deadlines are
still checked between its prepaid reads and before publication; a pause there
requires a new complete reservation and a fresh source check. No pause is returned
after publication. Reservations account for their full upper bound even if a
changed source or cancellation ends a check early.
General audit/replay cursors remain in-process; process exit requires a fresh
audit. Collection can restart from its canonical marker but redoes verification;
online database capture is described above. See the
[workload measurements](../operations/workload-measurements.md) for measured
history/epoch sizes, recovery costs and reference RPO/RTO objectives. The
measured recovery phases do not establish the complete end-to-end deployment
objective or total-disk-loss recovery.

Application inputs and snapshots are exported exactly. They may themselves
contain private data; the caller must choose an archiveable application profile
and explicit artifacts that exclude unnecessary PII or credentials. Removing
replay-relevant authorization, timeout or entropy events would alter history.
Content addressing is not a privacy filter.

## CLI and assurance report

```bash
cargo run --locked -p myelin-session-bundle -- inspect /path/to/bundle
cargo run --locked -p myelin-session-bundle -- verify-inventory /path/to/bundle /path/to/reviewed-trust.json
cargo run --locked -p myelin-session-bundle -- verify /path/to/bundle /path/to/reviewed-trust.json
cargo run --locked -p myelin-session-bundle -- export /path/to/closed-rocksdb /path/to/export-plan.json /path/to/new-bundle
cargo run --locked -p myelin-session-bundle -- work retain /path/to/closed-rocksdb /path/to/retention-plan.json
cargo run --locked -p myelin-session-bundle -- work collect /path/to/collection-plan.json
cargo run --locked -p myelin-session-bundle -- replay /path/to/bundle /path/to/reviewed-trust.json <session-hex> /path/to/sandbox-policy.json
```

`inspect` returns an `unverified-inventory` and performs no artifact scan.
`verify-inventory` checks the declared inventory, every content digest, the real
genesis bindings and independent trust entries. Its report always marks host
replay, CKB-VM verification, finality, DA, CKB inclusion and court adjudication
as `not-run`. `verify` additionally audits the complete histories and reports
`finality: verified`, with separate session, genesis, profile, consensus config,
module and head identities. This is each genesis-bound closed validator domain;
no combined security domain or canonical authority epoch/membership claim is
made. Evidence record bindings and required stages are checked, but the current
CLI does not run DA providers, CKB adapters or a court. Those stages and host
replay remain `not-run` in its report.

`replay` additionally invokes the isolated runner for one explicitly selected
session and emits the layered report above. `SandboxPolicy` JSON is bounded to
64 KiB. Build the reference application runner with
`cargo build --locked -p myelin-session-bundle --features reference-replayer --bin myelin-reference-transfer-replayer`.
An integration must archive the actual executable and its runtime layout when
producing its export; compiling a new program later cannot replace old pinned
code. The reference binary is not a universal runner for other applications.

`export` returns `exported-internally-audited` with the manifest commitment.
`retain` returns `retained` with per-block outcome counts; that stage makes no
new finality, application-replay, replication or physical-space-reclamation claim.
`backup` returns `backed-up` with the independent commitment and captured inventory;
`restore` returns `restored` with the storage report described above.
Prefix `verify-inventory`, `verify`, `export`, `pack-files`, `assemble-files`,
`retain`, `backup` or `restore` with `work` to retain a cursor across stdin JSON-line grants in one CLI process:

```bash
myelin-session-bundle work verify /path/to/bundle /path/to/reviewed-trust.json
# stdin, one grant per output; send further grants after status: paused:
{"budget":{"max_steps":100000,"max_io_bytes":8589934592,"max_elapsed_ms":30000},"cancelled":false}
```

Each grant is capped at 64 KiB. A paused response contains reason/progress and
no verification report; completion contains `status: complete`, `report` and
`usage`. EOF discards the process-local cursor. One-shot commands use one default
grant and report `paused` as an error when exhausted; they cannot resume after
process exit. `work` does not cover application sandbox replay or rebuilds.

Its plan uses the strict `ExportPlan` JSON schema: `roots`, `sessions`,
`artifacts` and optional `limits`. An artifact entry is `{ "id": "<BLAKE3 hex>",
"path": "<local file>", "dependencies": [] }`; relative paths resolve from the plan's directory.
The required sorted, unique `dependencies` list names other artifact source
digests. The exporter copies their transitive closure under the referring
artifact's role, preserving build-package chunks without treating them as
runtime libraries. It checks missing nodes, cycles, edge counts and depth before
creating the destination. These edges are combined with mandatory protocol edges;
they cannot remove a history dependency. Missing dependency lists are rejected.
The referring package bytes must themselves bind their member identities or
complete-file hashes: an arbitrary graph edge alone is not application-level
recognition of its content. Typed file-package assembly/reconstruction remains
separate from this generic export facility.
Session entries contain `session_id`, `network`, `consensus_toml` and the
`ExecutionArtifacts` object described above. The CLI bounds the plan to 8 MiB.

All read commands accept an optional final `limits.json` argument, bounded to
16 KiB; defaults apply when omitted. Overrides must stay within hard ceilings.
Unstripped debug replayers may need an explicit per-blob override up to 512 MiB.
The importer never accepts a bundle's `verified=true` assertion. Failures are
JSON on stderr with exit code 2.

The trust file is an externally obtained JSON array of anchors, bounded to 1 MiB.
Reviewing an anchor is an application/operator decision, not an action performed
by bundle verification.

## Current acceptance

```bash
cargo test --locked -p myelin-session-bundle
# Linux isolation lane: installed bubblewrap is required (MYELIN_BWRAP can override its absolute path).
cargo test --locked -p myelin-session-bundle --features test-fixture
```

The inventory tests cover changed artifact bytes, missing files/descriptors, public anchor
fields pasted over another genesis, changed network/profile/execution mapping,
strict format and budgets, dependency cycles, independent composition anchors,
directory replacement/symlinks and actual CLI assurance/error output.

Archive tests export actual scarce-transfer history, remove the original database
and reproduce source/target roots. They cover long history and tail corruption,
mandatory source/cancel/refund/successor dependencies, independent anchors,
forged terminal indexes, altered authority/rules/inputs/snapshots/replayer,
read-only stores, bounded and no-clobber export, a real concurrent source commit,
and separate CLI export/verify processes. Snapshot tampering is distinguished
from finality failure: an opaque snapshot can pass structural audit but must
fail application restore/replay.

The isolation lane adds real archived transfer execution after deleting the
source database and supplied executable, a separate CLI replay, denied host-file
read/write, archive/root writes, inherited descriptors/environment, sockets and
descendants. It exercises output floods, CPU and wall limits, failed large
allocation, process exit, forged replay totals, changed runtime mappings/bytes,
missing/wrong launchers and cancellation of an observed live application. The
host then observes that application terminate. A portable unit check rejects
runtime path injection and overlapping destinations.

With `MYELIN_BUNDLE_RUNNER_EVIDENCE_OUT` set to a new directory, the isolation
test retains its complete controlled archive, public fixture trust, policy,
limits, CLI executable and reports. The production gate uses this option. These
are real pinned executable/library artifacts, but their build/toolchain labels
are still synthetic. They do not establish reproducible builds. Large debug
executables can require several GiB of temporary storage when archive tests run
concurrently; set `TMPDIR` to a filesystem with sufficient per-user quota. The
sandbox fixture removes replaced artifacts and releases each fault archive
before constructing the next one.

## Fresh-container offline restore

The retained fixture can be independently packaged and replayed with Python
3.11+, Podman, `ldd`, and the same supported Linux namespace capabilities:

```bash
python3 scripts/myelin_bundle_container_acceptance.py \
  --evidence /absolute/evidence/isolated-bundle \
  --trusted-cli /absolute/evidence/isolated-bundle/myelin-session-bundle \
  --trusted-launcher /usr/bin/bwrap \
  --out /absolute/evidence/bundle-container
```

The output must be new and disjoint from the input directory. The two trusted
executables are operator inputs: dependency discovery invokes `ldd` only on
them, never on an archive-supplied program. Their complete discovered loader and
library files, the archive, and public operator inputs form a `FROM scratch`
image. Every file's SHA-256, length and mode are recorded. The launcher still has
to match the independently supplied sandbox policy's BLAKE3.

The script runs as a non-root container user with no network, host bind mounts,
capabilities, source checkout, original database or Cargo cache. The root is
read-only; bounded `/tmp` is for trusted verifier staging. Outer OCI seccomp and
proc masking are explicitly disabled so the trusted bubblewrap launcher can
create the required nested namespaces and proc mount. The inner application
sandbox retains its syscall allowlist, read-only root, descriptor/environment
clearing and resource limits. No privileged container or weaker replay fallback
is used. Host policy may prohibit nesting; that is a failed acceptance run.

After the first exact result/receipt comparison, the script exports
`runtime-image.tar`, imports it into a fresh empty Podman image store and repeats
the comparison. `acceptance.json` uses
`myelin-bundle-container-acceptance-v1` and binds the image/archive identities,
runtime inventory, commands, outcomes and host metrics. The adjacent stdout,
stderr and invocation JSON files retain raw evidence. The exported image can
be loaded offline using the recorded `image_load` and `offline_replay` commands.
This is clean-filesystem and empty-cache acceptance on the same kernel, not a
second physical machine, reproducible build, official game recognition or an
additional CKB/DA/court claim.

## Complete reference build kits

`scripts/myelin_reproducible_build.py` packages all nonignored Git-listed working
tree files (including uncommitted files), the complete `cargo vendor` directory,
an exported OCI toolchain image and the exact build script. It includes full
source/vendor file lengths, modes and SHA-256 values, checks those against the
uncompressed tar members without extracting on the host, and publishes
`manifest.json` last. The schema is `myelin-build-kit-v1`. This is a working-tree
snapshot with a baseline commit for context, not an assertion that the baseline
commit contains the uncommitted implementation.

Prepare the toolchain and inputs on a trusted build machine (Python 3.11+, Cargo,
Git and Podman are required):

```bash
podman build --pull=never -f scripts/reproducible-build/Containerfile \
  -t localhost/myelin-repro-toolchain:rust-1.92.0 scripts/reproducible-build
cargo vendor --locked --versioned-dirs /absolute/build-inputs/vendor
python3 scripts/myelin_reproducible_build.py prepare \
  --source "$PWD" --vendor /absolute/build-inputs/vendor \
  --builder-image localhost/myelin-repro-toolchain:rust-1.92.0 \
  --out /absolute/build-inputs/kit
```

The Containerfile pins the Rust 1.92.0 Debian base by digest; fetch that base
before `--pull=never` on a clean host. Preparing the toolchain installs the native
compiler/libclang/build tools and needs package-repository access. A proxy bound
to host loopback may require `podman build --network=host` during this preparation
only. The resulting toolchain image and its OCI archive are pinned in the kit;
later rebuilding neither repeats apt nor fetches crates. This workflow does not
claim reproducibility of the toolchain OCI image itself.

Recognize the printed manifest SHA-256 through an independent trusted channel.
The verifier requires that expected digest explicitly; it never accepts a trust
label read from inside the package:

```bash
python3 scripts/myelin_reproducible_build.py verify-inputs \
  --kit /absolute/build-inputs/kit --expected-manifest <trusted-SHA256>
python3 scripts/myelin_reproducible_build.py reproduce \
  --kit /absolute/build-inputs/kit --expected-manifest <trusted-SHA256> \
  --out /absolute/reproduction
```

`verify-inputs` performs no execution and cannot claim reproducibility. The
`reproduce` command imports the toolchain into an empty image store, then creates
two separate unprivileged containers with no network, capabilities or host bind
mounts. Each has a new source extraction, Cargo home and target directory. Builds
first verify the actual copied source/vendor/script bytes against the externally
recognized manifest, before even executing the build script. Builds
use Rust 1.92.0, frozen/offline Cargo, four jobs, path remapping and dev settings
with optimization/debug information/incremental compilation disabled. The image
pins the native compiler, linker, headers, libc and build tools. Per-build wall,
CPU, memory and process-count limits are operational controls. Podman's native
container timeout also caps runtime if the Python controller disappears. On
handled failure or cancellation the owned container is removed; logs and
outcomes remain. Abrupt controller loss can leave a running or stopped owned
container: use the recorded `create.stdout` identity and the same image-store
arguments to inspect/wait for it and retrieve its result. Never restart a build
merely because its controller or observation stream was interrupted.

The reference recipe builds `myelin-reference-transfer-replayer`, the bundle
CLI and the `inventory`, `turn_based` and `fixed_tick` examples, retaining all
five exact binaries, discovered runtime libraries and tool versions.
`reproduction.json` uses `myelin-build-reproduction-v1` and reports success only
when both full output inventories match byte for byte. Missing inputs or vendor
dependencies cannot be supplied by the host's checkout, network or Cargo caches.
Reproduction establishes the declared build's input completeness and binary
identity on the tested kernel/architecture. It does not establish source
correctness, release performance, official deployment recognition, a second
physical machine, or reproducibility of another application's recipe.

The CLI reports structured JSON failures for unknown trust anchors, missing
artifacts, integrity/format errors, deadline, cancellation and build failures.
The portable test suite checks those boundaries, archive traversal/links/duplicate
members and complete output inventories. A controlled post-copy replacement
drill checks that a changed build script is rejected before execution and the
owned container is removed.

## Typed file packages and archived build acceptance

A build-input or toolchain root must be a current file package. Its bytes start with
`MFPK`, followed by canonical compact JSON with format
`myelin-file-package-v1`. The remaining fields, in order, are `files`, containing
objects with `name`, `bytes`, `digest`, `chunks`; each ordered chunk has `id`
and `bytes`. All digests are raw BLAKE3. Files are sorted by name. The full root
digest commits names, lengths, complete-file digests and chunk order; each chunk
also has its ordinary bundle content identity. Repeated chunk references are
legal but count separately towards expanded bytes and reference budgets.

The root's manifest dependencies must include every chunk. Removing both a
chunk and its declared edge cannot hide the missing dependency: inventory
verification derives those edges again from the checked package bytes and
streams the ordered chunks to verify each assembled-file digest. Every root
is read with a byte cap and checked digest before decoding. The package
manifest is capped at the smaller of 8 MiB, the caller's manifest budget and its
blob budget. All packages together remain within the caller's expanded-byte and
reference budgets. Large files need more bounded chunks, not a larger blob cap.

Only regular file bytes are represented. Names are bounded relative portable
paths; traversal, absolute paths, backslashes, platform-reserved components,
case collisions and file/directory overlaps reject. No links, archive extraction,
permissions or executable hooks are interpreted. Empty files have no chunks and
must carry the empty-byte digest. Source paths appear only in the local packing
plan, never in the file package. Opaque build/toolchain metadata rejects; there
is no prefix probe or alternate path that skips package validation.

Create a packing plan with explicit source files and a chunk size, for example:

```json
{"files":[{"name":"toolchain.oci.tar","path":"/absolute/kit/toolchain.oci.tar"}],"chunk_bytes":16777216}
```

```bash
myelin-session-bundle pack-files plan.json /absolute/new-packed-directory > packed.json
myelin-session-bundle assemble-files /absolute/bundle /absolute/external-trust.json \
  <package-root-BLAKE3> /absolute/new-restored-directory
```

Merge the returned `artifacts` into an `ExportPlan`, and set the corresponding
`execution.build_inputs` or `execution.toolchain` to `root`. Both roles may bind
one complete kit package. These roots are part of the external execution
trust commitment. The exporter carries all transitively named chunks. The assembler
requires external recognition and full inventory/package checks before creating
its destination, then rechecks bytes while copying. It publishes each completed
file without overwriting an existing path. A failure returns no success report;
earlier completed files may remain in the newly created output directory.

`InventoryReport.file_packages` reports checked roots, file counts, expanded
bytes and repeated references. It establishes file integrity, not successful
compilation or replay. The independent fixed encoding is retained in
`fixtures/protocol/file-package-v1.json`; inventory tests use current genesis and exact artifact identities.

The complete reference acceptance lane connects these operations:

```bash
python3 scripts/myelin_bundle_build_acceptance.py \
  --kit /absolute/recognized-kit --expected-manifest <trusted-SHA256> \
  --outputs /absolute/original-reproduction/build-1/result \
  --trusted-cli /absolute/current/myelin-session-bundle \
  --out /absolute/new-build-acceptance
```

It packs the exact source/vendor/script/toolchain and original kit manifest,
exports a real development-key cross-session transfer with the original
reproduced executable and libraries, and restores all kit bytes using both the
library and CLI. It then rebuilds the restored kit twice in fresh offline
containers, compares all output files with the supplied historical inventory,
and runs the rebuilt original CLI against the new archive. The original result
must equal the current trusted verifier's result. No original database is needed
for that replay. The report uses `myelin-bundle-build-acceptance-v1`, with adjacent
command/stdout/stderr evidence. The supplied output inventory is not trusted as
a build proof: the restored builds must actually produce it again.

The same lane also runs all three original game examples in a fresh offline
container. Each commits its demo through the command journal and authority,
exports its own complete history and build kit, then deletes its fixture source
databases. The container is removed before isolated replay. Export anchors come
from the source processes and supplied artifact plan, not downloaded manifests.
Each archived program and runtime must match the reproduced output inventory;
its replayed state root and full range receipt must equal the native demo.
The explicit per-export work budget is 100,000 steps, 16 GiB accounted I/O and
30 seconds. It does not change default library budgets or retry pauses implicitly.

This lane requires Linux namespace support, bubblewrap, Python, Podman, Rust,
the current source checkout for the controlled fixture, and enough temporary
disk for chunks, restored inputs and a fresh image store. Source/toolchain
preparation can require network; the two container builds disable it. It remains
a same-host clean-filesystem test, not independent physical-machine acceptance.
The recognized historical source snapshot is deliberately preserved; later
exporter/package changes are not silently attributed to that old build.


### Retirement dependencies

The current MREC 13 envelope carries retirement blocks and terminal handoff indexes.
A retiring source block depends on the exact target consumption block as well as
its original issuance. A retired index retains both consuming and retiring
references; removing a source pending lock does not allow either history to be
pruned. Replayed application state removes only the consumed backing lock and
retains anti-replay nonces and target receipts.
