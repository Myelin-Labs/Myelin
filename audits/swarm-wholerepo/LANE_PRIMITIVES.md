# Myelin Swarm Audit — Exec + Crypto + Math Primitives

> Verifier-only review. No fixes proposed. Scope: `exec/src/{lib.rs, projection.rs,
> execution_report.rs, celltx/*, scheduler/*, serialization/*, vm/*}`,
> `crypto/hashes/src/*`, `crypto/muhash/src/*`, `math/src/*`.
>
> The branch diff is concentrated in `cli/src/main.rs` and `scripts/`; this lane
> audits the underlying primitives that the production-evidence layer depends on,
> so the production-gate consumer can see which foundation risks are inherited.

## Verdict

**Conditional PASS for the celltx/sighash + projection path that the
production-evidence CLI exercises in closed-validator mode, with one
collision-class defect and one ALWAYS-true warning logic that demand a
verifier-only eye before any witness type is widened.** The branch does not
touch any of the in-scope source files, but the CLI's
`recompute-from-molecule` evidence path
(`cli/src/main.rs:2150-2210`) is anchored to `deserialize_transaction_molecule`
+ `project_cell_tx_to_ckb` + `compute_txid` + `ckb_raw_transaction_hash_molecule`,
all of which live in this lane and are stable on this branch.

The two highest-impact findings are:

1. **`compute_conflict_hash` and `compute_typed_data_hash` lack length-prefixing
   on the script `args` field**, which collides `blake3(domain || code_hash ||
   hash_type || args || data)` for `(args="X", data="")` vs `(args="", data="X")`.
   The collision is on the type-cell identity path the CLI signed evidence
   bundles traverse (`execution_report.rs:99` and the typed-DAG `build_from_typed`
   in `dag.rs:150`). The contract `Script::hash_v1` (which DOES length-prefix
   `args`) is inconsistent with these two helpers, and the audit cannot see
   whether the production gate catches the collision.

2. **`project_cell_tx_to_ckb` has a dead-branch / ALWAYS-true warning** at
   `projection.rs:116-120`: every `CellTx` produced by `CellTx::new*`
   (which sets `version = CELL_TX_VERSION = 0xC001`) trips the
   `NonCkbTransactionVersion` warning regardless of the input. The
   second branch hard-codes `actual: CELL_TX_VERSION` (the constant) instead
   of `actual: tx.version`, so the warning field is a constant, not a
   real diagnostic.

The remaining findings are correctness/determinism gaps in the CKB-VM
integration boundary, two `expect(...)` paths in production
(`verifier.rs:157, 174`; `u3072.rs:166`), a missing keccak/`.s`
linkage, a `max_size=0` cache that still holds an entry, a streaming
deserializer that trusts the on-wire `total_len` for a 4 GB allocation,
a divide-by-zero `assert!` in `uint.rs::div_rem`, a NaN-collapse
`OrderedFloat::cmp` that mirrors the mempool audit's F-04, and the
deletion of `exec/src/vm/README_VM_STATUS.md` (the project's own
admission of VM incompleteness) without an obvious replacement.

## Findings

| # | Severity | Component | Finding | File:Line | Doc claim | Code reality |
|---|----------|-----------|---------|-----------|-----------|--------------|
| F-PRIM-01 | **CRITICAL** | celltx/types | `compute_conflict_hash` and `compute_typed_data_hash` hash `args` and `data` concatenated without length-prefixing. `(code_hash=H, hash_type=0, args="X", data="")` collides with `(code_hash=H, hash_type=0, args="", data="X")`. | `exec/src/celltx/types.rs:299-307, 316-324` | "Compute typed data hash. … blake3(domain ‖ code_hash ‖ hash_type ‖ args ‖ data)" (line 309-313) | Both fields are written via `hasher.update(args); hasher.update(data);` with no length prefix, so a boundary shift between args and data is not detected by blake3. |
| F-PRIM-02 | **HIGH** | celltx/types | `Script::hash_v1` length-prefixes `args` (`&(self.args.len() as u32).to_le_bytes()` at line 1464), but the two typed-cell helpers in the same file do not. The script-hash function and the typed-data/conflict-hash functions are structurally inconsistent. | `exec/src/celltx/types.rs:1458-1467` vs `299-307, 316-324` | "Calculate the V1 script hash with explicit domain separation and versioning" (line 1457) | The V1 hash avoids the collision. The typed-cell helpers do not. A consumer that hashes a `Script` via `hash_v1` and then uses `compute_typed_data_hash(script, data)` will see a different prefix boundary in the two hashes. |
| F-PRIM-03 | **HIGH** | projection | `project_cell_tx_to_ckb` always emits `NonCkbTransactionVersion` for txs created via `CellTx::new*` (version = `0xC001`). The second branch's `actual` field is the **constant** `CELL_TX_VERSION`, not `tx.version`. | `exec/src/projection.rs:116-120` | "Build a CKB-style projection report for a CellTx" (line 107) | The first branch fires only when `version ∉ {0, 0xC001}`. The second branch (`else if version == CELL_TX_VERSION`) fires for every normal tx and hard-codes the constant in the `actual` field. The branch ordering also has a logic gap: when `version == 0` the function emits **no** version warning at all. |
| F-PRIM-04 | **HIGH** | celltx/sighash | `CellTx::push_cellscript_scheduler_witness` admits only one scheduler witness. The Myelin extensions (coinbase payload in `calc_standard_signature_hash` line 351-362) are not in the CKB sighash but DO affect the standard Myelin sighash for coinbase txs. | `exec/src/celltx/sighash.rs:351-362, 266-277` | "Coinbase-style transactions … reserved for explicit session genesis or issuance contexts" (`types.rs:1796-1798`) | `compute_rw_bound_sighash` is used by the CLI's evidence path. The Myelin standard-lock sighash diverges from CKB sighash_all for coinbase txs. |
| F-PRIM-05 | **HIGH** | serialization/molecule_compat | `pack_number` silently truncates `usize` to `u32` (line 1063-1065). The check in `decode_table` (line 1119) only catches the case where the resulting u32 is out of `[first_offset, total_size]`. | `exec/src/serialization/molecule_compat.rs:1063-1065, 1110-1123` | "Molecule-compatible bytes are the public/default protocol boundary" (`lib.rs:41-43`) | On 64-bit, an encoder that produces a 4 GB+ table silently truncates the offset header. The decoder's `total_size != bytes.len()` check (line 1096) catches truncation on decode, but a round-trip `encode → decode` of a >4 GB table fails. The `overflow-checks = true` in release does not apply to `as u32` casts. |
| F-PRIM-06 | **HIGH** | serialization/streaming | `StreamingDeserializer::deserialize` (line 99-116) trusts the on-wire `total_len` for `Vec::with_capacity(total_len)` with no bound check beyond `total_len < 4`. A 4-byte input claiming `total_len = u32::MAX` triggers a 4 GB `Vec` allocation. | `exec/src/serialization/streaming.rs:99-116` | None explicit; `SecurityGuard` in `security.rs:161-166` has a `check_size` helper that is **not** invoked here. | The streaming deserializer is in the public API (`myelin_exec::serialization::streaming::StreamingDeserializer`) and the bound check is the only barrier. A malicious producer of a stream can OOM the consumer. |
| F-PRIM-07 | **HIGH** | scheduler/conflict | `OrderedFloat::cmp` collapses NaN to `Equal` (line 60: `f64::partial_cmp(...).unwrap_or(Ordering::Equal)`). This is the same anti-pattern flagged in the mempool audit (F-04) and is now reproducible from a different code path. | `exec/src/scheduler/conflict.rs:58-62` | "Deterministic conflict resolution" (line 3) | NaN is treated as equal to every other value, so a NaN-bearing `fee_density` is not panicking but is silently indistinguisable from another NaN. `select_winners` with multiple NaN inputs returns them in **declaration order** (Rust's stable sort) — non-canonical but deterministic. The "fee_density strictly dominates" claim is still violated. |
| F-PRIM-08 | **HIGH** | math/uint | `UintN::div_rem` (line 319) panics on division by zero via `assert_ne!(your_bits, 0, …)`. The `overflow-checks = true` profile in the workspace does **not** suppress this assert. | `math/src/uint.rs:319` | None | `assert_ne!` is unconditional — debug **and** release panic. The bench profile (`overflow-checks = false`) does not change `assert!` behaviour. The `mod_inverse` path (`uint.rs:347-361`) does not panic; it returns `None` for unreducible inputs. |
| F-PRIM-09 | **MEDIUM** | math/uint | `UintN::as_f64` (line 272-306) overflows the f64 exponent field for `Uint3072` and `Uint320` in the common case. The exponent calculation `Self::BITS + 1021 - leading_zeroes` yields values up to 4093 (for `Uint3072`), exceeding the 11-bit exponent field of f64 (max 2046). | `math/src/uint.rs:272-306` | None explicit. The test at line 1121-1122 only checks `Uint128` (BITS=128, max exponent 1149) which fits. | For `Uint3072` and any `UintN` with BITS > 1023, `f64::from_bits((exponent << 52) + mantissa)` produces a bit pattern that decodes as `+inf` or NaN. The function does not return a `Result`; callers that consume the f64 get NaN/inf without warning. |
| F-PRIM-10 | **MEDIUM** | serialization/cache | `SerializationCache::insert` accepts a key when `max_size = 0` and the cache is empty. The check `cache.len() >= max_size && !contains_key(&key)` is `0 >= 0 && !false` = `true && true` = `true`, so it tries to evict, finds nothing, and **still inserts** the key — leaving the cache at size 1 when `max_size = 0`. | `exec/src/serialization/cache.rs:150-161` | "Evict if necessary" (line 151) | The size invariant is silently violated. Also the eviction at line 155 is O(n) (`self.access_order.remove(0)`), so the "LRU" is FIFO + bump-to-back-on-hit with O(n) update. |
| F-PRIM-11 | **MEDIUM** | serialization/security | `SecureEnvelope::length` is `u32` (line 90) but `data.len() as u32` (line 97) silently truncates. The 4 GB+ envelope then fails the length check on `verify` (line 110), but a 4 GB allocation is still triggered by the constructor. | `exec/src/serialization/security.rs:83-115` | "Verifies data length before integrity check" (line 109) | `verify` rejects (good), but the cost of the 4 GB Vec + blake3 hash is paid on the encode side. The default `max_size = 100 MB` (line 40) saves most callers. |
| F-PRIM-12 | **MEDIUM** | vm/verifier | `VmSerializable for ResolvedHeader` and `ResolvedCell` call `.expect("Molecule serialization should not fail for ResolvedHeader")` (line 157, 174) in production code. The functions they call (`serialize_resolved_header_molecule`, `serialize_resolved_cell_molecule`) return `Result<Vec<u8>, MoleculeError>`; the `Ok(...)` is always taken today because the inner `encode_table` is infallible, but the `expect` is a panic in production if a future refactor makes the function fallible. | `exec/src/vm/verifier.rs:157, 174` | None | `.expect(...)` in production code is a latent panic. The `encode_table` body in `molecule_compat.rs:1074-1089` is currently infallible, so today the panic is unreachable — but the docstring does not promise this. |
| F-PRIM-13 | **MEDIUM** | vm/verifier | `TransactionScriptVerifier::verify_with_cycles` (line 420-430) runs script groups in parallel via Rayon (`par_iter`) but only the **cycle totals** are accumulated. The order of cycle contributions is the `BTreeMap` iteration order (lock-groups by hash, then type-groups by hash), so the final sum is deterministic. However, the *content* of each group's verification is determined by `verify_script_group` which in turn calls `vm.run()` — that is a real CKB-VM execution path, and any non-determinism in the CKB-VM cycle counter propagates. | `exec/src/vm/verifier.rs:420-430, 553-563` | "Keep error selection deterministic by folding results in the stable script-group order produced by extract_script_groups" (line 425-426) | The CKB-VM cycle counter is documented to be deterministic, but the comment doesn't reference the CKB-VM's `estimate_cycles` cost model. If CKB-VM changes the cost model between versions, the same script reports different cycles — this is a version-pinned reproducibility risk, not a code defect. |
| F-PRIM-14 | **MEDIUM** | vm/verifier | `prepare_group_runtime` (line 620-630) rejects every `hash_type != 0` (line 621-623). The `Script::hash_type` enum comment in `types.rs:1434-1441` documents `Type=1, Data1=2, Data2=4` as also valid CKB values. Only `Data=0` is accepted for script loading. | `exec/src/vm/verifier.rs:620-630` | "Reference: CKB ScriptHashType encoding" (`types.rs:1434-1441`) | The verifier is **functionally narrower** than the type definition. A CKB-projected transaction with a `Type=1` script will fail verifier-side even though it serialises correctly. The `MYELIN_CKB_PROJECTION_AUDIT.md` would need to call this out, but the audit couldn't find a doc that does. |
| F-PRIM-15 | **MEDIUM** | vm/verifier | `extract_script_groups` orders groups as **all lock-groups first, then all type-groups** (line 411: `lock_groups.into_values().chain(type_groups.into_values())`). CKB runs type-scripts before lock-scripts so the type-script can mutate state visible to the lock-script. The Myelin verifier runs **lock-then-type**, the opposite. | `exec/src/vm/verifier.rs:342-412` | "Lock scripts execute against resolved input cells" (line 348) | `verify_with_cycles` runs groups in `BTreeMap.into_values()` order, which is by group kind (lock, then type) and then by hash. If a type script's verification outcome must precede a lock script's check (CKB convention), the ordering is backwards. The lock-script's `LoadCell` syscall sees the **un-type-script-verified** cell. |
| F-PRIM-16 | **MEDIUM** | vm/scheduler | The `split_vm_abi_trailer` heuristic at `serialization/mod.rs:387-406` strips a 16-byte trailer if the last 16 bytes match `b"MYLNABI\0` + 4 zero bytes + 4 zero bytes (flags=0, reserved=0). The function returns the inner `program` slice, but `run_script` (`machine.rs:139-141`) discards the `VmAbiFormat` return. A random 16-byte tail that happens to match the magic and zero pattern (1/2^64 chance per buffer, with 12 of 16 bytes being the magic+pattern) silently strips 16 bytes from the ELF before loading. | `exec/src/serialization/mod.rs:370-406`, `exec/src/vm/machine.rs:139-141` | "Split an optional fixed VM ABI trailer from executable artifact bytes" (line 386) | The 16-byte trailer is **heuristic**, not signed. An attacker that can append 16 bytes to a script can cause the wrong code to be loaded. The discarded `VmAbiFormat` argument means the caller cannot detect the strip. |
| F-PRIM-17 | **MEDIUM** | crypto/hashes | `SchnorrSigningHash` (line 130) uses `sha256` while all other `CellTx*Hash` hashers use `blake3` (line 88-98). The file-level docstring at line 3 says "radical transition from blake2b & sha256 to full-scale BLAKE3 adoption" but the Schnorr hasher is still sha256. `SchnorrSigningHash` is used by `calc_standard_signature_hash` (`sighash.rs:438`). | `crypto/hashes/src/hashers.rs:88-130` | "Modified for the radical transition from blake2b & sha256 to full-scale BLAKE3 adoption" (line 3) | Two different hash families in the same standard-lock sighash path. Domain separation prevents cross-protocol forgery but increases verifier surface area. The "SchnorrSigningHash" name suggests it is the canonical Schnorr challenge hash, which is conventionally `blake2b` per BIP-340 — using sha256 is non-conventional. |
| F-PRIM-18 | **MEDIUM** | crypto/hashes | The vendored keccak `.s` files (`keccakf1600_x86-64.s`, `keccakf1600_x86-64-osx.s`) are compiled by `build.rs` only on x86_64 (non-Windows for the linux variant, macos for the osx variant). On aarch64 the build script is a no-op. The `keccak` Rust crate is declared as a dependency (`crypto/hashes/Cargo.toml:18, 27`) but **never used** in any `.rs` file (`grep -rn "use keccak\|keccak::" crypto/` returns 0 hits). The `.s` files are compiled into `libkeccak.a` but no `extern "C"` declaration in this crate links them. | `crypto/hashes/build.rs:1-16`, `crypto/hashes/Cargo.toml:15-27`, vendored at `crypto/hashes/src/keccakf1600_x86-64.s:7, 269` and `keccakf1600_x86-64-osx.s:7, 269` | "Implementations domain-separated hashing for Myelin blockchain" (`hashers.rs:1-3`) | Both the Rust `keccak` crate and the vendored `.s` are dead. The Cargo dep inflates compile time and lockfile; the `.s` files are a static-lib island with no consumer. The upstream provenance is the `dot-asm/cryptogams` repo (line 1 of each `.s` file). |
| F-PRIM-19 | **MEDIUM** | crypto/muhash | `U3072::inverse` (line 166) panics on inputs that are 0, equal to the modulus, or unreduced via `Uint3072(a.limbs).mod_inverse(Self::UINT_PRIME).expect("Cannot fail, 0 < a < prime").0`. The doc comment claims `0 < a < prime` is invariant at that point, but the function is public-in-crate and reachable from `div`. The contract is enforced by the caller's `is_overflow + full_reduce` and `a == zero` checks at lines 158-165, but a future caller that bypasses these will panic. | `crypto/muhash/src/u3072.rs:156-173` | "Cannot fail, 0 < a < prime" (line 166) | The assertion is a comment, not a debug_assert!; the panic is unconditional. The 2^-256 probability of producing a 0 element from `data_to_element` is negligible, but the public-ish `div_assign` (line 225-230) does not guarantee a reduced `*self` either, so adversarial inputs could in principle reach the panic. |
| F-PRIM-20 | **MEDIUM** | celltx/sighash | `compute_txid` and `compute_wtxid` (lines 103-169, 174-246) are **near-byte-identical** bodies. The two functions differ only in: (a) the domain constant, (b) the trailing witnesses block (wtxid only). Any change to the field encoding in one function must be mirrored in the other, but the diff is not enforced by a test. | `exec/src/celltx/sighash.rs:100-246` | "Compute txid (without witnesses) … Formula: blake3(CELL_TXID_DOMAIN ‖ …)" (line 102); "Compute wtxid (with witnesses)" (line 171) | 60+ duplicated lines. The standard celltx/sighash test (`test_sighash_computation` at line 562) exercises only `compute_rw_bound_sighash`; the cross-check between `compute_txid` and `compute_wtxid` for the same tx is asserted in `test_wtxid_computation` (line 548) by `assert_ne!`, but only that the hashes differ — not that they differ for the right reason. |
| F-PRIM-21 | **MEDIUM** | celltx/sighash | `compute_txid` writes `dep.dep_type.clone() as u8` (line 124, 194). `DepType` is a fieldless `#[repr(u8)]` enum (Code/DepGroup), so `clone()` is a memcpy, but the expression is misleading — it implies the enum has fields. The pattern repeats. | `exec/src/celltx/sighash.rs:124, 194` | "Cell transaction types (CKB-inspired)" (`mod.rs:3`) | Readability smell, not a defect. Performance is identical to `dep.dep_type as u8`. |
| F-PRIM-22 | **MEDIUM** | celltx/sighash | `calc_standard_signature_hash` (line 437-455) does **not** include `cell_deps` or `header_deps` in the canonical signed payload. A malicious actor can mutate `tx.cell_deps` (swap type script references) or `tx.header_deps` without invalidating a derived signature. CKB does the same, but Myelin's `compute_rw_bound_sighash` (line 266-277) does cover them **transitively** via `wtxid`. | `exec/src/celltx/sighash.rs:430-455` | "Canonical CellTx standard-lock sighash shared by signer tooling, consensus, and native lock verification" (line 428-429) | Two sighash entry points, with different coverage: `calc_standard_signature_hash` is the CKB-compatible sighash that omits `cell_deps`/`header_deps`; `compute_rw_bound_sighash` includes them via wtxid. The CLI evidence path uses the wtxid-binding path. |
| F-PRIM-23 | **MEDIUM** | celltx/sighash | `calc_standard_signature_hash` (line 437) panics via `&tx.inputs[input_index]` if `input_index >= tx.inputs.len()`. No bounds check. | `exec/src/celltx/sighash.rs:437` | "Canonical CellTx standard-lock sighash" (line 428) | Caller-supplied `input_index` is not validated. A malformed `input_index` (e.g. from a custom lock script) panics the process. |
| F-PRIM-24 | **MEDIUM** | celltx/types | `CellTx::payload` (line 1858-1868) returns `outputs_data.first()` for a coinbase tx with **any** number of outputs, not just the no-outputs case. The doc says "coinbase-style transactions have no inputs and are reserved for explicit session genesis or issuance contexts" (line 1796-1798) but a coinbase tx with multiple outputs still picks the first output's data as the "payload". `standard_payload_hash` then hashes this. | `exec/src/celltx/types.rs:1858-1868`, `exec/src/celltx/sighash.rs:351-362` | "Cellbase-style transactions have no inputs" (line 1796) | A coinbase tx with `[output_A, output_B]` and `[data_A, data_B]` has its payload bound to `data_A` only, while `compute_txid` and `compute_wtxid` hash both. This produces a cellbase tx whose sig covers one output's data but whose txid covers both — a structural asymmetry. |
| F-PRIM-25 | **MEDIUM** | scheduler/executor | `ParallelExecutor::execute_sequential` (line 68-79) has return type `Result<Vec<ExecutionResult>, ExecutionError>` but the `Err(_)` arm at line 76-77 is unreachable — closure errors are wrapped in `Ok(Failed { … })`. The function never returns `Err`. Also, the function does not check `txs.len()` against an expected count, so `ExecutionError::TxCountMismatch` is also unreachable. | `exec/src/scheduler/executor.rs:68-79` | "Execute transactions sequentially (for testing)" (line 67) | The `Result` return is misleading. Both `ExecutionError` variants are unreachable from this function. |
| F-PRIM-26 | **MEDIUM** | scheduler/dag | `CellDAG::build` (line 88-139) initialises `conflict_hash_conflicts: BTreeMap::new()` and never populates it. The typed-cell conflict rules are only in `build_from_typed` (line 150-203). A caller that intends typed-cell semantics but calls `build` (the simpler API) silently gets a DAG with no `conflict_hash_conflicts`. | `exec/src/scheduler/dag.rs:88-139, 150-203` | "Build DAG from a set of Cell transactions" (line 80) | Two builders with different field semantics. `build` looks like the obvious entry point; `build_from_typed` requires the caller to know to ask for it. The function names don't signal the difference. |
| F-PRIM-27 | **LOW** | vm/mod | `MYELIN_VM_VERSION` (line 38) and `MYELIN_VM_ISA` (line 41) are public constants but the `VmLimits::default()` is wired to `CKB_VM_MEMORY = 4 MB` (line 64) and the `VmContext::with_default_cycles` to `10_000_000` (line 105). The combination "10M cycles + 4 MB memory" is CKB's testnet default, not mainnet. The CLI does not surface which default it uses. | `exec/src/vm/mod.rs:38-94` | None | Defaults are hard-coded as constants; runtime configuration via `VmLimits::new` is the only override path. |
| F-PRIM-28 | **LOW** | scheduler/dag | `CellDAG::compute_layers` (line 282-291) detects an invariant violation (`successor out of bounds`, `reached zero in-degree too early`) and returns `Err(DagError::InvalidRWSet)`. The error messages are informative, but the function only validates during layer construction, not when edges are added. A bug in an edge-adder that produces a cycle is caught here; a bug that produces a phantom edge (successor with no entry in the in-degree array) is caught. | `exec/src/scheduler/dag.rs:282-308` | "Kahn's algorithm with layer tracking" (line 250) | The validation is correct but defensive; the precondition is the responsibility of the edge-adder. |
| F-PRIM-29 | **LOW** | celltx/types | `encode_cellscript_scheduler_witness_molecule` (line 919) and `decode_cellscript_scheduler_witness` (line 914) are separate encode/decode paths. The encode is infallible (returns `Vec<u8>`); the decode is fallible. Asymmetric API. | `exec/src/celltx/types.rs:914-919` | None | Documented in the celltx module. API smell. |
| F-PRIM-30 | **LOW** | exec (root) | The `exec/src/vm/README_VM_STATUS.md` file (190 lines) was deleted in commit `c8008e3` ("Clean stale Myelin artefacts") on this branch. The README was a 2025-10-22 status report that documented: "current verifier path reaches the real CKB-VM run loop", "most scripts beyond the always-success fixture still lack end-to-end execution coverage", and a per-area gap list. The deletion is bundled with several other doc cleanups. | `exec/src/vm/README_VM_STATUS.md` (deleted in `c8008e3`) | None — README is gone | The README's content is **still accurate** — the VM does call real CKB-VM, but full syscall/runtime completeness is not achieved. Deleting it removes the project's own admission of incompleteness. No replacement status doc was found in `exec/src/vm/` or `docs/`. |
| F-PRIM-31 | **LOW** | crypto/muhash | `U3072::mul` (line 97-100) short-circuits when `*self == Self::one()` to `*self = *other`. The check is `==`, not `!=`, so the optimisation is taken only for the multiplicative identity. The doc comment at line 92-96 says "If self ≠ one, the comparison should exit early, otherwise if they are equal -- we gain much more than we lose". The implementation is consistent with the comment but the wording is ambiguous. | `crypto/muhash/src/u3072.rs:90-100` | "Optimization: short-circuit when LHS is one" (line 92) | Behaviour is correct: `1 * x = x` short-circuits, `0 * x = 0` does not. |
| F-PRIM-32 | **LOW** | exec (root) | The workspace `[workspace.lints.clippy]` at `Cargo.toml:202-203` sets only `empty_docs = "allow"`. None of the crates in scope (`exec`, `crypto/hashes`, `crypto/muhash`, `math`) declare a `[lints]` table to inherit. | `Cargo.toml:202-203`, all in-scope `Cargo.toml` files | None | Lint configuration is workspace-wide but not enforced per-crate. The Myelin style is `missing_docs` warns (`#![warn(missing_docs)]` in `exec/src/vm/mod.rs:6`) but is private to that module. |
| F-PRIM-33 | **LOW** | math | `UintN::compact_target_bits` (line 89-103) uses `u64::from(Self::BITS) + 1021 - u64::from(leading_zeroes)`. The multiplication `(self.0[Self::LIMBS - 1] << (8 * (size - 3))) as u32` (line 95) silently truncates for large `UintN`. The `as u32` at line 95 truncates. | `math/src/lib.rs:64-103` | None | For `UintN` with BITS > 256, the `as_u64() as u32` truncation loses precision in the compact-target encoding. Practical impact: consensus layer (which uses Uint256) is fine; future users of larger `UintN` are not. |
| F-PRIM-34 | **INFO** | crypto/hashes | `Hash::Display::fmt` (line 115-117) uses `unsafe { str::from_utf8_unchecked(&hex) }` with **no SAFETY comment**. The hex output is by construction 64 ASCII chars (`0-9a-f`), so the unsafe is safe-by-construction. | `crypto/hashes/src/lib.rs:115-118` | None | Latent readability defect: the safety argument is implicit. A reviewer must verify that `faster_hex::hex_encode` produces ASCII-only output. |
| F-PRIM-35 | **INFO** | math | `UintN` macros emit 4 `unsafe { str::from_utf8_unchecked }` blocks (lines 759, 818, 836, 850). Three have explicit `// SAFETY: …` comments; the one at line 850 does not. | `math/src/uint.rs:759, 818, 836, 850` | None | Same as F-PRIM-34. All 4 are in `LowerHex` / `Display` / `Binary` / `Serialize` for the `UintN` type. |
| F-PRIM-36 | **INFO** | math (wasm) | `math/src/wasm.rs` provides `js_value_to_vec_u8` (line 4) for wasm32 targets. The function uses `Vec::try_from(js_value: JsValue)` and depends on `js_sys`/`wasm_bindgen`. Not in the CLI's hot path. | `math/src/wasm.rs:1+` | None | Wasn't reviewed in depth. |
| F-PRIM-37 | **INFO** | workspace | `[profile.bench]` sets `overflow-checks = false` (`Cargo.toml:195`). The benchmark profile therefore produces different behaviour for `debug_assert!` overflow checks in `math/src/uint.rs:536, 548, 560, 572, 584, 671, 683`. The `release` profile correctly has `overflow-checks = true` (`Cargo.toml:189`). | `Cargo.toml:186-201` | None | Benchmarks are not the production path, so the relaxed profile is acceptable. |
| F-PRIM-38 | **INFO** | exec/scripts (out of scope) | `exec/src/scripts/fixtures/*.rs` are RISC-V lock-script sources compiled to ELF for the CKB-VM. They use `unsafe { … }` blocks (mostly via `extern "C"` + `#[unsafe(no_mangle)]`) and inline assembly helpers. Not in scope of this audit (these are not Rust libraries, they are C/Rust compiled to RISC-V), but the unsafe usage is significant. | `exec/src/scripts/fixtures/*.rs` (33+ unsafe blocks across 8 files) | None | Listed for completeness. The RISC-V lock scripts are C/Rust translated to RISC-V via `rustc` and linked by the CKB-VM ELF loader. |

## CellTx + sighash — field-coverage analysis

For every CellTx field, the table below records whether it is covered by
each canonical hash function. **Bold = covered. Plain = not covered.**

| Field | `compute_txid` (Myelin) | `compute_wtxid` (Myelin) | `calc_standard_signature_hash` (CKB-style) | `compute_rw_bound_sighash` (CLI evidence) |
|---|---|---|---|---|
| `version` | via domain-anchored blake3 input | via domain-anchored blake3 input | `write_u32(tx.version)` | transitively via wtxid |
| `inputs` (full, including since) | all input fields via txid blake3 | all input fields via wtxid blake3 | partial (gated by `is_sighash_*`) | transitively via wtxid |
| `inputs[input_index].previous_output` | via txid | via wtxid | via `hash_outpoint` | transitively via wtxid |
| `cell_deps` | via txid | via wtxid | **NOT covered** | transitively via wtxid |
| `header_deps` | via txid | via wtxid | **NOT covered** | transitively via wtxid |
| `outputs` (with type, capacity, lock) | via txid | via wtxid | partial (gated by `is_sighash_*`) | transitively via wtxid |
| `outputs_data` | via txid | via wtxid | via `hash_cell_output` | transitively via wtxid |
| `witnesses` (full) | **NOT** (txid omits witnesses by design) | via wtxid | n/a (sighash is the input's hash, not the tx hash) | transitively via wtxid |
| `inputs[input_index].since` | via txid | via wtxid | partial (`is_sighash_single/none/anyone_can_pay` returns ZERO) | transitively via wtxid |
| `payload` (coinbase) | via txid `coinbase-payload-fallback` | via wtxid | via `standard_payload_hash` | transitively via wtxid |
| `hash_type` (sighash flag) | n/a | n/a | last byte of canonical hash | n/a |

**Key takeaway**: `cell_deps` and `header_deps` are **only** covered transitively
via `wtxid` in `compute_rw_bound_sighash`. A signer who signs at the standard
lock (`calc_standard_signature_hash`) and then publishes a tx with mutated
`cell_deps` will pass signature verification but bind different on-chain state.
This is **CKB-compatible** behaviour (F-PRIM-22) but is a real risk if the
production-evidence layer ever adopts the standard lock instead of the
wtxid-bound lock.

## Scheduler — determinism check

| Path | Source of nondeterminism | Verdict |
|---|---|---|
| `CellDAG::build` (line 88-139) | None. `BTreeMap` is sorted. Iterates `txs` in input order. | **Deterministic** |
| `CellDAG::build_from_typed` (line 150-203) | None. Nested `for i in 0..entries.len()` is sequential; dedup check is deterministic. | **Deterministic** |
| `CellDAG::compute_layers` (line 253-308) | `current_layer.sort_unstable()` (line 276) is stable on the input set. | **Deterministic** |
| `ConflictResolver::compute_key` (line 90-98) | `f64` arithmetic, possible NaN. `OrderedFloat::cmp` collapses NaN. | **Deterministic but non-canonical under NaN** (F-PRIM-07) |
| `ConflictResolver::select_winners` (line 117-129) | `Vec::sort_by` is stable, so declaration order is preserved within NaN. | **Deterministic** |
| `ParallelExecutor::execute` (line 33-65) | `par_iter().collect()` is non-deterministic in collect order, but results are written to `results[node_id]` (line 56). | **Deterministic** (the order in the output Vec is by `node_id`) |
| `ParallelExecutor::execute_sequential` (line 68-79) | Closure-driven, sequential. | **Deterministic** but the `Result` return is misleading (F-PRIM-25) |

**No `RwLock` is held in the scheduler layer.** The brief asked specifically
about `RwLock contention, lock ordering, re-entrancy`. The scheduler is
stateless across calls (it builds a fresh `BTreeMap` and `Vec` per
invocation). Concurrency safety depends on the *caller* not sharing
`CellDAG` instances across threads, which is not enforced.

## VM — cost-model determinism

`vm/cost_model.rs` is pure integer arithmetic (`transferred_byte_cycles`,
`memory_cycles`, `syscall_cycles`). `INSTRUCTION_CYCLES = 1`,
`MEMORY_PAGE_CYCLES = 1024`, `SYSCALL_BASE_CYCLES = 500` are constants.
The cost model is wired into CKB-VM via `ckb_vm::cost_model::estimate_cycles`
in `machine.rs:10`. **The CKB-VM cost model is documented to be deterministic
across runs, but is not version-pinned by the Myelin wrapper.** F-PRIM-13
flags this as a version-pinned reproducibility risk.

**README_VM_STATUS.md was deleted in commit `c8008e3`** (F-PRIM-30). The
deleted file documented the same cost-model concern ("current verifier
path reaches the real CKB-VM run loop", "most scripts beyond the
always-success fixture still lack end-to-end execution coverage"). The
deletion is intentional (bundled with several other doc cleanups in a
"Clean stale Myelin artefacts" commit) but the content is still accurate.

## Serialization — encode/decode roundtrip checks

| Function | Encode side | Decode side | Roundtrip strict? |
|---|---|---|---|
| `encode_table` / `decode_table` (`molecule_compat.rs:1074, 1091`) | Infallible `Vec<u8>` | Strict `field_count` check, offset monotonicity check, total-size check. | **Yes** for valid inputs. `as u32` truncation on 64-bit (F-PRIM-05). |
| `encode_bytes` / `decode_bytes` (`molecule_compat.rs:1126, 1133`) | Infallible | Length-prefixed, strict. | **Yes** |
| `encode_fixvec_cell_deps` / decode (line ~1500-1600) | Encode is infallible, uses `pack_number` (truncates to u32) | Decode validates count, then each entry. | **Yes** for `<2^32` entries |
| `ckb_sighash_all_message_molecule` (line 516-536) | Hashes raw tx, then signing_witness, then extras | n/a (one-way) | n/a |
| `SecureEnvelope::to_bytes` / `from_bytes` (`security.rs:118-143`) | Encodes length+hash+data | Decodes and verifies hash | **Yes** for `<4 GB` data; truncation on 4 GB+ (F-PRIM-11) |
| `StreamingDeserializer::deserialize` (line 99-116) | n/a (deserialize only) | **Trusts on-wire `total_len` for `Vec::with_capacity` (F-PRIM-06)** | **No** — no bound check |
| `VersionedEnvelope::from_bytes` (validation.rs:147) | n/a | Format-version check (0x80-0x8F allowed), schema-version check, payload size check | **Yes**, but `StreamingDeserializer` bypasses it |
| `serialize_transaction_molecule` ↔ `deserialize_transaction_molecule` (`molecule_compat.rs:867, 872`) | `Transaction = RawTransaction + Witnesses` | Decodes `RawTransaction` then `witnesses` | **Yes** with `outputs.len() == outputs_data.len()` check |

**Cache vs decode consistency**: `SerializationCache::get_or_serialize` caches
by `TypeId + blake3(serialized_bytes)`. Two serialisations of the same Rust
value that produce different bytes (e.g. due to a different `CellTx`
versioning) would miss the cache. The cache is per-instance (`HashMap`), not
shared, so cross-process consistency is not a concern. The `max_size = 0`
edge case (F-PRIM-10) is the only correctness defect.

## Projection — lossy transformations

`project_cell_tx_to_ckb` (`exec/src/projection.rs:108-172`) is a *report* —
it does not produce a transformed artefact. It computes:
- `source_txid` via `compute_txid` (Myelin native, blake3)
- `ckb_raw_tx_hash` via `ckb_raw_transaction_hash_molecule` (CKB blake2b)
- `ckb_wtx_hash` via `ckb_transaction_witness_hash_molecule` (CKB blake2b)
- `molecule_transaction_bytes` via `serialize_transaction_molecule` (the
  byte size of the CKB-shaped encoding)

**Fields that survive**: All `CellTx` fields are reflected in the
`molecule_transaction_bytes` (CKB Molecule) and the two CKB hashes.

**Fields that are dropped or transformed**:
- The Myelin-native `txid` (`source_txid`) and the CKB `ckb_raw_tx_hash`
  are **different** for the same `CellTx` because they use different hash
  families and different field orderings (Myelin: inputs first, then deps;
  CKB: deps first, then inputs).
- The `version` field is included in both, but the CKB version is `0` (the
  CKB convention), while `CellTx::new*` sets `version = 0xC001`. The
  projection emits a `NonCkbTransactionVersion` warning (F-PRIM-03) but
  the CKB hash is computed with the Myelin version, not a substituted
  CKB version. This is **intentional** (the report says what the
  Myelin-native tx hashes to under the CKB algorithm) but the warning
  is structurally broken.

**Divergence from a freshly-encoded Myelin-native artefact**: A `CellTx`
encoded via `serialize_transaction_molecule` and then re-decoded via
`deserialize_transaction_molecule` produces the same `CellTx` (assuming
`outputs.len() == outputs_data.len()`). The `compute_txid` and
`ckb_raw_transaction_hash_molecule` of the original and the round-tripped
`CellTx` agree. The projection report is therefore **consistent** with
re-encoding, except for the dead-branch logic defect (F-PRIM-03).

## Hashes — domain string coverage and unsafe review

| Hasher | Domain string | Hash function | Public? |
|---|---|---|---|
| `CellTxHash` | `b"TransactionHash"` | blake3 (via `blake3_256`) | `pub` |
| `CellTxId` | `b"TransactionID"` | blake3 | `pub` |
| `CellTxSigningHash` | `b"TransactionSigningHash"` | blake3 | `pub` |
| `BlockHash` | `b"BlockHash"` | blake3 | `pub` |
| `MerkleBranchHash` | `b"MerkleBranchHash"` | blake3 | `pub` |
| `MuHashElementHash` | `b"MuHashElement"` | blake3 | `pub` |
| `MuHashFinalizeHash` | `b"MuHashFinalize"` | blake3 | `pub` |
| `CellMessageSigningHash` | `b"PersonalMessageSigningHash"` | blake3 | `pub` |
| `CellTxSigningHashEcdsa` | `b"TransactionSigningHashECDSA"` | blake3 | `pub` |
| `SchnorrSigningHash` | `b"SchnorrSigningHash"` | **sha256** | `pub` |

**F-PRIM-17**: `SchnorrSigningHash` is the only sha256 hasher in the
registry, all others are blake3. The file's docstring claims "radical
transition from blake2b & sha256 to full-scale BLAKE3 adoption" but this
single hasher did not migrate. It is wired into `calc_standard_signature_hash`
(`sighash.rs:438`).

**Keccak ASM**: F-PRIM-18. The `.s` files are vendored from
`https://github.com/dot-asm/cryptogams/blob/master/x86_64/keccak1600-x86_64.pl`
(provenance: line 1 of each file). The build script only compiles them
on x86_64 (linux non-windows or macos). On aarch64 the build script
is a no-op. The `keccak` Rust crate is declared as a dependency but
**never used** (`grep -rn "use keccak\|keccak::" crypto/` returns 0 hits).
The compiled `libkeccak.a` is never linked (no `extern "C"` declaration).

**Vendored .s inspection** (`keccakf1600_x86-64-osx.s`):
- The `__KeccakF1600` loop body uses `jmp L$oop` to enter and `testq $255, %r15` / `jnz L$oop` to continue for 24 rounds (line 16, 258-259). `r15` is the round-constant pointer.
- The OSX wrapper (`_KeccakF1600`) saves callee-saved registers (rbx, rbp, r12-r15) per SysV ABI, allocates 200 bytes of stack via `subq $200, %rsp` (line 294), and restores via `addq $200, %rsp` and `popq` of the saved registers (line 317, 320-336). The asymmetry: `subq $200, %rsp` and `addq $200, %rsp` balance; `pushq` and `popq` are 5 pairs (rbx, rbp, r12-r15), balancing 5 push + 5 pop.
- The round constants (line 344-368) are 24 iotas matching the SHA-3 standard.
- The trailing `.byte 75, 101, …` (line 370) is the "Keccak-1600 absorb and squeeze for x86_64, CRYPTOGAMS by <appro@openssl.org>" provenance marker.
- The `leaq 100(%rdi), %rdi` and `leaq -100(%rdi), %rdi` (line 293, 315) shift the pointer by 100 bytes (the state is treated as 25 u64s, accessed via negative offsets).

The ASM is a faithful port of the upstream cryptogams implementation.
No obvious bugs (off-by-one in stack frames, wrong register clobbers,
missing ret) were spotted on visual review, **but the audit did not run
the ASM under a disassembler to verify register clobbering against the
SysV ABI**. The Linux variant is structurally identical (different
symbol names only).

**Output length handling**: All `blake3_hasher!` and `sha256_hasher!`
output 32 bytes. No length-truncation is performed. `finalize` consumes
`self` (not `&self`), so the hasher is single-use; a mis-caller can
`reset()` via the `Hasher` trait method (line 73-76) to re-use with the
same domain prefix.

**Unsafe in `crypto/hashes`**: One `unsafe { str::from_utf8_unchecked }`
at `lib.rs:115-118` in `Hash::Display::fmt`. The `hex` is 64 ASCII chars
produced by `faster_hex::hex_encode` (an explicit hex encoder), so the
unsafe is safe-by-construction. **No SAFETY comment is present**
(F-PRIM-34). The block is not reachable unless `Display::fmt` is called
on a `Hash`, which is reachable from `celltx` debug output and
`state::cell_tree::CellEntry` debug.

## MuHash — U3072 additive property check

`MuHash` is a multiplicative multiset hash. The "additive" property
the audit brief asked about is actually the *commutativity of
add_element / remove_element*:

- `add_element(a); add_element(b); finalize() == add_element(b); add_element(a); finalize()` ✓ — both paths produce the same numerator (`1 * H(a) * H(b)`), so `finalize` is the same.
- `add_element(a); remove_element(a); finalize() == new().finalize() == EMPTY_MUHASH` ✓ — numerator stays 1, denominator becomes H(a), `normalize` divides to 1/H(a) * H(a) = 1.
- `combine` is symmetric: `self.numerator *= other.numerator; self.denominator *= other.denominator` (line 88-90). ✓

**The additive (commutative) property is preserved.**

**U3072 arithmetic correctness**:
- `add` is not a function in the public API; only `MulAssign` and `DivAssign` are. The underlying `mul` is a textbook schoolbook multiplication with Montgomery-style reduction via the prime difference `PRIME_DIFF = 1103717`. The reduction is performed via `full_reduce` when `is_overflow` is true.
- `inverse` is delegated to `malachite_nz::natural::Natural::mod_inverse`, which is a battle-tested library. The `expect` at line 166 is a panic if the input is not in `[1, prime)` (F-PRIM-19).
- `div` is `self *= other.inverse()` after reduction. ✓

**Panic paths**:
- `inverse` panics on `self == 0` (if `a == 0` after the early-return at line 163, but that early-return is before `mod_inverse`, so `a == 0` returns 0 without panicking). The doc comment is correct.
- `inverse` panics on `self == prime` (if `full_reduce` is buggy and leaves `self == prime`). The `is_overflow` check (line 51) returns `true` for `prime` (limbs[0] > MAX - PRIME_DIFF, limbs[1..] all MAX), so the `full_reduce` is called. **If `full_reduce` has a bug that fails to reduce `prime` to `0`, the next `inverse` panics.**
- `mul` asserts `carry_highest == 0` at line 123 and `carry_high == 0` at line 143, and `carry_low in {0, 1}` at line 144. These are `assert_eq!` / `assert!` (not `debug_assert!`), so they **panic in release as well as debug**. They are post-condition checks after a complex carry chain. A bug that triggers one of these would crash the production process.

**`is_overflow` (line 49-56)**: checks `self.limbs[0] <= MAX - PRIME_DIFF` (not overflown) or `self.limbs[1..]` all MAX (overflown). This is **the only** overflow detection; intermediate operations that produce a value just below `prime` but with `limbs[1..]` not all MAX would be **not** flagged as overflown, even though the value is fine. So the check is conservative for the "overflowed" case (it only flags values that match the exact MAX-PRIME_DIFF+1 pattern) and lenient for the "not overflowed" case (anything else, including the prime itself if `limbs[1..]` are not all MAX, would be classified as not overflown). This is **subtly wrong** if the U3072 value ever holds a pattern where `limbs[0] > MAX - PRIME_DIFF` but not all other limbs are MAX — but such a value is mathematically `>= prime` and should be reduced. The current `is_overflow` would miss it.

## Math — public API surface

| Function | Behaviour | Panics? |
|---|---|---|
| `UintN::from_u64`, `from_u128` (line 22, 28) | Infallible | No |
| `UintN::leading_zeros` (line 62) | Pure bit op | No |
| `UintN::overflowing_shl` (line 67) | Returns `(value, overflowed)` | No |
| `UintN::wrapping_shl` (line 87) | Wraps bits | No |
| `UintN::overflowing_shr` (line 92) | Returns `(value, overflowed)` | No |
| `UintN::overflowing_add` (line 112) | Returns `(value, overflowed)` | No |
| `UintN::overflowing_add_u64` (line 130) | Returns `(value, overflowed)` | No |
| `UintN::overflowing_sub` (line 143) | Returns `(value, overflowed)` | No |
| `UintN::saturating_sub` (line 162) | Saturates | No |
| `UintN::saturating_add` (line 168) | Saturates | No |
| `UintN::overflowing_mul_u64` (line 175) | Returns `(value, overflowed)` | No |
| `UintN::overflowing_mul` (line 193) | Returns `(value, overflowed)` | No |
| `UintN::from_le_bytes` (line 213) | Infallible | No |
| `UintN::from_be_bytes` (line 225) | Infallible | No |
| `UintN::to_le_bytes` (line 236) | Infallible | No |
| `UintN::to_be_bytes` (line 245) | Infallible | No |
| `UintN::to_be_bytes_var` (line 255) | Infallible | No |
| `UintN::div_rem_u64` (line 262) | Panics on `other == 0`? Let me check. | **Need to verify** |
| `UintN::as_f64` (line 272) | Infallible but overflows for BITS > 1023 | No panic, NaN/inf result (F-PRIM-09) |
| `UintN::div_rem` (line 310) | **`assert_ne!(your_bits, 0, …)` panics on div by zero** (F-PRIM-08) | **YES** |
| `UintN::mod_inverse` (line 347) | Returns `Option` | No (returns `None` for `self == 0`) |
| `UintN::from_hex` (line 411) | Returns `Result` | No |
| `UintN::from_be_bytes_var` (line 424) | Returns `Result` | No |
| `UintN::to_bigint` (line 442) | wasm-only | No |

`UintN::div_rem_u64` (line 262-269) does **not** check `other == 0` explicitly, but Rust's `/` and `%` operators panic on division by zero. So `div_rem_u64` panics on `other == 0` via the inner `/` and `%` calls (line 266-267). The brief asked: "panicking paths in const-fn contexts". `div_rem` is **not** a const fn (the macro doesn't define it as const). `from_le_bytes` and `from_be_bytes` are **const fn** (line 213, 225) — they cannot panic.

**NaN/inf paths**: `as_f64` (line 272-306) is the only path that produces NaN or inf. For `UintN` with BITS > 1023, the exponent field overflows (F-PRIM-09). The function does not return a `Result`; the caller gets the f64. No `NaN` is produced via `from_*` paths.

**Workspace `overflow-checks = true` in release profile** (`Cargo.toml:189`):
applies to the `+`/`-`/`*` operators on `u64` inside the macro. The `as u32` and `as usize` casts are **not** arithmetic ops and are not affected by `overflow-checks`. So `pack_number` truncating `usize` to `u32` is silent in release.

## Determinism / hash purity from CLI evidence paths

Walking from `cli/src/main.rs:2150-2210` (the recompute-from-molecule evidence
path) back to the primitives:

```
cli/src/main.rs:2159  deserialize_transaction_molecule(&molecule_transaction)
  → exec/src/serialization/molecule_compat.rs:872  decode_table + decode_*
    → blake3 (in hashers.rs) — deterministic
    → String allocations — deterministic
cli/src/main.rs:2160  project_cell_tx_to_ckb(&tx)
  → exec/src/projection.rs:108  project_cell_tx_to_ckb
    → exec/src/celltx::compute_txid — deterministic
    → exec/src/serialization::ckb_raw_transaction_hash_molecule — CKB blake2b — deterministic
    → exec/src/serialization::ckb_transaction_witness_hash_molecule — CKB blake2b — deterministic
    → exec/src/serialization::serialize_transaction_molecule — deterministic
cli/src/main.rs:2161  TeeworldsChunkProjectionReport::from(projection)
  → deterministic struct construction
cli/src/main.rs:2162-2210  push_check × 7 — comparison only
```

**Every primitive on the evidence-recompute path is deterministic.** No
`SystemTime::now`, `Instant::now`, `thread_rng`, `rand::*`, `getrandom`,
or atomic counter is called. The only randomness on the production
primitive path is `MuHash::data_to_element` (line 171-177 of muhash/lib.rs),
which is **deterministic** because it seeds ChaCha20Rng from a blake3
hash of the input. (Confirmed: `rand_chacha` is only imported in
`muhash/lib.rs:11` and used in `data_to_element` and `MuHashElementBuilder::finalize`,
both of which seed from a hash, not from the OS RNG.)

**However**, F-PRIM-13 (CKB-VM cost-model version) is a future risk: if
CKB-VM ever ships a non-deterministic cost model (e.g. per-CPU
calibration), the cycle totals in `verify_with_cycles` would diverge
across runs.

## Unsafe usage

`grep -rn "unsafe" exec/src crypto/ math/src`:

### exec/src

| File:Line | Block | Safety argument | Verdict |
|---|---|---|---|
| `vm/verifier.rs` | none | n/a | n/a |
| `vm/machine.rs` | none | n/a | n/a |
| `vm/scheduler.rs` | none | n/a | n/a |
| `vm/cost_model.rs` | none | n/a | n/a |
| `vm/error.rs` | none | n/a | n/a |
| `celltx/*.rs` | none | n/a | n/a |
| `scheduler/*.rs` | none | n/a | n/a |
| `serialization/*.rs` | none | n/a | n/a |
| `projection.rs` | none | n/a | n/a |
| `execution_report.rs` | none | n/a | n/a |
| `lib.rs` | none | n/a | n/a |
| `scripts/fixtures/*.rs` | 33+ `unsafe { … }` blocks across 8 files (`load_ecdsa_signature_hash.rs`, `secp256k1_blake3_lock.rs`, `secp256k1_lock_fixture.rs`, `always_success.rs`, `load_header_timestamp.rs`, `htlc.rs`, `timelock_absolute.rs`, `load_dep_cell_data.rs`, `htlc_minimal.rs`, `load_input_since.rs`, `timelock_relative.rs`) | Mostly `extern "C"` + `#[unsafe(no_mangle)]` for RISC-V lock-script entry points. Some inline assembly via `core::arch::asm!`. | Out of scope for the Rust audit (these are compiled to RISC-V ELF, not consumed as Rust libraries). Listed for completeness. |

### crypto/

| File:Line | Block | Safety argument | Verdict |
|---|---|---|---|
| `hashes/src/lib.rs:115-118` | `unsafe { str::from_utf8_unchecked(&hex) }` in `Hash::Display::fmt` | Implicit: `faster_hex::hex_encode` produces 64 ASCII hex chars | **Safe by construction. No SAFETY comment. (F-PRIM-34)** |
| `muhash/src/u3072.rs` | none | n/a | n/a |
| `muhash/src/lib.rs` | none | n/a | n/a |

### math/

| File:Line | Block | Safety argument | Verdict |
|---|---|---|---|
| `src/uint.rs:759` | `unsafe { core::str::from_utf8_unchecked(&hex[first_non_zero..]) }` in `LowerHex::fmt` | Explicit comment: "The string is hex encoded so must be valid UTF8." | Safe. |
| `src/uint.rs:818` | `unsafe { std::str::from_utf8_unchecked(&buf[curr..]) }` in `Display::fmt` | Explicit comment: "everything up to `curr` is valid UTF8 because `DEC_DIGITS_LUT` is." | Safe. |
| `src/uint.rs:836` | `unsafe { std::str::from_utf8_unchecked(&buf[first_one..]) }` in `Binary::fmt` | Explicit comment: "We only wrote '0' and '1' so this is always valid UTF-8" | Safe. |
| `src/uint.rs:850` | `unsafe { std::str::from_utf8_unchecked(&hex) }` in `Serialize::serialize` | **Implicit: same hex-encode case** | **Safe. No SAFETY comment. (F-PRIM-35)** |

No `unsafe` in `math/src/{int,wasm,lib}.rs` other than the macro-expanded
`uint.rs` blocks. The wasm helpers are wasm-bindgen glue and not `unsafe`
themselves.

## Open questions

1. **F-PRIM-01 (CRITICAL)**: Does the production-evidence layer re-derive
   `typed_data_hash` for any CellTx with `type_=Some(...)`? If yes, the
   collision on `(args="X", data="")` vs `(args="", data="X")` would let
   an attacker substitute one cell for another in a witness bundle. The
   audit consumer should ask whether `MYELIN_PRODUCTION_GATE.sh` includes
   a witness cell with a non-empty `type_script.args` and a non-empty
   `outputs_data[i]`; if not, the collision is latent but unreachable.

2. **F-PRIM-03 (HIGH)**: Is `NonCkbTransactionVersion` warning ever consumed
   by a gate? If the gate uses `warning.is_empty()` as a pass condition,
   every valid Myelin tx would fail. If the gate ignores it, the warning
   is documentation noise. The audit couldn't find the gate consumer in
   the in-scope files.

3. **F-PRIM-04 (HIGH)**: Is the Myelin `calc_standard_signature_hash`
   ever used as a fallback when `compute_rw_bound_sighash` is unavailable?
   The two paths cover different fields; mixing them across sessions
   would let a signer bind cell_deps in one session and not in another.

4. **F-PRIM-05 (HIGH)**: Is any Molecule table in production larger than
   2^32 bytes? The brief said the `state/` store handles segments up to
   1 GB (per the `state` audit's memmap2 claim, which was found to be
   unused). For segments < 1 GB, the `as u32` truncation is benign. For
   any future 4 GB+ table, the encoding silently corrupts the offset header.

5. **F-PRIM-06 (HIGH)**: Is `StreamingDeserializer` ever used on
   untrusted input? The 4 GB allocation is reachable from any
   4-byte input. The `SecurityGuard::check_size` helper exists but
   is not wired into the deserializer.

6. **F-PRIM-13 (MEDIUM)**: The CKB-VM cost model is imported via
   `use ckb_vm::cost_model::estimate_cycles;` in `machine.rs:10` but the
   version is not pinned in `Cargo.toml` (only `ckb_vm = …` workspace
   dep). A CKB-VM major version bump could change the cycle totals.

7. **F-PRIM-14 (MEDIUM)**: Is the `hash_type != 0` rejection in
   `prepare_group_runtime` (line 621) a deliberate scope cut, or a
   bug? The `Script::hash_type` doc says `Type=1` is valid CKB. A
   CKB-projected transaction with a `Type` script cannot pass
   verifier-side — this is a functional limitation, not a security
   check.

8. **F-PRIM-15 (MEDIUM)**: Is the lock-before-type ordering
   intentional, or a holdover from the original Myelin run order?
   The CKB convention is type-before-lock. A CKB-projected
   transaction where a type script must be verified before a lock
   script can observe the mutated state would have its lock script
   see stale state under the Myelin verifier.

9. **F-PRIM-16 (MEDIUM)**: Is the `VmAbiFormat` returned by
   `split_vm_abi_trailer` actually used anywhere? The current call in
   `run_script` discards it. If the format is supposed to gate the
   ELF loader (e.g., reject non-Molecule artifacts), the discard is
   a defect.

10. **F-PRIM-18 (MEDIUM)**: Why is the `keccak` crate declared as a
    dependency? It is never used in source. The `.s` files are also
    unused. Was there a previous use that was removed? The `Cargo.toml`
    `[features] no-asm = ["keccak"]` (line 13) suggests the `keccak`
    crate was supposed to be the fallback when ASM is disabled, but
    the source code never reaches for it.

11. **F-PRIM-19 (MEDIUM)**: Is the `expect` at `u3072.rs:166` reachable
    in production? The defensive checks in `inverse` (line 158-165)
    should prevent the panic for `data_to_element`-derived inputs.
    A bug in `full_reduce` that fails to reduce `prime` to `0` would
    surface here.

12. **F-PRIM-22 (MEDIUM)**: Is the CKB-compatible sighash ever used
    in the production-evidence layer? If yes, the `cell_deps` /
    `header_deps` coverage gap is a real risk.

13. **F-PRIM-25 (MEDIUM)**: Is the `Result` return type of
    `execute_sequential` (line 68-79) load-bearing for callers? If
    yes, the unreachable `Err` arm is misleading. If no, the function
    should return `Vec<ExecutionResult>` directly.

14. **F-PRIM-26 (MEDIUM)**: Does any caller accidentally use `build`
    when `build_from_typed` was intended? The DAG semantics differ
    only in the `conflict_hash_conflicts` field.

15. **F-PRIM-30 (LOW)**: Is there a replacement status doc for
    `exec/src/vm/README_VM_STATUS.md`? The audit couldn't find one.
    The README's content is still accurate; deleting it removed a
    self-admission of incompleteness.

## Per-crate hygiene summary

| Crate | `unsafe` | `#[allow(...)]` | `random`/`thread_rng` | Random source | Notes |
|---|---|---|---|---|---|
| `exec` | 0 in scope Rust files; 33+ in `scripts/fixtures/*.rs` (out of scope) | `#[allow(ambiguous_glob_reexports, missing_docs)]` in `vm/mod.rs:6` | None | None | Celltx/sighash deterministic; verification wraps CKB-VM. No `[lints]` inheritance. |
| `crypto/hashes` | 1 `unsafe { str::from_utf8_unchecked }` at `lib.rs:116` (F-PRIM-34) | `#[allow(dead_code)]` on `blake3d` and `blake3_stream` | None | None | `keccak` crate is dead dep (F-PRIM-18). `sha256` only in `SchnorrSigningHash` (F-PRIM-17). |
| `crypto/muhash` | 0 | `#[allow(dead_code)]` not used | None (only in tests) | None | `expect` panic in `inverse` (F-PRIM-19). `mul` carry-chain asserts (release-panic). |
| `math` | 4 `unsafe { str::from_utf8_unchecked }` in `uint.rs` (3 with SAFETY comments, 1 without — F-PRIM-35) | None | None | None | `assert_ne!` panic in `div_rem` (F-PRIM-09). `as_f64` overflows for BITS > 1023 (F-PRIM-09). |

Workspace `[workspace.lints.clippy]` (`Cargo.toml:202-203`) sets only
`empty_docs = "allow"`. None of the in-scope crates declare a `[lints]`
table to inherit. This means the workspace-level lint config has no
effect on `exec`, `crypto/hashes`, `crypto/muhash`, or `math`.

`clippy.toml` (not in this lane's scope, but referenced) sets
`too-many-arguments-threshold = 10`. None of the public functions in
scope exceed this.

Workspace `[profile.release]` (`Cargo.toml:186-189`) sets
`overflow-checks = true`. The `bench` profile (`Cargo.toml:191-195`)
sets `overflow-checks = false`. The release profile's `as u32`
truncations in `molecule_compat::pack_number` and `u3072::is_overflow`
are **not** arithmetic ops and are unaffected by `overflow-checks`
(F-PRIM-05, F-PRIM-19 observation).
