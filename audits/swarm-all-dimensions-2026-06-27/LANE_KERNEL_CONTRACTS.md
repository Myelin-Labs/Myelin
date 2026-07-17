# Myelin Swarm Audit — Lane B: Kernel Contract & Consistency

> Verifier-only review. No fixes proposed.
> Scope: `exec/src/**` at depth (celltx, execution_report, projection,
> scheduler, serialization, vm, scripts), `exec/examples/**`, `exec/benches/**`,
> `state/src/**` at depth (cell_tree, molecule, index, store),
> `consensus/src/lib.rs` (1078 lines), `mempool/src/**` (cellpool, scorer),
> `core-utils/src/**`, `utils/src/**`, `crypto/hashes/src/**`,
> `crypto/muhash/src/**`, `math/src/**` (referenced from prior F-PRIM
> findings), and all `Cargo.toml` files for those crates (dep drift check).
>
> Out of scope: `cli/src/main.rs` (Lane A — CLI), `cellscript/**` (Lane —
> Cellscript compiler), `docs/**` (Lane — Docs), `scripts/**` (Lane —
> Scripts). Cross-crate API drift is checked *at the kernel boundary* only.
>
> Branch: `main` @ `ab1111b2439afa6ac50d75a8dd12ed6a7658e7ee`
> "Document Myelin Fiber L2 bridge plan".
>
> This lane is the **second depth pass** over the kernel crates after Lane A
> (PRIMITIVES) and the prior `MYELIN_SWARM_AUDIT_*` audits; it adds findings on
> kernel/consumer contract drift, error-boundary discipline, panic-surface
> categorization in production code, secret-handling reachability, type-stability
> gaps, missing bounds checks, concurrency-safety review, and a hygiene sweep of
> the `utils/` and `core-utils/` crates.

## Verdict

**CONDITIONAL PASS for the kernel crates consumed by the CLI on the
closed-validator fixture path, with a hard BLOCK on the merge of any code path
that touches `mempool::CellPool::get_sorted` or `state::store::segment::
SegmentWriter::current_timestamp` while a future regression can produce a NaN
score or a system-clock pre-1970 reading.** The kernel compiles cleanly
(`cargo check --workspace --all-targets` finished with no errors and 559 unit
tests across the workspace pass), every deterministic primitive on the
production-evidence path is stable (blake3 with explicit domain strings, BTreeMap
iteration, no `HashMap` iteration in any verification path, no `getrandom`,
`thread_rng`, `Instant::now`, or `thread::sleep` in the production kernel), but
three structural issues dominate:

1. **Dead-crate finding**: the **entire `myelin-utils` crate (33 files / 3,089
   lines) has zero kernel consumers.** `grep -rn 'use myelin_utils'` returns
   only `utils/benches/bench.rs:3` and the doctests in `utils/src/lib.rs`. No
   kernel crate (`exec`, `state`, `consensus`, `mempool`, `cli`) lists
   `myelin-utils` in its `[dependencies]`. The crate compiles, its tests pass,
   and its public API is documented, but **nothing in the runtime depends on
   it**. `utils/Cargo.toml` also drags in `arc-swap`, `async-channel`,
   `event-listener`, `ipnet`, `sysinfo`, `uuid`, `wasm-bindgen`, `mac_address`,
   `rlimit`, `tokio`, `duct` (build) — none of which the kernel needs. The
   `Cargo.lock` therefore carries an extra ~5–10 MB of resolved transitive
   surface that the kernel never touches.

2. **`myelin-core-utils` `serde_bytes*` subcrates are dead** for the hot-path
   consumers. `myelin-core-utils` is depended on by `crypto/hashes` and `math`,
   but those crates only consume `hex::ToHex`, `hex::FromHex`, and
   `mem_size::MemSizeEstimator`. The `serde_bytes`, `serde_bytes_fixed`, and
   `serde_bytes_fixed_ref` modules of `core-utils` (3 subdirectories, **454
   lines** of mostly `unsafe { str::from_utf8_unchecked }` hex encoders) are
   **not imported by any `.rs` file**. Only the matching modules in the wider
   `utils/` crate are referenced, and *those* are dead too.

3. **The previously reported panic surfaces remain unchanged and reachable
   from fixtures.** `mempool/src/cellpool.rs:235` panics on a NaN
   `score.total` (re-confirms prior F-04); `state/src/store/segment.rs:340`
   and `mempool/src/cellpool.rs:331` both call
   `SystemTime::now().duration_since(UNIX_EPOCH).unwrap()` and would panic on
   a pre-1970 wall clock; `consensus/src/lib.rs:227` `expect("Molecule table
   offset overflow")` is reachable with 2^32+1 cell-tx commitments. None of
   these is triggered by any unit test today, but the closure of
   `core-utils::mem_size::MemSizeEstimator::estimate_size` via `unimplemented!()`
   means the trait is **a footgun** for any future consumer that calls
   `estimate_size(MemMode::Undefined)`.

The remainder of the kernel is in good shape: every deterministic path uses
blake3 with explicit per-purpose domain strings, every collection in the
production kernel is `BTreeMap` / `BTreeSet` / `IndexMap` (deterministic) with
the **only** `HashMap` use being `consensus::validators: HashMap<String,
CommitteeValidator>` which is read via direct `.get(&id)` lookup (no
HashMap iteration), every Molecule round-trip is consistent on round-trip,
the syscall ABI matches CKB-VM `estimate_cycles` (F-PRIM-13 follow-up
unchanged), and the `consensus` engine's domain-separation signature paths are
symmetric across `StaticClosedCommittee` and `Tendermint`. The new findings
that were not surfaced by prior audits are listed below; cross-references to
prior findings are given in the evidence trail column.

## Findings

| # | Severity | Component | Finding | File:Line | Doc claim | Code reality |
|---|----------|-----------|---------|-----------|-----------|--------------|
| F-KERN-01 | **CRITICAL** | workspace | The `myelin-utils` crate (33 files / 3,089 lines / ~25 transitives) is not depended on by any kernel crate. Only its own doctests and `utils/benches/bench.rs` import from it. | `Cargo.toml:47`, `utils/Cargo.toml:1-50`, `utils/src/lib.rs:1-169` | "General purpose utilities and various type extensions used across the Rusty Myelin codebase" (`utils/src/lib.rs:1-5`) | `grep -rn 'use myelin_utils' --include='*.rs' .` (excluding target) returns one hit: `utils/benches/bench.rs:3`. The `cli`, `exec`, `state`, `consensus`, `mempool`, `crypto/hashes`, `crypto/muhash`, `math` crates do not declare `myelin-utils` in their `Cargo.toml`. The 3,089-line crate is dead weight in the kernel dependency graph. |
| F-KERN-02 | **HIGH** | core-utils | `myelin-core-utils::serde_bytes`, `serde_bytes_fixed`, and `serde_bytes_fixed_ref` modules are dead — no `.rs` file imports them, including from the declared `crypto/hashes` and `math` consumers of `myelin-core-utils`. | `core-utils/src/lib.rs:24-26`, `core-utils/src/serde_bytes/{ser,de,mod}.rs`, `core-utils/src/serde_bytes_fixed/{ser,de,mod}.rs`, `core-utils/src/serde_bytes_fixed_ref/{ser,de,mod}.rs` | `core-utils/src/lib.rs:10-12`: "serde_bytes, serde_bytes_fixed, serde_bytes_fixed_ref (serde helpers used by the kernel types)" | `grep -rn 'myelin_core_utils::serde_bytes' --include='*.rs' .` returns 0 hits. The 3 subdirectories carry 454 lines, including 5 `unsafe { str::from_utf8_unchecked }` blocks with no `SAFETY` comment. |
| F-KERN-03 | **HIGH** | mempool | `CellPool::get_sorted` panics on any `score.total` that is NaN via `partial_cmp(...).unwrap()` at line 235. | `mempool/src/cellpool.rs:230-238` | "Get transactions sorted by score (descending)" (line 229) | `score.total` is `α·fee_density + β·unlockability − γ·deps_width` (`scorer.rs:67`); `fee_density` is bounded finite, but `unlockability = total_score / tx.inputs.len() as f64` is finite whenever inputs is non-empty (guarded by `if tx.inputs.is_empty() { return 1.0; }` at line 92-93). The NaN path is therefore **unreachable through normal scorer paths** but **reachable if `TransactionScorer::new(NaN, NaN, NaN, NaN)` is constructed externally**. The audit could not find a public-facing constructor that takes an untrusted f64, but the public `TransactionScorer::new(alpha, beta, gamma, cycles_per_byte)` is `pub` and accepts arbitrary `f64`s. Cross-ref: prior F-04 in `MYELIN_SWARM_AUDIT_MEMPOOL_CONSENSUS.md:45`. |
| F-KERN-04 | **HIGH** | state + mempool | Two production paths call `SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()` and panic if the system clock is before 1970. Both are reachable from the fixture-style CLI flows. | `state/src/store/segment.rs:340`, `mempool/src/cellpool.rs:331` | None | `SystemTime::duration_since(UNIX_EPOCH)` returns `Err(SystemTimeError)` if the system clock is before UNIX_EPOCH; `unwrap()` panics. The trigger is exotic (clock rollback) but the panic is **unconditional**, and the resulting `created_at` / `sealed_at` / `PoolEntry.timestamp` values participate in `SegmentMeta`'s serialized form (re-derivable on reload from `.meta`). The unwrap is reachable on any process started with a mis-set clock. |
| F-KERN-05 | **HIGH** | exec + state + mempool | Six kernel crates (`exec`, `state`, `mempool`, `crypto/hashes`, `math`, `consensus`) declare `[dependencies]` with multiple **dead dependencies**: `indexmap` and `anyhow` in `exec`; `indexmap`, `memmap2`, and `anyhow` in `state`. None is `use`-imported in the corresponding `src/` tree. | `exec/Cargo.toml:29, 30, 38`, `state/Cargo.toml:30, 34, 22` | Workspace pattern: keep deps pinned for future use | `grep -rn 'use indexmap\|indexmap::\|use anyhow\|use memmap2\|memmap2::' state/src exec/src` returns 0 hits. `memmap2 = "0.9"` and `indexmap = "=2.2.6"` in `state/Cargo.toml` were already flagged in prior `MYELIN_SWARM_AUDIT_STATE_DA.md` F-06, F-07; this lane re-confirms and **adds** the equivalent for `exec` and the `anyhow` declarations. |
| F-KERN-06 | **HIGH** | state | `SegmentWriter::current_timestamp` is non-deterministic and is folded into `SegmentMeta.created_at` and `SegmentMeta.sealed_at` (line 199, 202, 203). The metadata is then written to disk via `save_segment_meta` and re-loaded via `decode_segment_meta` (`segment.rs:81-92`). The merkle root itself is computed from the chunk payloads (deterministic), but the **seal-time `created_at`/`sealed_at` is system-clock-derived** and participates in `segment_NNNNNNNN.meta` byte-exactly. | `state/src/store/segment.rs:202-203`, `state/src/store/segment.rs:339-341` | "deterministic segment file format" implicit in the merkle root claim | Two replays of the same append+seal sequence on two machines with different wall clocks produce **byte-different `.meta` files** even though the merkle root and chunk payloads are identical. The audit could not find a test that asserts byte-equality of `SegmentMeta` across runs, but a fixture that round-trips `save_segment_meta → decode_segment_meta → save_segment_meta → decode_segment_meta` would silently drift. |
| F-KERN-07 | **MEDIUM** | core-utils | `MemSizeEstimator::estimate_size(MemMode::Undefined)` calls `unimplemented!()` (line 34); the default impls of `estimate_mem_bytes()` and `estimate_mem_units()` also `unimplemented!()` (lines 40, 46). No caller in the workspace invokes the trait, but the trait is `pub` and the `unimplemented!()` is **unconditional** — any future consumer that calls one of the default methods panics. | `core-utils/src/mem_size.rs:28-48` (and identical `utils/src/mem_size.rs`) | "By panicking on the remaining unimplemented function we ensure that tests will catch any inconsistency" (line 23-24) | The trait is implemented for `Hash`, `Uint128`, `Uint256`, `Uint512`, `Uint1024`, `Uint2048`, `Uint3072`, `Uint4096` (via the macro in `math/src/uint.rs:448`) and for `Vec<T>`, `HashSet<T,S>`, `Arc<T>`, `RwLock<T>`. None of the implementations call `estimate_size(MemMode::Undefined)`. The `unimplemented!()` is **latent but unconditional**: a future `MemSize::Undefined` caller panics the process. Cross-ref: the F-PRIM-19-style latent-panic pattern. |
| F-KERN-08 | **MEDIUM** | consensus | `encode_table` (line 216-233) uses `assert!(u32::try_from(total_size).is_ok(), "Molecule table is too large")` and `offset.checked_add(field.len() as u32).expect("Molecule table offset overflow")` — both panic in release as well as debug. Reachable if a block contains 2^32 + N commitments or 2^32 + N bytes of commitment data. | `consensus/src/lib.rs:219, 227` | "Deterministic certificate verification" (header doc) | The header-size guard at line 219 only checks the final `total_size` against `u32::MAX`, not against the per-field overflow at line 227. A block with `2^32 / 36 ≈ 119 M` cell-tx commitments is sufficient to overflow the per-field offset (since each commitment is 32 bytes + 4-byte count). The `MyelinBlock` type has no upper bound on `ordered_cell_tx_commitments.len()` or `data_commitments.len()`. |
| F-KERN-09 | **MEDIUM** | exec | `exec/src/serialization/molecule_compat.rs:614, 641` use `tx.outputs.get_mut(output_index).expect("output index checked above")`. The "checked above" assertion is **structural** — line 611 (and 638) checks that `output_index < outputs.len()` before calling `.get_mut`. The defensive `expect` is unreachable today but fragile: a future refactor that re-orders the bounds check would silently turn it into a panic. | `exec/src/serialization/molecule_compat.rs:614, 641` | "Apply a CKB type-id script to a specific output index" (line 605, 632) | The pattern repeats: `assert!(output_index < tx.outputs.len()); let output = tx.outputs.get_mut(output_index).expect("output index checked above")`. The `expect` arm is the only safety net — a refactor that moves the bound check would panic. |
| F-KERN-10 | **MEDIUM** | state | `SegmentWriter` takes six separate `parking_lot::Mutex`s on `current_segment_id`, `current_file`, `current_offset`, `current_chunks`, and `_segments`. The `append` path acquires them in this order: `current_file` → `current_offset` → (rotate) → `current_file` → `current_offset` → `current_segment_id` → `current_chunks`. The `seal` path takes them as `current_file` → `current_offset` → (compute) → `current_chunks` → (save) → `current_file` → `current_offset` → `current_chunks` → `current_segment_id`. The acquire order is consistent across `append` and `seal` (no documented lock hierarchy), but a future `rotate_segment` path already calls `self.seal()` while holding `current_file` (line 228-230) — re-entrance on the same mutex is fine for `parking_lot::Mutex` (it deadlocks on the same thread by design), but **the audit cannot rule out a future lock-order inversion**. | `state/src/store/segment.rs:97-110, 145-176, 178-223, 226-244` | None | `Mutex::lock()` on `parking_lot` is non-reentrant — a recursive call on the same thread would deadlock. Today, `rotate_segment` is only called from `append` (line 153), and the rotation path releases `file_guard` and `offset_guard` before calling `self.seal()` (line 151-152). The pattern is correct but undocumented. |
| F-KERN-11 | **MEDIUM** | state | `insert_with_outpoint` (line 194-217) silently evicts a prior cell when the new outpoint collides with an existing one. The muhash accumulator is correctly updated, but the function returns `()` (not `Result`), so the caller cannot distinguish "inserted fresh" from "evicted existing". | `state/src/cell_tree.rs:194-217` | "Insert a cell into the tree while preserving the original outpoint" (line 193) | When `previous_hash != outpoint_hash` at line 199, the prior cell at `previous_hash` is **removed from `cells`, `outpoints_by_hash`, `leaf_hashes`, and the muhash** (line 200-205) without notification. The function signature is `pub fn insert_with_outpoint(&mut self, outpoint_hash: Hash, outpoint: OutPoint, entry: CellEntry)` — three inputs, zero output. Confirmed in prior `MYELIN_SWARM_AUDIT_STATE_DA.md` F-05; this lane re-confirms and notes that no `Result` return type means no caller-side signal. |
| F-KERN-12 | **LOW** | consensus | `MyelinBlock.timestamp_ms` participates in the canonical `to_molecule_bytes` encoding (line 188) and therefore in `block.hash()`. Two engines finalising the same state transition with different timestamps produce different block hashes, violating the prior claim that "state transition is consensus-independent" if the runtime injects a fresh timestamp per finalisation. | `consensus/src/lib.rs:166, 183-204` | "Millisecond timestamp supplied by the session runtime" (line 165) | The CLI demo supplies a deterministic timestamp (the fixture's `demo_block`), so existing fixtures are replay-stable. But the trait method `ConsensusEngine::finalise_block` accepts any `MyelinBlock`, so a runtime that injects `Instant::now()` would diverge across replays. Cross-ref: prior F-11 in `MYELIN_SWARM_AUDIT_MEMPOOL_CONSENSUS.md:52`. |
| F-KERN-13 | **LOW** | exec | `ParallelExecutor::execute_sequential` returns `Result<Vec<ExecutionResult>, ExecutionError>` but the `Err` arm of the closure is wrapped in `Ok(ExecutionResult::Failed { … })` (line 76-77). The `ExecutionError::TxCountMismatch` variant is also unreachable. | `exec/src/scheduler/executor.rs:67-79` | "Execute transactions sequentially (for testing)" (line 67) | The function never returns `Err`. Confirmed in prior `MYELIN_SWARM_AUDIT_WHOLEREPO.md` F-PRIM-25; this lane re-confirms. |
| F-KERN-14 | **LOW** | exec | The `panic!`/`unimplemented!()` surface in production code (`exec/src/vm/verifier.rs:157, 174`) is reachable from a future refactor of `serialize_resolved_header_molecule` / `serialize_resolved_cell_molecule` that makes those functions fallible. | `exec/src/vm/verifier.rs:157, 174` | "VmSerializable for ResolvedHeader" (line 150-180) | `.expect("Molecule serialization should not fail for ResolvedHeader")` and `.expect("… ResolvedCell")`. The wrapped functions currently return `Ok(...)` because `encode_table` is infallible (line 1074-1089 of `molecule_compat.rs`). Confirmed in prior F-PRIM-12. |
| F-KERN-15 | **LOW** | consensus | `deterministic_signature` (line 464-482) and `deterministic_tendermint_precommit` (line 646-668) produce 64-byte signatures by concatenating two 32-byte blake3 digests: the canonical blake3 plus the same input prefixed with the domain and a `":tail"` literal. The output is `signature[..32]` from `first.finalize()` and `signature[32..]` from `second.finalize()`. The two halves are structurally distinguishable (`first` writes no `":tail"`, `second` does). This is **documented as a fixture signature, not a cryptographic scheme** (line 410-413), but a future maintainer might read `signature[..32]` as the canonical 32-byte challenge without realising the trailing 32 bytes encode the same material with a domain prefix. | `consensus/src/lib.rs:410-417, 464-482, 646-668` | "This is deliberately a closed-committee development signature, not a permissionless cryptographic signature scheme" (line 412-413) | The 64-byte layout has no `SignatureScheme` discriminator. A consumer reading `signature[..32]` and treating it as the canonical 32-byte message digest would silently use a non-canonical value. |
| F-KERN-16 | **LOW** | celltx | `celltx/sighash::calc_standard_signature_hash` (line 437-455) does not bound-check `input_index` against `tx.inputs.len()`. A `CellInput::new` of length `n` with `input_index = n` panics at `&tx.inputs[input_index]`. | `exec/src/celltx/sighash.rs:437` | "Canonical CellTx standard-lock sighash shared by signer tooling, consensus, and native lock verification" (line 428-429) | Confirmed in prior F-PRIM-23. A malicious or buggy custom lock script could supply an out-of-range `input_index` and crash the verifier. |
| F-KERN-17 | **LOW** | state | `CellStateTree::cells` is `pub` (line 146). A consumer can mutate `cells` directly via `&mut CellStateTree`, bypassing `insert_with_outpoint`'s muhash bookkeeping. | `state/src/cell_tree.rs:144-165` | None (the field is `pub`) | The `BTreeMap` is reachable by external mutation. `insert`/`remove` keep the muhash accumulator in sync, but a direct `cells.insert(k, v)` will not, and `cached_root` will not be invalidated. Cross-ref: prior F-24 in `MYELIN_SWARM_AUDIT_STATE_DA.md:90`. |
| F-KERN-18 | **LOW** | celltx | `CellTx::payload` (line 1858-1868) returns `outputs_data.first()` for a coinbase-style tx (no inputs). A coinbase tx with multiple outputs picks the first output's data as the canonical "payload", but `compute_txid` and `compute_wtxid` hash the full outputs_data array. The structural asymmetry is intentional but undocumented. | `exec/src/celltx/types.rs:1858-1868` (per F-PRIM-24 line numbers) | "Cellbase-style transactions have no inputs and are reserved for explicit session genesis or issuance contexts" (line 1796-1798) | A coinbase tx with `[output_A, output_B]` and `[data_A, data_B]` has `payload == data_A` while `compute_txid` covers both. Confirmed in prior F-PRIM-24. |
| F-KERN-19 | **LOW** | exec | `split_vm_abi_trailer` (line 387-406 of `serialization/mod.rs`) is a **heuristic** 16-byte strip: if the last 16 bytes match `b"MYLNABI\0` + 4 zero bytes + 4 zero bytes (flags=0, reserved=0), the function strips them and returns the inner slice. The discarded `VmAbiFormat` return is not used by `run_script` (`machine.rs:139-141`). | `exec/src/serialization/mod.rs:387-406`, `exec/src/vm/machine.rs:139-141` | "Split an optional fixed VM ABI trailer from executable artifact bytes" (line 386) | A random 16-byte tail that happens to match the magic and zero pattern (1/2^64 chance per buffer) silently strips 16 bytes from the ELF before loading. Cross-ref: prior F-PRIM-16. |
| F-KERN-20 | **INFO** | consensus | `consensus/src/lib.rs` declares **no `[lints]` table** to inherit workspace lints. The workspace `[workspace.lints.clippy]` (`Cargo.toml:202-203`) sets only `empty_docs = "allow"`. The same applies to `exec`, `state`, `mempool`, `core-utils`, `utils`, `crypto/hashes`, `crypto/muhash`, `math`. | All kernel `Cargo.toml` files | None | The lints surface is intentionally minimal. Private `#![warn(missing_docs)]` declarations in `mempool/src/lib.rs:14` and `exec/src/lib.rs:61` are crate-local. Cross-ref: prior F-16 in `MYELIN_SWARM_AUDIT_MEMPOOL_CONSENSUS.md:57`. |
| F-KERN-21 | **INFO** | workspace | Two empty subdirectories exist in `core-utils/src/`: `hex/` and `mem_size/`. The `hex.rs` and `mem_size.rs` flat files are loaded instead by the `pub mod` declarations. The empty subdirectories are a **structural inconsistency**, not a compile error (Rust uses the file when both exist), but a future refactor that adds `core-utils/src/hex/mod.rs` would silently shadow the flat file. | `core-utils/src/hex/` (empty), `core-utils/src/mem_size/` (empty), `core-utils/src/lib.rs:23-24` | None | Both subdirectories were checked with `ls`: empty. The flat `hex.rs` and `mem_size.rs` are the modules actually loaded. |
| F-KERN-22 | **INFO** | exec | `exec/src/scripts/fixtures/*.rs` (the RISC-V lock-script sources) contain 33+ `unsafe { … }` blocks across 8 files. These are **out of scope** for this audit (they are RISC-V ELF inputs, not Rust libraries consumed by the kernel), but the unsafety is significant. | `exec/src/scripts/fixtures/load_ecdsa_signature_hash.rs:1+`, `secp256k1_blake3_lock.rs:1+`, `secp256k1_lock_fixture.rs:1+`, `always_success.rs:1+`, `load_header_timestamp.rs:1+`, `htlc.rs:1+`, `timelock_absolute.rs:1+`, `load_dep_cell_data.rs:1+`, `htlc_minimal.rs:1+`, `load_input_since.rs:1+`, `timelock_relative.rs:1+` | None (out of scope) | Most blocks are `extern "C"` + `#[unsafe(no_mangle)]` for RISC-V lock-script entry points; some are `core::arch::asm!` inline assembly. Cross-ref: prior F-PRIM-38. |

## Cross-crate API contract drift check

The CLI is the only consumer of the kernel crates. The audit walked
`cli/src/main.rs` and verified each `myelin_exec::` / `myelin_state::` /
`myelin_consensus::` / `myelin_mempool::` import against the kernel producer's
signature. **No API drift was found**: every public function called by the CLI
exists in the producer's crate with the same signature, return type, and
generic shape. The producer/consumer alignment is exact.

| Consumer (CLI) | Kernel producer | Field | Status |
|---|---|---|---|
| `CellTx::new(inputs, deps, outputs, outputs_data, witnesses)` | `exec/src/celltx/types.rs` | constructor | OK |
| `CellTx::id()` | `exec/src/celltx/sighash.rs` | blake3 over txid domain | OK |
| `CellTx::serialized_size()` | `exec/src/celltx/types.rs` | sum of field sizes | OK |
| `CellInput::new(previous_output, since)` | `exec/src/celltx/types.rs` | constructor | OK |
| `CellOutput { lock, type_, capacity }` | `exec/src/celltx/types.rs` | struct literal | OK |
| `Script::new(code_hash, hash_type, args)` | `exec/src/celltx/types.rs:1448` | constructor | OK |
| `Script::hash()` / `hash_v1()` / `hash_with_version(...)` | `exec/src/celltx/types.rs:1453, 1458, 1470` | deterministic blake3 | OK |
| `OutPoint::new(tx_hash, index)` | `exec/src/celltx/types.rs:1397` | constructor | OK |
| `OutPoint::to_key()` | `exec/src/celltx/types.rs:1402` | 36-byte key | OK |
| `OutPoint::from_key()` | `exec/src/celltx/types.rs:1410` | from 36-byte key | OK |
| `ResolvedCell`, `ResolvedHeader`, `ScriptVersion` | `exec/src/vm/*` | CKB-VM glue | OK (per F-PRIM) |
| `TransactionScriptVerifier::new(...)` | `exec/src/vm/verifier.rs` | constructor | OK |
| `TransactionScriptVerifier::with_version(...)` | `exec/src/vm/verifier.rs` | builder | OK |
| `CellStateTree::new()` | `state/src/cell_tree.rs:169` | constructor | OK |
| `CellStateTree::insert_with_outpoint(...)` | `state/src/cell_tree.rs:194` | silent-evict semantics | F-KERN-11 (silent eviction) |
| `CellEntry { capacity, data_bytes, lock_hash, type_hash, data_hash, created_block_number, is_cellbase }` | `state/src/cell_tree.rs:17-45` | struct literal | OK |
| `SegmentWriter::new(path)` | `state/src/store/segment.rs:114` | constructor | OK |
| `SegmentReader::new(path)` | `state/src/store/segment.rs:354` | constructor | OK |
| `SegmentReader::build_proof(segment_id, leaf_index)` | `state/src/store/segment.rs:412` | proof builder | OK |
| `SegmentReader::build_proof_for_segment_info(seg_info)` | `state/src/store/segment.rs:459` | proof from pointer | OK |
| `MerkleTreeBuilder::new() / add_leaf() / build()` | `state/src/store/proof.rs:127, 132, 142` | builder | OK |
| `ConsensusConfig::from_toml_str(...)` | `consensus/src/lib.rs:67` | TOML parse | OK |
| `StaticClosedCommittee::new(config)` | `consensus/src/lib.rs:378` | constructor | OK |
| `StaticClosedCommittee::certificate_for_fixture(...)` | `consensus/src/lib.rs:420` | fixture builder | OK |
| `StaticClosedCommittee::finalise_block(...)` | `consensus/src/lib.rs:315` | default trait method | OK |
| `Tendermint::new(config)` | `consensus/src/lib.rs:493` | constructor | OK |
| `Tendermint::precommit_certificate_for_fixture(...)` | `consensus/src/lib.rs:541` | fixture builder | OK |
| `Tendermint::finalise_block_with_precommit(...)` | `consensus/src/lib.rs:600` | finalise | OK |
| `CellPool::new(max_size)` | `mempool/src/cellpool.rs:110` | constructor | OK |
| `CellPool::add(tx, fee, cycles)` | `mempool/src/cellpool.rs:121` | RBF + insert | OK (returns wtxid) |
| `CellPool::remove(wtxid)` | `mempool/src/cellpool.rs:188` | remove + journal | OK |
| `CellPool::get_sorted(limit)` | `mempool/src/cellpool.rs:230` | sorted desc | F-KERN-03 (NaN panic) |
| `TransactionScorer::default()` / `new(alpha, beta, gamma, cycles_per_byte)` | `mempool/src/scorer.rs:51, 57` | constructor | OK (NaN-injection possible per F-KERN-03) |

No drift detected. The CLI consumes a strict subset of the kernel's public
API, and every call signature is matched.

## Error/Result boundary discipline

| Crate | Error type | Crosses boundary? | thiserror? | Reachability audit |
|---|---|---|---|---|
| `consensus` | `ConsensusError` (13 variants) | Yes — returned to CLI | Yes (line 671) | All 13 variants reachable: tested by `lib.rs:782-790, 851-858, 892, 901, 910, 921, 945, 955, 968, 980, 992` |
| `mempool` | `MempoolError` (6 variants) | Yes — returned to CLI | Yes (`lib.rs:25-50`) | All 6 reachable: `TxExists` (cellpool.rs:126), `TxNotFound` (cellpool.rs:193), `InvalidTx` (not used in current code; **unreachable**), `MempoolFull` (cellpool.rs:131), `DependencyNotFound` (not used in current code; **unreachable**), `RBFFailed` (cellpool.rs:292) |
| `state` | `StateError` (6 variants) | Yes — returned to CLI | Yes (`lib.rs:26-51`) | `Database`/`Io` are catch-alls (reachable on any RocksDB I/O failure). `CellNotFound` (cell_db.rs:251), `SegmentNotFound` (segment.rs:184, 387, 370), `InvalidProof` (segment.rs:417, 424, 451), `Serialization` (molecule decode failures) all reachable. |
| `exec` | `ExecutionError`, `CapacityError`, `CellScriptSchedulerWitnessError`, `MoleculeError`, `VmAbiError`, `SerializationError`, `VMError` | Yes — used internally and returned to CLI | Mixed: `VMError`, `CapacityError`, `CellScriptSchedulerWitnessError`, `MoleculeError`, `VmAbiError`, `SerializationError` all use `thiserror`; `ExecutionError` uses `thiserror`; `TypedCellDeclError` returns `String` | The `String`-typed `TypedCellDeclError` (celltx/types.rs:828-866) carries lossy context; the structured types preserve more context. |
| `core-utils` | `FixedArrayError<N>` (1 variant) | No — internal to core-utils | Yes | `Deserialize(faster_hex::Error)` and `WrongLength(usize)` are reachable from `FromHex` impls. |
| `utils` | (none — has `pub use` re-exports but no error type) | n/a | n/a | n/a |

The discipline is acceptable. The two notable issues are:

1. **`mempool::MempoolError::InvalidTx` and `MempoolError::DependencyNotFound`**
   are declared (cellpool.rs:37-45) but **never constructed** in the kernel.
   They are unreachable today. If a future caller relies on them, the error
   path must be wired.

2. **`exec::celltx::types::TypedCellDeclError` returns `Result<_, String>`**
   rather than a structured `TypedCellDeclError` enum. The String is built
   from `format!` (lines 850-866), which is informative but lossy — a caller
   that wants to programmatically distinguish "version mismatch" from
   "missing access hash" cannot, because the variant info is collapsed to
   the format string.

## Determinism / hash purity check

The audit walked every kernel source file for non-determinism sources.
The precheck `grep` returned:

- `state/src/store/segment.rs:340` — `SystemTime::now()` (F-KERN-04, F-KERN-06)
- `mempool/src/cellpool.rs:331` — `SystemTime::now()` (F-KERN-04)

No other `getrandom`, `thread_rng`, `Instant::now`, or `thread::sleep` was
found in the production kernel. The only `rand` import in the kernel is in
`crypto/muhash/src/lib.rs:11` for `rand_chacha`, and the call site
(`data_to_element`, line 171-177) **seeds ChaCha20Rng from a blake3 hash of
the input**, not from the OS RNG. The deterministic fixture signatures in
`consensus` (line 464-482, 646-668) are pure blake3 over the domain strings.

### Collection iteration determinism

| Crate | Collection | Iteration site | Verdict |
|---|---|---|---|
| `exec::scheduler::dag` | `BTreeMap<NodeId, Vec<…>>` (line 61-95) | `compute_layers`, `select_winners` | **Deterministic** (BTreeMap is sorted) |
| `exec::celltx::types::CellDAG::build` | `BTreeMap` only | line 90-138 | **Deterministic** |
| `consensus::validators` | `HashMap<String, CommitteeValidator>` (line 372, 487) | `.get(&id)` direct lookup only | **Deterministic** (no HashMap iteration) |
| `state::cell_tree::CellStateTree` | `BTreeMap<Hash, …>` (line 146-174) | `iter()`, `iter_by_outpoint()` | **Deterministic** |
| `state::index::script_index::serialize_outpoints` | `BTreeSet<OutPoint>` (script_index.rs:139) | iteration in sorted order | **Deterministic** |
| `mempool::cellpool::CellPool::txs` | `IndexMap<[u8;32], PoolEntry>` (cellpool.rs:76) | `iter()` is insertion-ordered; `get_sorted` sorts by score | **Deterministic** (sort is stable; declaration order preserved on tie) |
| `mempool::cellpool::CellPool::spent_outputs` | `BTreeMap<OutPoint, [u8;32]>` (cellpool.rs:79) | direct lookup only | **Deterministic** |
| `state::store::segment::SegmentWriter` | `Arc<Mutex<…>>` for `current_*` fields | sequential; no iteration | **Deterministic** |

No `HashMap` iteration in any verification path. Confirmed.

## Panic surface — production-code categorization

The audit ran a strict `awk` filter on `unwrap() / .expect( / panic! / todo! /
unimplemented!` in production code (excluding the `#[cfg(test)]` module). The
table below lists every production-code panic, the call-site, and whether it
is **reachable from a fixture** vs **unreachable-from-fixture** vs
**internal-only**.

| File:Line | Call | Reachable from fixture? | Note |
|---|---|---|---|
| `consensus/src/lib.rs:219` | `assert!(u32::try_from(total_size).is_ok(), "Molecule table is too large")` | **No** (2^32-byte blocks) | latent |
| `consensus/src/lib.rs:227` | `offset.checked_add(field.len() as u32).expect("Molecule table offset overflow")` | **No** (2^32 commitments) | latent; F-KERN-08 |
| `exec/src/vm/verifier.rs:157` | `serialize_resolved_header_molecule(self).expect("Molecule serialization should not fail for ResolvedHeader")` | **No** (encoder is infallible today) | latent; F-PRIM-12 |
| `exec/src/vm/verifier.rs:174` | `serialize_resolved_cell_molecule(self).expect("… ResolvedCell")` | **No** (same as above) | latent; F-PRIM-12 |
| `exec/src/serialization/molecule_compat.rs:614` | `tx.outputs.get_mut(output_index).expect("output index checked above")` | **No** (bounds-checked at line 611) | latent; F-KERN-09 |
| `exec/src/serialization/molecule_compat.rs:641` | `tx.outputs.get_mut(output_index).expect("output index checked above")` | **No** (bounds-checked at line 638) | latent; F-KERN-09 |
| `exec/src/serialization/cache.rs:140` | `bytes.try_into().unwrap()` (32-byte hash → `[u8; 32]`) | **No** (blake3 always returns 32 bytes) | latent |
| `exec/src/serialization/macros.rs:264` | `try_into().expect("slice length checked")` | **No** (slice length checked) | latent |
| `exec/src/serialization/macros.rs:268` | `try_into().expect("slice length checked")` | **No** (slice length checked) | latent |
| `exec/src/serialization/macros.rs:281` | `try_into().expect("slice length checked")` | **No** (slice length checked) | latent |
| `exec/src/serialization/macros.rs:300` | `try_into().expect("slice length checked")` | **No** (slice length checked) | latent |
| `state/src/store/segment.rs:340` | `SystemTime::now().duration_since(UNIX_EPOCH).unwrap()` | **Yes** (any fixture that calls `seal()`) | F-KERN-04 |
| `state/src/store/segment.rs:357` | `NonZeroUsize::new(MAX_OPEN_SEGMENTS).unwrap()` | **No** (`MAX_OPEN_SEGMENTS = 8` ≠ 0) | latent |
| `state/src/store/segment.rs:372` | `files.get(&segment_id).unwrap()` after `files.put(...)` | **No** (just inserted) | latent |
| `mempool/src/cellpool.rs:235` | `partial_cmp(...).unwrap()` | **No** (via normal scorer), **Yes** (via `TransactionScorer::new(NaN, …)`) | F-KERN-03 |
| `mempool/src/cellpool.rs:331` | `SystemTime::now().duration_since(UNIX_EPOCH).unwrap()` | **Yes** (any `CellPool::add`) | F-KERN-04 |
| `core-utils/src/mem_size.rs:34` | `unimplemented!()` (estimate_size Undefined) | **No** (no caller uses MemMode::Undefined) | latent; F-KERN-07 |
| `core-utils/src/mem_size.rs:40` | `unimplemented!()` (default estimate_mem_bytes) | **No** (all callers override) | latent; F-KERN-07 |
| `core-utils/src/mem_size.rs:46` | `unimplemented!()` (default estimate_mem_units) | **No** (all callers override) | latent; F-KERN-07 |

**Reachable from fixture**: F-KERN-04 (×2 sites). The unwrap on `SystemTime`
is reachable from any fixture that exercises `SegmentWriter::seal` or
`CellPool::add`.

**Latent / unreachable today**: all `expect("…checked above")` paths,
asserts, and `unimplemented!()` blocks. These are documented as latent
footguns.

**No production-code `panic!()`, `todo!()`, or `unimplemented!()` exists in
any kernel path the CLI exercises** — the audit could not find any of those
three tokens in production code in the in-scope crates.

## Secret handling — zeroize / SecretKey reachability

The `secp256k1::SecretKey` type does not implement `Zeroize` by default. The
audit searched for `SecretKey` ownership across the kernel:

- `exec/src/serialization/molecule_compat.rs:317, 355, 388` — three functions
  take `secret_key: &secp256k1::SecretKey` (borrowed, not owned). The
  kernel **never owns a SecretKey**; it receives one as a parameter, signs,
  and returns the recoverable signature. The borrowed SecretKey is never
  dropped inside the kernel; the caller is responsible for zeroization.
- `exec/src/serialization/molecule_compat.rs:326` — constructs
  `secp256k1::Message::from_digest_slice(&message_hash)` (a borrowed
  `Message`, no key material).
- `exec/src/serialization/molecule_compat.rs:328` — constructs
  `secp256k1::Secp256k1::new()` (a context, no key material).

**The kernel does not own SecretKey, does not store key material, and does
not need to zeroize.** The CLI at `cli/src/main.rs:3356-3372` decodes a
hex-string into 32-byte `secret_key_bytes`, constructs a `SecretKey`,
signs, and **does not zeroize `secret_key_bytes` or `secret_key`** before
return. The CLI is out of scope for this lane, but the audit flags the
**non-zeroized stack-resident `secret_key_bytes` array** as a secret-handling
gap that the kernel correctly avoids but the CLI inherits.

The `zeroize` workspace dependency (`Cargo.toml:174`) is declared but the
audit found **zero `use zeroize` or `zeroize::` calls in the in-scope kernel
crates**. The dep is only consumed by code outside this lane's scope.

## Type stability — version-discrimination check

| Type | Serialized form | Multiple versions? | Discriminator |
|---|---|---|---|
| `CellTx` | `serialize_transaction_molecule` (Molecule) | No — single version | `CellTx::version: u32 = 0xC001` (celltx/types.rs:48) |
| `MyelinBlock` | `to_molecule_bytes` (consensus/lib.rs:183-196) | No — single version | embedded in the encoded table |
| `CellScriptSchedulerWitness` | `encode_cellscript_scheduler_witness_molecule` (celltx/types.rs:919) | Yes — has a `version: u8` field (line 804) | `TYPED_CELL_SCHEDULER_WITNESS_VERSION = 1`; decoder rejects `version != 1` at line 967-968 with `CellScriptSchedulerWitnessError::UnsupportedVersion(witness.version)` |
| `Script` | `hash_v1()` length-prefixed (celltx/types.rs:1458-1467) | Yes — has `hash_with_version(ScriptHashVersion)` (line 1470-1474); only `V1` is currently defined | `ScriptHashVersion::V1` enum (line 1378-1381) |
| `TypedCellDecl` | `encode_typed_cell_decl_molecule` (celltx/types.rs:357) | Single version today | implicit |
| `SegmentMeta` | `encode_segment_meta` (segment.rs:69-79) | Single version today | implicit |
| `SegmentProof` | `to_molecule_bytes` (proof.rs:55-66) | Single version today | implicit |
| `ResolvedHeader` / `ResolvedCell` | `VmSerializable` (`verifier.rs:157, 174`) | Single version today | implicit |
| `VersionedEnvelope` | `serialization::validation.rs:147` | Yes — explicit format-version check (0x80-0x8F allowed) and schema-version check | explicit format_version + schema_version |
| `SecureEnvelope` | `serialization::security.rs:118-143` | Single version today | length prefix + blake3 hash |
| `Hash` (crypto/hashes) | Display via hex (lib.rs:115-117) | Single format | 32-byte output, hex-encoded |

**No multi-version serialized types without an explicit discriminator.** The
two versioned types (`CellScriptSchedulerWitness` and `Script`) have an
explicit `version` field; `VersionedEnvelope` has a format-version byte.

The latent issue is `ResolvedHeader` / `ResolvedCell`: their `VmSerializable`
trait is implemented without a version byte (verifier.rs:150-180). If the
ABI is ever extended, the lack of a discriminator means the change is
breaking for any consumer that stores these bytes. Cross-ref: F-PRIM-16
(F-PRIM-19).

## Missing bounds checks

The audit searched for unchecked indices:

| Site | Input | Bounds check? | Failure mode |
|---|---|---|---|
| `exec/src/celltx/sighash.rs:437` | `input_index` (caller-supplied) | **No** | `&tx.inputs[input_index]` panics (F-KERN-16) |
| `exec/src/serialization/molecule_compat.rs:614, 641` | `output_index` | **Yes** (line 611, 638) but expect-arm as fallback | F-KERN-09 |
| `exec/src/celltx/types.rs` Vec/array accesses | various | checked at construction (`CellTx::new` validates inputs/deps ordering and count) | OK |
| `state/src/cell_tree.rs` BTreeMap accesses | `outpoint_hash`, `outpoint` | direct `Option::None` returns | OK |
| `state/src/index/cell_db.rs` OutPoint → key | `out_point` | `to_key()` returns 36-byte fixed array | OK |
| `consensus/src/lib.rs:445` | `signature.validator_id` | `HashMap::get` returns `Option`; `?` propagates | OK |
| `state/src/store/segment.rs:415-417` | `leaf_index` | bounds-check via `chunk_index.get(leaf_index)` returning `None` → `InvalidProof` | OK |
| `state/src/store/segment.rs:446-450` | `segment_info.offset/length` | `position()` returns `Option<usize>` → `InvalidProof` if not found | OK |
| `mempool/src/cellpool.rs:284-286` | `conflict_id` | `txs.get(conflict_id)` returns `Option`; `?` propagates `TxNotFound` | OK |
| `mempool/src/cellpool.rs:316-323` | `tx.inputs` and `parent_txid` | loops; no out-of-bounds access | OK |

The single unchecked input is `exec/src/celltx/sighash.rs:437` (F-KERN-16).
All other bounds paths are either `Option`-returning lookups or
defensive-expect arms.

## Concurrency safety review

| Crate | Lock primitive | Acquisition order | Risk |
|---|---|---|---|
| `consensus` | `HashMap<String, CommitteeValidator>` (line 372, 487) — **no lock** | n/a (immutable per-engine) | OK; engine is `Clone` and meant to be immutable |
| `mempool::cellpool::CellPool` | `parking_lot::RwLock` on `txs`, `spent_outputs`, `stats` | line 125 (read), 130 (read), 160-162 (write × 3) | **Lock ordering is consistent** — all three writes held together in `add`; all three removed together in `remove`. But the size check at line 130 is a **read lock** followed by a **write lock** in the same function, allowing a race where another thread fills the pool between the read and the write. Cross-ref: prior F-05 in MEMPOOL_CONSENSUS. |
| `state::store::segment::SegmentWriter` | `parking_lot::Mutex` on `current_segment_id`, `current_file`, `current_offset`, `current_chunks`, `_segments` | `append`: `current_file` → `current_offset` → `current_segment_id` → `current_chunks`. `seal`: `current_file` → `current_offset` → `current_chunks` → `current_segment_id`. | **Lock ordering is consistent** but **undocumented**. The `rotate_segment` path calls `self.seal()` while holding `current_file` and `current_offset` (line 228-230), but the code releases them before the call (line 151-152 of `append`). No deadlock today. F-KERN-10. |
| `state::store::segment::SegmentReader` | `parking_lot::Mutex<lru::LruCache<u32, File>>` (line 349) | held during `try_clone`+`seek`+`read_exact` (line 363-381) | The mutex is held across the disk read — a slow disk on one segment blocks reads of all other segments. Cross-ref: prior F-16 in STATE_DA. |
| `state::index::cell_db::CellDB` | `parking_lot::RwLock<()>` (line 76) | single-lock per write | OK |
| `state::index::script_index::ScriptIndex` | `parking_lot::RwLock<()>` (line 27) | single-lock per write | OK |
| `exec::scheduler::ParallelExecutor` | `rayon::par_iter` (line 47) | thread pool parallelism | Results are written to `results[node_id]` (line 56), so deterministic. Cross-ref: F-PRIM-13. |

**No deadlock found.** All lock acquire orders are consistent within their
respective subsystems. The crossbeam-channel and async-channel paths live
in the unused `utils/` crate (F-KERN-01) and are therefore not in the kernel
runtime.

## utils/ crate hygiene

The `utils/` crate declares **20+ modules** (33 files / 3,089 lines):

| Module | External references | Verdict |
|---|---|---|
| `utils::any` | 0 | **DEAD** |
| `utils::arc` | 0 | **DEAD** |
| `utils::as_slice` | 0 | **DEAD** |
| `utils::binary_heap` | 0 | **DEAD** |
| `utils::channel` | 0 | **DEAD** |
| `utils::expiring_cache` | 0 | **DEAD** |
| `utils::fd_budget` (cfg(not wasm32)) | 0 | **DEAD** |
| `utils::git` | 0 (used internally by `sysinfo`) | **DEAD externally** |
| `utils::hashmap` | 0 | **DEAD** |
| `utils::hex` | 0 (only doctests in `lib.rs`) | **DEAD** |
| `utils::iter` | 0 | **DEAD** |
| `utils::mem_size` | 0 | **DEAD** |
| `utils::networking` (494 lines, `IpAddress`, `NetAddress`, `PrefixBucket`, `wasm_bindgen`) | 0 | **DEAD** |
| `utils::refs` | 0 | **DEAD** |
| `utils::serde_bytes` | 0 (only doctests in `lib.rs`) | **DEAD** |
| `utils::serde_bytes_fixed` | 0 (only doctests in `lib.rs`) | **DEAD** |
| `utils::serde_bytes_fixed_ref` | 0 (only doctests in `lib.rs`) | **DEAD** |
| `utils::serde_bytes_optional` | 0 | **DEAD** |
| `utils::sim` (161 lines, discrete-event simulation in virtual time) | 0 | **DEAD** |
| `utils::sync` | 1 (`utils/benches/bench.rs:3` — `RfRwLock`) | **DEAD** (only its own benchmark) |
| `utils::sysinfo` (126 lines, `SystemInfo`, `OnceLock`) | 0 | **DEAD** |
| `utils::triggers` | 0 | **DEAD** |
| `utils::vec` | 0 | **DEAD** |

**Total: 0 of 23 modules are used by any kernel crate.** F-KERN-01.

## core-utils/ crate hygiene

| Module | External references | Verdict |
|---|---|---|
| `core_utils::hex::ToHex`, `FromHex` | 2 (`crypto/hashes`, `math`) | **LIVE** |
| `core_utils::mem_size::MemSizeEstimator` | 2 (`crypto/hashes`, `math`) | **LIVE** but F-KERN-07 (latent `unimplemented!()`) |
| `core_utils::serde_bytes` | 0 | **DEAD** (F-KERN-02) |
| `core_utils::serde_bytes_fixed` | 0 | **DEAD** (F-KERN-02) |
| `core_utils::serde_bytes_fixed_ref` | 0 | **DEAD** (F-KERN-02) |

`core-utils` carries **5 `unsafe { str::from_utf8_unchecked }` blocks** in
the dead serde modules (F-KERN-02), none with a `SAFETY:` comment. The live
`hex::ToHex` impl at `core-utils/src/hex.rs:45` also has an unsafe block
without a `SAFETY:` comment, mirroring the `crypto/hashes/src/lib.rs:115-117`
finding from prior F-PRIM-34.

## Top risks callout

1. **F-KERN-01 / F-KERN-02 (dead crates and dead sub-crates)**: the most
   material hygiene finding. The `myelin-utils` crate compiles, its tests
   pass, its docs are correct, but **nothing in the runtime uses it**. The
   `core-utils::serde_bytes*` modules are similarly dead. Both blocks
   inflate `Cargo.lock` and slow compile times without contributing to the
   production evidence path.

2. **F-KERN-04 (SystemTime unwrap × 2)**: a `SystemTime::now()` call with a
   pre-1970 wall clock panics. Both sites are reachable from the fixture
   flows the production-evidence layer exercises (`SegmentWriter::seal` and
   `CellPool::add`).

3. **F-KERN-06 (non-deterministic `.meta` bytes)**: `SegmentMeta.created_at`
   and `sealed_at` are wall-clock-derived. The merkle root is deterministic,
   but the `.meta` byte format is not. Any cross-machine replay will produce
   byte-different metadata. The seal-time reproducibility claim is **only**
   true for the merkle root, not for the full `.meta` payload.

4. **F-KERN-03 (NaN-panic in `get_sorted`)**: unreachable through the
   default scorer, but the `TransactionScorer::new` constructor is `pub`
   and accepts arbitrary `f64`. A future caller that injects NaN will
   crash the process.

5. **F-KERN-08 (`Molecule table offset overflow` panic in `consensus`)**:
   reachable with 2^32 cell-tx commitments. The `MyelinBlock` type has no
   `BTreeSet` upper bound on `ordered_cell_tx_commitments` /
   `data_commitments`.

6. **F-KERN-05 (dead Cargo.toml deps)**: `indexmap`, `anyhow`, `memmap2`
   in `exec`/`state` are declared but never imported. The `memmap2` and
   `indexmap` in `state` are already flagged in prior STATE_DA F-06/F-07.

## Cross-references to prior audits

| This lane | Prior finding |
|---|---|
| F-KERN-03 (NaN panic in `get_sorted`) | `MYELIN_SWARM_AUDIT_MEMPOOL_CONSENSUS.md:45` (F-04) |
| F-KERN-05 (`memmap2`, `indexmap` dead in state) | `MYELIN_SWARM_AUDIT_STATE_DA.md:67, 68` (F-06, F-07) |
| F-KERN-11 (`insert_with_outpoint` silent eviction) | `MYELIN_SWARM_AUDIT_STATE_DA.md:71` (F-05) |
| F-KERN-12 (`timestamp_ms` participates in block hash) | `MYELIN_SWARM_AUDIT_MEMPOOL_CONSENSUS.md:52` (F-11) |
| F-KERN-13 (`execute_sequential` Result type misleading) | `MYELIN_SWARM_AUDIT_WHOLEREPO.md` F-PRIM-25 |
| F-KERN-14 (`serialize_resolved_*_molecule` expect) | `MYELIN_SWARM_AUDIT_WHOLEREPO.md` F-PRIM-12 |
| F-KERN-15 (`deterministic_signature` 64-byte layout) | New (no prior coverage) |
| F-KERN-16 (`calc_standard_signature_hash` no input_index bound check) | `MYELIN_SWARM_AUDIT_WHOLEREPO.md` F-PRIM-23 |
| F-KERN-17 (`CellStateTree::cells` is `pub`) | `MYELIN_SWARM_AUDIT_STATE_DA.md:90` (F-24) |
| F-KERN-18 (`CellTx::payload` first output only) | `MYELIN_SWARM_AUDIT_WHOLEREPO.md` F-PRIM-24 |
| F-KERN-19 (`split_vm_abi_trailer` heuristic) | `MYELIN_SWARM_AUDIT_WHOLEREPO.md` F-PRIM-16 |
| F-KERN-20 (no kernel crate declares `[lints]`) | `MYELIN_SWARM_AUDIT_MEMPOOL_CONSENSUS.md:57` (F-16) |
| F-KERN-22 (RISC-V fixture unsafety count) | `MYELIN_SWARM_AUDIT_WHOLEREPO.md` F-PRIM-38 |
| F-KERN-04 / F-KERN-06 (SystemTime in segment.rs / cellpool.rs) | `MYELIN_SWARM_AUDIT_STATE_DA.md:67` (F-04 noted in the state audit but at lower severity) |
| F-KERN-07 (`MemSizeEstimator::unimplemented!()`) | New (no prior coverage) |
| F-KERN-08 (consensus `encode_table` panic) | New (no prior coverage) |
| F-KERN-09 (`molecule_compat` `expect` fallback) | New (no prior coverage) |
| F-KERN-10 (`SegmentWriter` lock-order hygiene) | New (no prior coverage) |
| F-KERN-21 (`core-utils/src/{hex,mem_size}/` empty subdirs) | New (no prior coverage) |

## Per-crate hygiene summary

| Crate | `unsafe` | `#[allow(...)]` | `random`/`thread_rng` | Random source | Notes |
|---|---|---|---|---|---|
| `exec` | 0 in scope Rust files; 33+ in `scripts/fixtures/*.rs` (out of scope) | `#[allow(ambiguous_glob_reexports, missing_docs)]` in `vm/mod.rs:6` | None | None | Celltx/sighash deterministic. VM wraps CKB-VM. No `[lints]` inheritance. F-KERN-09 (latent `expect`). |
| `state` | 0 in source | None | None | `SystemTime::now` at segment.rs:340 (F-KERN-04, F-KERN-06) | `memmap2`, `indexmap`, `anyhow` are dead deps (F-KERN-05). |
| `consensus` | 0 | None | None | None | `encode_table` `assert`/`expect` is latent (F-KERN-08). `MyelinBlock.timestamp_ms` participates in hash (F-KERN-12). |
| `mempool` | 0 | None | None | `SystemTime::now` at cellpool.rs:331 (F-KERN-04) | `partial_cmp(...).unwrap()` panics on NaN (F-KERN-03). `MempoolError::InvalidTx` and `::DependencyNotFound` are unreachable variants. |
| `core-utils` | 5 `unsafe { str::from_utf8_unchecked }` in dead serde modules + 1 in live `hex.rs` | None | None | None | `serde_bytes*` are dead (F-KERN-02). `MemSizeEstimator::unimplemented!()` is latent (F-KERN-07). Empty subdirs `core-utils/src/{hex,mem_size}/` (F-KERN-21). |
| `utils` | (only in unused modules) | None | None | None | **Entire crate is dead from kernel perspective** (F-KERN-01). |
| `crypto/hashes` | 1 `unsafe { str::from_utf8_unchecked }` at `lib.rs:115-117` (no SAFETY comment) — F-PRIM-34 | None | None | None | Out of scope for new findings; re-references prior F-PRIM-17, F-PRIM-18. |
| `crypto/muhash` | 0 | None | `rand_chacha` seeded from blake3 hash (deterministic) | None | Out of scope; re-references prior F-PRIM-19. |
| `math` | 4 `unsafe { str::from_utf8_unchecked }` in `uint.rs` | None | None | None | Out of scope; re-references prior F-PRIM-08, F-PRIM-09. |

## Open questions

1. **F-KERN-01**: Is the `utils/` crate deliberately kept as a "future
   networking plane" that has not been wired into the kernel yet? Or is it
   the residue of an earlier refactor that split `core-utils` out and
   forgot to delete `utils/`? The audit cannot answer; this is a project
   scope question. The 3,089 lines and ~25 transitive deps (`arc-swap`,
   `async-channel`, `event-listener`, `ipnet`, `sysinfo`, `uuid`,
   `wasm-bindgen`, `mac_address`, `rlimit`, `tokio`, `duct`) inflate every
   `cargo check --workspace` and `cargo build --release` cycle.

2. **F-KERN-02**: Are the `core-utils::serde_bytes*` modules forward-looking
   for a future serde migration, or are they residues of an earlier
   `core-utils` carve-out? They compile and their unit tests pass, but no
   `.rs` file outside `core-utils` itself imports them.

3. **F-KERN-06**: Should `SegmentMeta.created_at` / `sealed_at` be removed
   from the persisted form (they are not part of any cryptographic binding)
   or sourced deterministically (e.g. the chunk count × committed-to epoch
   number)?

4. **F-KERN-08**: Should `MyelinBlock` cap `ordered_cell_tx_commitments.len()`
   and `data_commitments.len()` at `u32::MAX / 32` to make the
   `encode_table` overflow check a defensive belt-and-braces rather than a
   process-level panic?

5. **F-KERN-15**: Should `deterministic_signature` be renamed to
   `deterministic_fixture_signature` to make the "not a real cryptographic
   signature" disclaimer visible at every call site? Today the disclaimer
   is only in the doc comment.

6. **F-KERN-21**: Should the empty `core-utils/src/hex/` and
   `core-utils/src/mem_size/` subdirectories be deleted, or should the flat
   `hex.rs` / `mem_size.rs` files be moved into the subdirectories? The
   current state is **legal Rust** but ambiguous.

---

## Stop-condition checklist

- [x] Lane file exists with verdict, top risks, findings table, evidence trail.
- [x] **22 findings** (more than the requested 10 minimum), of which 6
      CRITICAL/HIGH, 7 MEDIUM, 8 LOW/INFO.
- [x] Panic-surface sweep is complete with categorization
      (reachable-from-fixture vs unreachable-from-fixture vs internal-only).