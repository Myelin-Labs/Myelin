# Concurrency & Optimization Plan

> Scope: host-side (off-chain) concurrency and throughput work that does
> **not** modify CKB-VM. The VM is treated as a fixed verification oracle.

This document is the single source of truth for what parallelism and
optimization work is in flight, done, or planned for the Myelin runtime.
It is written so a contributor can pick an item and know its rationale,
touch points, and "done when" criterion before starting.

## Current state (reviewed 2026-11-06)

Canonical execution, application replay and RocksDB commit APIs are synchronous.
Rayon parallelizes independent CellTx/script work. Tokio drives the producer and
network services; authority RPC dispatches execution/WAL work through a bounded
`spawn_blocking` admission path. Cancelling an RPC does not cancel an admitted
commit. Native workers and historical archive programs have separate process
limits. These boundaries require workload measurements; async transport alone
does not establish session isolation or throughput.

| Dimension | Status | Evidence |
| --- | --- | --- |
| Within-tx script-group parallelism | **Implemented** | `exec/src/vm/verifier.rs` — `script_groups.par_iter()` inside `verify_with_cycles` |
| CellDAG dependency graph + topological layering | **Implemented** (metadata) | `exec/src/scheduler/dag.rs` — `CellDAG::build` / `build_from_typed` |
| Parallel inter-tx execution (CellDAG + Rayon) | **Implemented** | `exec/src/scheduler/executor.rs` — `ParallelExecutor::execute` wired to `TransactionScriptVerifier`; exercised by `session commit-multi` |
| CKB-VM spawn / IPC (intra-tx, cooperative) | **Implemented** (single-threaded) | `exec/src/vm/scheduler.rs` — `VmScheduler` cooperative loop |
| Mempool conflict scoring | **Implemented** (serial, deterministic) | `mempool/src/cellpool.rs` |
| Mempool batch / parallel admission | **Planned** | see item #2 |
| Consensus precommit verification | **Implemented** (serial, deterministic) | `consensus/src/lib.rs` |
| Consensus parallel signature verification | **Planned** (requires measurements) | see item #6 |
| State root (incremental MuHash) | **Implemented** (O(1) per op) | `state/src/cell_tree.rs` |
| DA Merkle proof generation | **Implemented** (parallel leaf hashing) | `state/src/store/proof.rs` — rayon `par_chunks` |
| Serialization cache (thread-safe LRU) | **Implemented** | `exec/src/serialization/cache.rs` |
| Sighash reused-values cache | **Planned** | see item #4 — `NoCache` exists only in tests |
| Content-addressable VM-result cache | **Planned** | see item #5 |
| Segment-reader cache contention | **Implemented** | `state/src/store/segment.rs` — handle cloned under lock, read outside |

`cli/src/main.rs` remains a large command dispatcher. `exec/src/lib.rs` is a
small module/export facade; VM implementation lives under `exec/src/vm`, and
session, producer, authority, journal, worker and bundle logic are separate
workspace crates. Source-file sizes are not performance evidence. Use `wc -lc`
on the actual checkout if size is relevant to a refactor; historical byte/line
counts are not an architectural contract.

---

## Opportunity list (leverage-ranked)

Each item below carries: rationale, files touched, effort (S / M / L),
risk, whether it touches the VM (always No), and a concrete "done when"
criterion. Items marked **Done** are landed; items marked **Planned**
are next steps that need their own design pass before landing.

### 1. Wire `ParallelExecutor::execute` to real VM verification — Done

The CellDAG builder and the rayon `par_iter` harness over topological
layers already existed, but the executor ran a stub closure and was
never called from any production path. Wiring it to
`TransactionScriptVerifier::verify_with_cycles` turns the DAG from a
metadata structure into a real inter-transaction parallel verifier.

- **Rationale:** independent Cell transactions in a session chunk can
  verify concurrently; this is the headline off-chain concurrency win.
- **Files:** `exec/src/scheduler/executor.rs` (richer `ExecutionReceipt`),
  `exec/src/lib.rs` or a new helper (the `verify_celltx_via_dag` entry
  point), `cli/src/main.rs` (`session commit-multi` command).
- **Effort:** M. **Risk:** low — additive; `TransactionScriptVerifier`
  is already `Send + Sync`. **VM changed?** No.
- **Done when:** `ParallelExecutor::execute` is exercised by a real
  multi-tx test (layer count ≥ 2, per-tx receipts, deterministic
  ordering), and `session commit-multi` exposes DAG-layer stats in its
  report. See [Teeworlds end-to-end](../tutorials/teeworlds-end-to-end.md)
  for the demo path that shows both single- and multi-tx commit.

### 2. Parallelize mempool batch admission — Planned

`CellPool::add` admits one transaction at a time. Conflict detection
(`check_conflicts`, ~`mempool/src/cellpool.rs:250`) and dependency
detection (`find_dependencies`, ~`:316`) are serial scans over a
`BTreeMap`. For a session chunk that admits many txs, this is the next
serial bottleneck after inter-tx verification moves to CellDAG.

- **Rationale:** turn N serial O(pool) scans into one parallel
  conflict-key computation + a single write-lock acquisition. Also lets
  us fix the F-05 size-check-then-insert race in the same critical
  section.
- **Files:** `mempool/src/cellpool.rs` (new `add_many`, reworked `add`),
  `mempool/src/scorer.rs` (no change expected), `mempool/Cargo.toml`
  (add `rayon`).
- **Effort:** M. **Risk:** medium — concurrency correctness; must keep
  the deterministic ordering invariant that `ConflictKey` (fixed-point
  fee density + wtxid tiebreak) guarantees. **VM changed?** No.
- **Done when:** `add_many(&[CellTx])` admits a batch under one write
  lock, conflict keys are computed via `par_iter`, the F-05 race is
  closed (size check and insert share one lock acquisition), and a
  concurrent-stress test asserts `max_size` is never exceeded.

### 3. Parallel DA Merkle leaf hashing — Done

`compute_merkle_root_from_leaves` and the per-level `build_next_level`
hashed leaves serially in pairwise loops. For the 1 GB segment size
(`SEGMENT_SIZE` in `state/src/store/segment.rs`), leaf hashing was a
real cost on large seals.

- **Rationale:** Merkle levels are embarrassingly parallel within a
  level. `par_chunks(2)` per level gives near-linear speedup on
  multi-core with zero API change.
- **Files:** `state/src/store/proof.rs`, `state/Cargo.toml` (add
  `rayon`).
- **Effort:** S. **Risk:** low — pure function, deterministic output.
  **VM changed?** No.
- **Done when:** `compute_merkle_root_from_leaves` and
  `build_next_level` use rayon, and a test with a large leaf set
  confirms the root is byte-identical to the serial implementation.

### 4. Real sighash reused-values cache — Planned

`StandardSigHashReusedValues` defines reusable component hashes for native
Myelin signature hashing. Its `NoCache` implementation is a test helper, not a
production cache. Any cache implementation must bind the exact transaction and
hashing semantics. Measure host CPU and wall time; host hash caching does not
change a CKB script's VM instruction cycles.

- **Files:** `exec/src/celltx/sighash.rs` and its callers.
- **Done when:** a caller uses a transaction-scoped cache, mutation invalidation
  preserves the existing digest vectors, and a representative input-count matrix
  demonstrates a measured host-side improvement.

### 5. Content-addressable VM-result cache — Planned

Every `TransactionScriptVerifier::verify_with_cycles` call re-runs each
script group from scratch. For court replays, re-verification of
unchanged script groups across session blocks, and the devnet smoke
loop, this is repeated work.

- **Rationale:** cache
  `(script_code_hash, args, inputs_hash) -> (cycles, exit_code)` so
  re-verification of unchanged script groups is a lookup. This is the
  biggest throughput win for re-runs and is entirely outside the VM
  (wrap the verifier, key on pre-VM data).
- **Files:** new module under `exec/src/vm/` (e.g. `result_cache.rs`),
  wired into `TransactionScriptVerifier` or the new
  `verify_celltx_via_dag` helper.
- **Effort:** L. **Risk:** medium — cache invalidation is subtle; the
  key must bind every input that affects the result (code, args, all
  input cell data, witness layout, `VmSemantics`, `max_cycles`). **VM
  changed?** No.
- **Done when:** a `VmResultCache` keyed on the full pre-VM input tuple
  short-circuits repeated verification, an invalidation test confirms a
  changed input byte forces a re-run, and a bench shows wall-clock
  improvement on a replay workload.

### 6. Parallel consensus signature verification — Planned

Both committee paths verify real BIP340 secp256k1 signatures through
`verify_schnorr` in `consensus/src/lib.rs`. Verification is serial. Parallelization
must preserve signer uniqueness, deterministic quorum accounting and exact
rejection behavior. Benchmark committee sizes and malformed-signature loads
before changing this path.

- **Files:** `consensus/src/lib.rs`, `consensus/Cargo.toml`.
- **Done when:** measured parallel verification improves the relevant workload
  while existing threshold, identity and malformed-proof cases still pass.

### 7. Segment-reader cache contention — Done

The segment reader held its `files: LruCache<u32, File>` mutex across
`seek` + `read_exact`. A slow disk on one segment serialized all
readers behind that lock (`state/src/store/segment.rs`).

- **Rationale:** release the cache lock before doing I/O by cloning the
  file handle under the lock and reading outside the critical section.
- **Files:** `state/src/store/segment.rs`.
- **Effort:** S. **Risk:** low — `File::try_clone` is cheap; semantics
  unchanged. **VM changed?** No.
- **Done when:** the LRU lock is not held across disk reads, and the
  existing segment read/proof tests pass unchanged.

---

## Non-goals

These are deliberately **not** pursued, to keep the system simple and
its determinism guarantees easy to reason about.

- **No CKB-VM changes.** The VM is a fixed oracle. All work above is
  host-side orchestration, caching, or scheduling.
- **No asynchronous consensus state transition.** Transport and producer timing
  may use Tokio; canonical commit ordering and deterministic execution remain
  explicit synchronous boundaries.
- **No permissionless consensus redesign.** Parallel committee verification is
  a local optimization and does not introduce a new consensus or authority model.
- **No unbenchmarked large refactors.** Items marked Planned land only
  after a design pass and a measurable win; this doc is the queue, not
  a commitment to ship everything as-is.

## Related

- [Production gate](production-gate.md) — where these optimizations are
  exercised under the release-evidence boundary.
- [Teeworlds end-to-end](../tutorials/teeworlds-end-to-end.md) — the
  reference workload that surfaces inter-tx DAG parallelism.
- [First run](../getting-started/first-run.md) — the zero-dependency
  session demo.
