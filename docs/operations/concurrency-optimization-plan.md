# Concurrency & Optimization Plan

> Scope: host-side (off-chain) concurrency and throughput work that does
> **not** modify CKB-VM. The VM is treated as a fixed verification oracle.

This document is the single source of truth for what parallelism and
optimization work is in flight, done, or planned for the Myelin runtime.
It is written so a contributor can pick an item and know its rationale,
touch points, and "done when" criterion before starting.

## Current state (as of 2026-07)

Myelin is a **synchronous library** that uses **rayon data-parallelism**
in a small number of well-defined places. There is deliberately no
async runtime (no tokio / async-std) in the runtime crates; the session
loop is a straight call chain in the CLI. This is a design choice, not a
gap — see [Non-goals](#non-goals).

| Dimension | Status | Evidence |
| --- | --- | --- |
| Within-tx script-group parallelism | **Implemented** | `exec/src/vm/verifier.rs` — `script_groups.par_iter()` inside `verify_with_cycles` |
| CellDAG dependency graph + topological layering | **Implemented** (metadata) | `exec/src/scheduler/dag.rs` — `CellDAG::build` / `build_from_typed` |
| Parallel inter-tx execution (CellDAG + Rayon) | **Implemented** | `exec/src/scheduler/executor.rs` — `ParallelExecutor::execute` wired to `TransactionScriptVerifier`; exercised by `session commit-multi` |
| CKB-VM spawn / IPC (intra-tx, cooperative) | **Implemented** (single-threaded) | `exec/src/vm/scheduler.rs` — `VmScheduler` cooperative loop |
| Mempool conflict scoring | **Implemented** (serial, deterministic) | `mempool/src/cellpool.rs` |
| Mempool batch / parallel admission | **Planned** | see item #2 |
| Consensus precommit verification | **Implemented** (serial, deterministic) | `consensus/src/lib.rs` |
| Consensus parallel signature verification | **Planned** (waits on real crypto) | see item #6 |
| State root (incremental MuHash) | **Implemented** (O(1) per op) | `state/src/cell_tree.rs` |
| DA Merkle proof generation | **Implemented** (parallel leaf hashing) | `state/src/store/proof.rs` — rayon `par_chunks` |
| Serialization cache (thread-safe LRU) | **Implemented** | `exec/src/serialization/cache.rs` |
| Sighash reused-values cache | **Planned** | see item #4 — `NoCache` placeholder today |
| Content-addressable VM-result cache | **Planned** | see item #5 |
| Segment-reader cache contention | **Implemented** | `state/src/store/segment.rs` — handle cloned under lock, read outside |

The single biggest structural caveat (out of scope for "no VM changes",
flagged here for visibility): `cli/src/main.rs` is a ~17.5k-line single
translation unit and `exec/src/lib.rs` is ~1.1 MB. Splitting either is a
prerequisite for rustc's own parallel codegen and for editor
responsiveness, but it does not affect runtime concurrency.

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

`exec/src/celltx/sighash.rs` ships a `NoCache` placeholder for CKB's
`StandardSigHashReusedValues`. The structural hook exists; nothing is
cached today, so repeated sighash sub-computations across inputs of the
same tx are recomputed.

- **Rationale:** fill the placeholder with the real reused-values cache
  that CKB-VM's lock-script path expects. Pure host-side optimization.
- **Files:** `exec/src/celltx/sighash.rs`.
- **Effort:** M. **Risk:** low if it mirrors the ckb-std / ckb-vm
  reference shape exactly. **VM changed?** No.
- **Done when:** `NoCache` is replaced by a `ReusedValues` impl that
  caches the documented sub-fields, and the teeworlds VM probe cycle
  count is measured before/after to confirm it does not regress.

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

### 6. Parallel consensus signature verification — Planned (waits on real crypto)

Both consensus engines verify precommit certificates with a serial
`for signature in &certificate.signatures` loop. Today the "signatures"
are deterministic blake3 digests (explicitly a closed-committee
development signature, `consensus/src/lib.rs`), so the cost is trivial
and parallelism is not worth it yet.

- **Rationale:** once the deterministic-blake3 stubs are replaced with
  real secp256k1 / BLS, signature verification becomes the hot path and
  the loop parallelizes trivially with rayon.
- **Files:** `consensus/src/lib.rs`, `consensus/Cargo.toml`.
- **Effort:** S (once crypto lands). **Risk:** low — embarrassingly
  parallel, preserve stable ordering for the quorum accumulation. **VM
  changed?** No.
- **Done when:** certificate verification uses `par_iter`, quorum weight
  accumulation stays deterministic, and a test with N validators
  confirms correct accept/reject under threshold.

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
- **No async runtime.** Myelin stays synchronous + rayon. Introducing
  tokio/async-std would touch every crate, complicate the deterministic
  finality model, and is not justified by any current bottleneck — the
  session loop is CPU-bound, not I/O-bound.
- **No permissionless consensus concurrency.** Committee signature
  fan-out (#6) is gated on real cryptography arriving first; the closed
  committee today does not benefit.
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
