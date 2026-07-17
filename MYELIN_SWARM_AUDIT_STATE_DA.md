# Myelin Swarm Audit — State + DA

**Scope:** `state/src/lib.rs`, `state/src/cell_tree.rs`, `state/src/molecule.rs`,
`state/src/index/{cell_db.rs,script_index.rs,mod.rs}`,
`state/src/store/{mod.rs,segment.rs,proof.rs}`, `state/Cargo.toml`,
`state/README.md`, plus the CLI surface in `cli/src/main.rs` that consumes the
state layer (`session_da_manifest`, `session_da_anchor_package`,
`session_submit_da_anchor_package`, `external_da_receipt_evidence`,
`da_availability_evidence`, `external_da_receipt_production_guarantee_checked`).

Review-only. No fixes proposed.

## Verdict

**Block on merge.** The state/DA crate compiles and its 49 unit tests pass,
but the audit surfaces **three CRITICAL issues** that materially weaken the
soundness story the production-gate relies on, plus several HIGH/MEDIUM items:

1. **Empty-tree Merkle root is `[0u8; 32]`, which collides with the
   "unwritten hash" sentinel** (`proof.rs:201`). A `SegmentProof` over an
   empty segment verifies as `true` against a zero root, and the produced
   proof is structurally indistinguishable from "this segment has no leaves"
   vs. "this segment has one leaf that happened to hash to zero" (not
   possible for blake3, but the design uses `[0u8; 32]` for an empty
   tree and `[0u8; 32]` for the unused-portion of the audit-log
   commitment decoding path).
2. **The `external_da_receipt` path never crosses the `state/` module
   boundary.** Production-SLA enforcement (`service_level`,
   `retention_seconds >= 30d`, `https://` endpoint, 32-byte
   `audit_log_commitment`) lives only in `cli/src/main.rs:3398`. Any
   consumer of `myelin-state` that wants to verify a receipt must
   re-implement this check; there is no state-level verifier.
3. **`SegmentMeta::sealed = true` is purely informational.** There is no
   API that *prevents* writes to a sealed segment ID; the writer rotates
   the in-memory current segment away from a sealed one, but a process
   re-opening with `SegmentWriter::new` will silently re-attach to an
   unsealed `segment_*.dat` that has no `.meta` file, recompute the
   merkle root from a chunk index that the same writer controls, and
   report `sealed = true` with a fresh root. This is the documented
   "recovery" flow (`test_segment_writer_recovers_unsealed_segment_after_restart`),
   so it is by design — but it means the seal is **the writer's claim,
   not a cryptographic binding to the data**.

Also material:

- `memmap2 = "0.9"` is a Cargo.toml entry that **no source file imports**.
  Dead dependency. The README's "1GB append-only files with mmap" claim
  is **false** — the writer uses `OpenOptions::new().append(true)` +
  `file.sync_data()`, not mmap.
- `indexmap = "=2.2.6"` is declared but never imported in `state/src`.
  Dead dependency.
- `lru` *is* used (`segment.rs:349`) but only as an open-file cache with
  capacity 8; eviction is LRU but the writer holds `File` objects outside
  the LRU. Not a security issue, but inconsistent with the cache's
  semantics.
- The cell-tree API `insert_with_outpoint` performs **silent cross-cell
  eviction** when a new outpoint collides with an existing one
  (`cell_tree.rs:198-207`): the prior `cells[hash_A]` entry is removed
  without any signal. The accumulated `muhash` is correctly updated, but
  callers that rely on `outpoint_hashes` for stable pagination can be
  surprised.

## Findings

| # | Severity | Component | Finding | File:Line |
|---|----------|-----------|---------|-----------|
| F-01 | CRITICAL | store/proof | `compute_merkle_root_from_leaves(&[])` returns `[0u8; 32]`; a SegmentProof with `merkle_path = []` and `leaf = [0u8;32]` verifies against that root. Combined with the fact that the seal sets the segment root from the writer's chunk index, an attacker who can write an empty segment and a forged `segment_root = 0x00…00` gets a valid proof. | `state/src/store/proof.rs:201-203` |
| F-02 | CRITICAL | store/segment | `SegmentMeta::sealed` is not a write gate. The writer's `append` always writes to the *current* segment, but `SegmentWriter::new` re-opens the most recent `segment_*.dat` that has no `.meta` and treats it as the active writer. An attacker who deletes `segment_NNNNNNNN.meta` (or the whole `*.idx`) and replays fresh appends to a previously-sealed segment ID gets a new `merkle_root` from a self-authored chunk index. The "seal" is the writer's *assertion*, not a cryptographic witness over the bytes. | `state/src/store/segment.rs:113-140`, `247-266` |
| F-03 | CRITICAL | cli/main + state boundary | External DA receipt is parsed and verified **only** in `cli/src/main.rs`. There is no `state::external_da_receipt` module. Any other consumer of `myelin-state` (a future SDK, an embedded verifier, a watcher) cannot reuse the schema/SLA check. The README's "external DA receipt" claim does not match the crate boundary. | `cli/src/main.rs:2840-3003, 3398-3409` |
| F-04 | HIGH | store/segment | On `write_all` failure during `append`, the chunk index is **not** updated but the file may have written some bytes. The function returns the error with `?` while holding `current_file` and `current_offset` locks. On retry / next append, the offset is *stale* and the new data is appended *after* the partial bytes, leaving the on-disk file out of sync with `current_chunks` and the segment size. `sync_data()` only runs on success, so durability for that chunk is also undefined. | `state/src/store/segment.rs:145-176` |
| F-05 | HIGH | cell_tree | `insert_with_outpoint` silently evicts a prior cell at a different `outpoint_hash` when the new outpoint collides with one already indexed (`outpoint_hashes` returns the prior hash, code path at line 200-206 removes the prior `cells[hash]`, `outpoints_by_hash[hash]`, `leaf_hashes[hash]`, and rolls the muhash). The function name and docstring suggest "insert", not "insert and possibly evict". This is a contract surprise. | `state/src/cell_tree.rs:194-217` |
| F-06 | HIGH | state/Cargo.toml | `memmap2 = "0.9"` is declared but **no `use memmap2::…`** in `state/src`. Confirmed by `rg '^use ' state/src` and `rg memmap2 state/`. The README "1GB append-only files with mmap" is therefore incorrect — the writer uses std-fs append + `sync_data()`. | `state/Cargo.toml:34`, `state/README.md:11,34-37` |
| F-07 | HIGH | state/Cargo.toml | `indexmap = "=2.2.6"` is declared but **never imported** in `state/src` (`rg indexmap state/src` → 0 hits). Dead dep. | `state/Cargo.toml:31` |
| F-08 | HIGH | cli/main | `audit_log_commitment` is validated as `len() == 64 && hex::decode(commitment).is_ok()` — this allows any 32-byte hex string (e.g. all-zeros, a tx hash, a random 32 bytes) to satisfy the check. There is no commitment-content check (e.g. must be a blake3 hash, must be a previously-issued commitment). The "32-byte audit_log_commitment" production SLA field is therefore a syntactic not a semantic check. | `cli/src/main.rs:3408` |
| F-09 | HIGH | state/README | The README's CF list mentions `cells_by_lock`, `segments`, `spend_journal`, but the code defines `cells`, `spent`, `spend_journal` (cell_db.rs:16-18) and `lock_index`, `type_index` (script_index.rs:15-16). The README also references an `kv/` module that doesn't exist (`# KV abstraction layer`), a `writer.rs` (only `segment.rs` exists with the writer inside), and a `reorg/spend_journal.rs` that doesn't exist. README is significantly out of sync with implementation. | `state/README.md:17-30, 42-47` |
| F-10 | MEDIUM | store/proof | `MerkleTreeBuilder::get_proof` is implemented for an *even-only* tree — when the final level has an odd node, `build_next_level` lifts the unpaired node without hashing (line 194: `chunk[0]`), so the sibling path for an index whose subtree had an odd number of leaves is missing one hash. The verifier at `verify_merkle_proof` doesn't notice (it just iterates the path), but this is the standard Bitcoin/SSZ convention; a node at the highest odd position has no sibling in the path. Correctness is preserved by the convention, but it's not documented and a future maintainer reading only the verifier would not realize that `path.len() < ceil(log2(n))` for some indices is intentional. | `state/src/store/proof.rs:147-167, 191-198` |
| F-11 | MEDIUM | index/cell_db | `batch_spend_in_block` does a `get_cf` outside the `WriteBatch` (line 327) and then constructs the batch from the read. The in-process `write_lock` prevents races between writers in this process, but it does not protect against an external RocksDB writer (e.g. another process opening the same DB directory). RocksDB does not support multi-process transactions against a single `WriteBatch`. If multi-process access is ever supported, this read-then-write pattern becomes a TOCTOU. | `state/src/index/cell_db.rs:312-343` |
| F-12 | MEDIUM | index/cell_db | `Options::default()` is used for the per-CF `ColumnFamilyDescriptor`s (line 155-157) while the DB-level `Options` sets `Snappy` compression. Per-CF compression in RocksDB overrides the DB-level setting with `Options::default()`'s `None`. The README's "snappy" feature is therefore enabled in the feature flag but only applied to the DB's *default* column family metadata, not to the actual `cells` / `spent` / `spend_journal` column families. | `state/src/index/cell_db.rs:147-163` |
| F-13 | MEDIUM | index/cell_db | `cf_handle` is called on every public method (e.g. 14 `cf_handle` lookups in `cell_db.rs`). The handle is stable for the DB's lifetime but is re-fetched per call. Cheap but noisy. | `state/src/index/cell_db.rs:171, 190-193, 219, 239-242, …` |
| F-14 | MEDIUM | cli/main | `da_availability_evidence` recomputes the entire availability commitment on every call (line 3420-3550). The committee is hard-coded with three `da-node-N` keys (`[0x31;32]`, `[0x32;32]`, `[0x33;32]`) and the secp256k1 signatures are produced on the fly. The "attestation" is therefore *self-signed by the test fixture*, not by a real DA committee. The `availability_commitment` is meaningful as a deterministic digest but the `attestation_signatures` carry no external trust. | `cli/src/main.rs:3456-3496` |
| F-15 | MEDIUM | state/Cargo.toml | `rocksdb = { version = "0.24", default-features = false, features = ["snappy"] }`. The `default-features = false` strips the `jemalloc` feature, which is normally a **good** thing (jemalloc is controversial in Linux-only deployments). However it also strips `multi-threaded-cf` and other defaults that may matter. The `snappy` feature is the only one explicitly kept, and the dep tree confirms `librocksdb-sys` is built statically with `snappy` enabled. This is internally consistent, but the comment in the Cargo.toml doesn't explain the choice. | `state/Cargo.toml:14` |
| F-16 | MEDIUM | store/segment | The LRU cache is `Arc<Mutex<lru::LruCache<u32, File>>>` with capacity `MAX_OPEN_SEGMENTS = 8`. Holds `std::fs::File` objects, not `Arc<File>`. The reader clones the file via `file.try_clone()?` (line 377) for each read, then drops the clone. The cache mutex is held across the seek+read on the cloned handle — a slow disk on one segment blocks reads of all other segments through the cache mutex. The `try_clone` is necessary because the read is `&self` (and the `File` is in the cache by value), but the mutex-held window is wider than required. | `state/src/store/segment.rs:362-382` |
| F-17 | LOW | store/proof | `SegmentProof::new` is public and the `merkle_path` field is `pub`. A caller can construct a `SegmentProof` with a non-empty `chunk_data` and an *empty* `merkle_path`, then call `verify` and the verifier happily walks zero siblings before comparing to root. With `chunk_data` non-empty the leaf hash is non-zero, so a `root == leaf` (single-leaf tree) verifies. With `chunk_data` empty and `root == 0x00…00` it also verifies (see F-01). The `verify` function should assert `merkle_path.len() == ceil(log2(leaves))` or take a `MerkleTreeBuilder` reference. | `state/src/store/proof.rs:31-52, 222-232` |
| F-18 | LOW | store/proof | `hash_leaf` and `hash_internal` use blake3 with distinct domain strings (`myelin-segment/leaf`, `myelin-segment/node`). The ckb / exec / molecule layer in this repo uses CKB's blake2b with `ckb-default-hash` personalization. Mixing hash families in the same DA evidence document is fine for soundness (domain separation) but means the proof can't be cheaply cross-verified by a CKB-native verifier that only knows blake2b. | `state/src/store/proof.rs:176-189` |
| F-19 | LOW | molecule | `decode_table` in `state/src/molecule.rs:81-84` uses `first_offset != min_size` as a structural check, where `min_size = 4 + expected_fields * 4`. The check works but differs from the strict exec `decode_table` which derives `field_count = first_offset / 4 - 1` and asserts equality with `expected_fields` and also enforces `first_offset % 4 == 0` and `<= bytes.len()`. The state version is weaker (it accepts a `first_offset` that exactly matches min_size but doesn't verify all offset entries in the header are in non-decreasing order). | `state/src/molecule.rs:70-102` vs `exec/src/serialization/molecule_compat.rs:1091-1124` |
| F-20 | LOW | molecule | `decode_dynvec` in `state/src/molecule.rs:139-141` checks `first_offset < NUMBER_SIZE || first_offset > total_size || first_offset % NUMBER_SIZE != 0` but does not enforce that `header_end == first_offset` after the loop, nor that the items fit between `header_end` and `total_size`. A malformed buffer with a `first_offset` larger than `header_end` (impossible from `encode_dynvec` but possible from a malicious peer) is accepted. The exec version's `decode_dynvec` is similarly lax; the weakness is shared. | `state/src/molecule.rs:125-161` |
| F-21 | LOW | index/cell_db | `encode_cell_meta` calls `serialize_cell_output_molecule` which returns `Result<Vec<u8>, MoleculeError>`. The error is mapped via `.map_err(|error| StateError::Serialization(error.to_string()))`. A serialization error on a `CellOutput` becomes a `Serialization` error — same enum, but the message string is lossy (the `MoleculeError` variants `InvalidHeaderVersion`, `OutPointHashLength`, etc. all collapse to one string). The two crates should share a richer error type. | `state/src/index/cell_db.rs:97-107` |
| F-22 | LOW | cell_tree | `MuHash` is a multiplicative accumulator over the multiplicative group mod a 521-bit prime. The README describes the cell tree as a "Merkle tree for live cells", and the public field is `cells: BTreeMap<Hash, CellEntry>`. The `root()` returns the finalized MuHash value, not a Merkle root. The README's `MerkleBranchHash` is used only for the *leaf* hashing (`compute_leaf_hash`); the *tree* shape is not a Merkle tree at all. A user that expects to verify inclusion via a Merkle path will be surprised — there are no siblings. | `state/src/cell_tree.rs:5, 144-178, 256-278` |
| F-23 | INFO | store/segment | `SegmentWriter::new` reads `find_max_segment_id` from the directory and either reopens the most recent dat file (if no `.meta` exists) or starts a fresh one. The recovery path (`test_segment_writer_recovers_unsealed_segment_after_restart`) is exercised by a test and the logic is correct, but the recovery semantics — "an unsealed segment can be appended to and re-sealed" — is a deliberate trust-on-first-use model. There is no anti-rollback (no monotonic `sealed_at` chain, no hash of previous-segment meta in the new meta). A malicious operator with directory access can append, seal, and produce an unrelated merkle root for the same segment ID over time. | `state/src/store/segment.rs:113-140, 562-580` |
| F-24 | INFO | state/lib | `pub mod cell_tree;` is exported with `pub use cell_tree::{CellEntry, CellStateTree}` (lib.rs:21). The `CellStateTree` is `Clone` and `pub` field `cells: BTreeMap<Hash, CellEntry>` is mutable through the public type. There is no `&mut self` boundary check on the tree. This is fine for a builder pattern, but means the tree is **not** safe to share across threads via `&CellStateTree` — it must be `RwLock<CellStateTree>` or similar at the call site. | `state/src/lib.rs:21`, `state/src/cell_tree.rs:144-178` |
| F-25 | INFO | state/Cargo.toml | The `tempfile` and `proptest` dev-deps are declared (lines 39-41) but `proptest` is not used by any test in the crate (`rg proptest state/src` → 0 hits in source, only in Cargo.toml). | `state/Cargo.toml:40-41` |

## Merkle proof verification

Walking the proof construction and verification, step by step:

| Step | Construct | Verify | Result |
|------|-----------|--------|--------|
| 1 | `hash_leaf(data)` = `blake3("myelin-segment/leaf" ‖ data)[:32]`. Domain-separated, deterministic. | Re-runs blake3 with the same prefix on `chunk_data`. | **VERIFIED** for non-empty `chunk_data` (test `test_proof_verification` passes). For empty `chunk_data`, `hash_leaf(&[])` is a deterministic 32-byte value, also VERIFIED. |
| 2 | `hash_internal(L, R)` = `blake3("myelin-segment/node" ‖ L ‖ R)[:32]`. Note: the *order* of L and R is preserved — the position of "current" vs "sibling" is determined by the leaf index. | Same function called with the same arguments when verifying. | **VERIFIED** — the verifier uses `is_multiple_of(2)` to decide left/right, matching the builder. |
| 3 | `build_next_level` lifts the unpaired node at odd length: `hash = if chunk.len() == 2 { hash_internal(...) } else { chunk[0] }`. This is the standard Bitcoin/SSZ convention. | The verifier never sees a sibling for the lifted node, so the path length is `ceil(log2(n)) - 1` for the rightmost leaf. The `verify_merkle_proof` loop terminates with `current` equal to the lifted value. | **VERIFIED** by convention, but the convention is **not enforced**. A proof with `merkle_path` shorter than `ceil(log2(n))` for non-lifted leaves is rejected only by chance (the recomputed root won't match). For the lifted leaf it's accepted. |
| 4 | `get_proof(index)` walks the levels, at each level pushing `level[sibling_index]` if it exists. If the index is the lifted (last) node at an odd level, the sibling doesn't exist, **no sibling is pushed** (the `if sibling_index < level.len()` guard). | The verifier walks the same path. | **VERIFIED** for the typical case. UNVERIFIED for the empty-tree case (see step 5). |
| 5 | **Empty-tree case**: `compute_merkle_root_from_leaves(&[])` returns `[0u8; 32]`. `MerkleTreeBuilder::new` and `.build()` with no leaves produce the same. `get_proof(0)` on an empty builder returns `vec![]` *and* also returns `vec![]` for any `index >= 0`. `verify_merkle_proof(&[0u8;32], &[], &[0u8;32], 0)` is `true`. | The verify path is happy. | **UNVERIFIED for the security case** — a `SegmentProof { segment_id: 0, leaf_index: 0, chunk_data: vec![], chunk_offset: 0, chunk_length: 0, merkle_path: vec![], segment_root: [0u8;32] }` *passes* `verify()`. The contract "the proof binds a payload hash to a segment root" is false for the empty payload. |
| 6 | `chunk_data.len() != chunk_length as usize` returns `Ok(false)` (defensive). | Length mismatch is rejected before the merkle walk. | **VERIFIED**. |
| 7 | Molecule roundtrip via `to_molecule_bytes` / `from_molecule_bytes` covers 7 fields including the `merkle_path` dynvec. | Test `test_segment_proof_molecule_roundtrip` passes for a 2-leaf tree. | **VERIFIED**. |
| 8 | `verify_merkle_proof` takes a `&[[u8; 32]]` proof and trusts `index` to be honest. The verifier does *not* check that `index < n` (number of leaves). | A proof with `index = 1_000_000` against a 4-leaf tree with the right siblings at level 0/1 still walks to a wrong root. | **VERIFIED** to *fail* — the recomputed root won't match a real 4-leaf root. But the failure is silent (returns `Ok(false)`). |

**Summary:** the Merkle proof is sound for non-empty segments. It is **not** sound for the empty-segments-as-`[0u8;32]` convention: an empty proof against `[0u8;32]` is a valid proof, and any caller that doesn't reject empty segments at the protocol layer is exposed.

## Sealed-segment tamper matrix

After `meta.sealed = true` is set and `save_segment_meta` writes the `.meta` file, what can an attacker with file-system write access to the segment directory do?

| Action | Detected? | Effect |
|--------|-----------|--------|
| Append bytes to `segment_NNNNNNNN.dat` after seal | Not detected by the writer on next `SegmentWriter::new` only if the `.meta` is deleted; the writer will re-open the segment and re-seal with a *new* merkle root computed from the appended bytes + existing chunk index. The new root is the writer's choice. | If `.meta` is intact, the writer treats this segment as sealed and starts a new segment ID. The new bytes are silently truncated on the next `rotate_segment` (the writer reopens in `append` mode, so it writes at end-of-file). The on-disk file grows but no `cells` pointer references the new bytes. **Partial write tamper possible.** |
| Truncate `segment_NNNNNNNN.dat` to a smaller size | Not detected. The chunk index in `.idx` references offsets that are now out of bounds. `build_proof` calls `read_exact` which returns `Err`. The reader's `resolve_segment_root` falls back to `builder.build()` from in-memory data… which would also fail. The proof generation errors out. | **Detected** at proof-construction time, but only at runtime. A consumer of `load_meta` who only reads the meta sees `sealed = true` and trusts it. |
| Modify bytes inside `segment_NNNNNNNN.dat` (preserving length) | The merkle root in `.meta` was computed from these bytes (because `compute_merkle_root` reads them at seal time). On reload, `resolve_segment_root` returns the *stored* `merkle_root` from `.meta` (line 401-408), so the proof is built against the original root. `build_proof` then reads the *current* bytes from the file, hashes them, and walks the path. The leaf hash will not match the merkle root, so `proof.verify()` returns `Ok(false)`. | **Detected** at proof verification time. |
| Replace `segment_NNNNNNNN.dat` wholesale | Same as above: the on-disk bytes don't match the merkle root in `.meta`. | **Detected.** |
| Modify `segment_NNNNNNNN.meta` to a new merkle root | If the attacker also modifies `segment_NNNNNNNN.dat` to match the new root, the proof is *consistent* with the new root. The seal is "the writer's word" — there is no chain binding the seal to a previous state. | **Not detected.** A new merkle root + matching bytes passes every check. |
| Modify `segment_NNNNNNNN.idx` (chunk index) | `compute_merkle_root` at seal time uses the writer's in-memory `current_chunks` *and reads from the file*. The merkle root is committed to the file contents at the chunk boundaries the writer believed in. If the `.idx` is modified to point to different offsets, `build_proof` (which re-reads the file at the modified offsets) will produce a proof against a root that the `.meta` no longer claims. The on-disk `.meta` root and the recomputed root diverge — `build_proof` at line 423-428 returns `InvalidProof`. | **Detected** at proof-build time, *if* the consumer calls `build_proof`. A consumer that trusts only `load_meta` sees the stale root. |
| Delete `segment_NNNNNNNN.meta` | `find_max_segment_id` finds the `.dat` file. `SegmentWriter::new` reopens the `.dat`, loads (or re-creates) the `.idx`, treats the segment as the active one, accepts new appends, and seals with a fresh root. | **Not detected** at the file-system level. The "seal" can be reversed by deleting one file. |

**What can't the attacker do?** They can't forge a proof against a merkle root they don't know, *as long as* the merkle root is independently anchored (e.g. embedded in a CKB CellTx as in the `da-anchor-package` flow). The chain binding is the anchor cell, not the local segment file. The local segment is an evidence cache; the canonical commitment is whatever the anchor package embeds.

## Receipt / production-SLA schema check

The README's promise (lines 311-321): a `production_ready = true` receipt
must carry `service_level = "production"`, `retention_seconds >= 30 days`,
an HTTPS `retrieval_endpoint`, and a 32-byte `audit_log_commitment`, with
the provider's secp256k1 signature covering the SLA fields.

| Field | Where parsed | Where enforced | Where the binding to the manifest lives |
|-------|--------------|----------------|------------------------------------------|
| `schema` = `"myelin-external-da-receipt-v2"` | `cli/src/main.rs:2856-2864` | Same; rejected on mismatch. | None outside the JSON. |
| `provider`, `namespace`, `receipt_id`, `availability_window` (required strings) | `cli/src/main.rs:2865-2881` | All non-empty. | Folded into `provider_message_hash` for signature. |
| `payload_hash` (required, 32-byte hex) | `cli/src/main.rs:2877, 2904-2916` | Must equal the manifest's `molecule_transaction_hash`. | `provider_message_hash` binds it. |
| `segment_root` (required, 32-byte hex) | `cli/src/main.rs:2878, 2909-2918` | Must equal the manifest's `segment_root`. | `provider_message_hash` binds it. |
| `provider_pubkey_hash` (required, 20 bytes) | `cli/src/main.rs:2881, 2920-2926` | Length-checked. | Used in secp256k1 recovery. |
| `provider_signature` (required, 65 bytes) | `cli/src/main.rs:2882, 2927-2935` | Length-checked, then secp256k1-verified against `provider_message_hash` and `provider_pubkey_hash` at line 2950-2953. | The signature is the binding; `provider_message_hash` includes all other fields. |
| `service_level` (optional) | `cli/src/main.rs:2895-2897` | `external_da_receipt_production_guarantee_checked` (line 3405): must be `Some("production")`. | `provider_message_hash` includes it (line 3031), so it is signature-covered. |
| `retention_seconds` (optional u64) | `cli/src/main.rs:2898-2903` | Line 3406: `>= 30 * 24 * 60 * 60`. | `provider_message_hash` includes its string form (line 3032). |
| `retrieval_endpoint` (optional string) | `cli/src/main.rs:2896-2897` | Line 3407: must start with `https://` and have length > 8. **Note:** this is a *syntactic* check, not a connectivity or certificate check. | `provider_message_hash` includes it. |
| `audit_log_commitment` (optional string) | `cli/src/main.rs:2897` | Line 3408: `len() == 64 && hex::decode(commitment).is_ok()`. **Note:** this does not check the commitment against any actual audit log; any 32-byte hex passes. | `provider_message_hash` includes it. |
| `receipt_hash` (recomputed) | `cli/src/main.rs:2955` | `blake3("myelin:external-da-receipt-document:v2", receipt_bytes)`. | Includes the *raw* receipt bytes, so any field mutation invalidates the hash. |
| `receipt_commitment` (recomputed) | `cli/src/main.rs:2957-2975` | blake3 over a chunked list of all fields. | Cross-checks the typed values. |
| `production_guarantee_checked` (recomputed) | `cli/src/main.rs:2976-2981, 3398-3409` | Boolean. | Embedded in `availability_commitment` (line 3515). |

**Where the schema is enforced:** `cli/src/main.rs` only. There is no
`myelin_state::external_da_receipt` module, no public re-export, no
state-level schema type. A future consumer (SDK, on-chain verifier,
watcher) must re-implement the entire parser and the four production-SLA
checks. This is **F-03 (CRITICAL)**.

**Where the receipt binds to the manifest:**
`external_da_receipt_evidence` (line 2840) takes the manifest's
`molecule_transaction_hash` and `segment_root` as arguments and compares
them to the receipt's `payload_hash` and `segment_root`. The signature's
message hash (`provider_message_hash`, line 3019-3037) covers both
fields. So the binding is:

```
secp256k1_recover(blake3("myelin:external-da-receipt-provider-signature:v2",
                         schema, provider, namespace, payload_hash,
                         segment_root, receipt_id, availability_window,
                         service_level, retention_seconds,
                         retrieval_endpoint, audit_log_commitment)) == provider_pubkey_hash
```

and additionally

```
blake3("myelin:external-da-receipt-document:v2", receipt_bytes) == receipt_hash
```

`receipt_commitment` is a cross-check over the typed fields. The full
availability commitment (line 3442-3519) folds in `receipt_hash`,
`receipt_commitment`, `provider_message_hash`,
`provider_signature_verified`, and `production_guarantee_checked`.

## Open questions

1. **Empty-segment root collision.** Is `[0u8; 32]` the intended empty
   root, or is it an oversight of the `leaves.is_empty()` short-circuit
   in `compute_merkle_root_from_leaves`? If the former, the spec should
   explicitly forbid `SegmentProof { segment_root: [0u8;32], chunk_data: vec![] }`.
   If the latter, the empty case should error.

2. **Seal is the writer's word.** Should the seal `merkle_root` be
   cross-referenced to the chunk index hash (e.g.
   `seal_hash = blake3("myelin:segment/seal", merkle_root, chunk_index_hash)`)
   so that modifying the chunk index after seal is detectable without a
   proof-construction roundtrip?

3. **External-DA-receipt module boundary.** The state crate's `README`
   describes "DA Proofs" but does not mention the receipt path. Is the
   intent that the receipt parser lives in `state/` (under e.g.
   `state::external_da_receipt`)? The current placement in `cli/` makes
   it impossible to reuse from any non-CLI consumer.

4. **Cell-tree `insert_with_outpoint` silent eviction.** Is the
   cross-cell eviction at `cell_tree.rs:198-207` intentional (a "merge
   two views of the same cell") or a bug? If intentional, the docstring
   should describe it. If a bug, the eviction should be removed and the
   call should be a no-op (or return an error) on outpoint collision.

5. **README ↔ implementation drift.** `state/README.md` describes column
   families, modules, and behaviors that don't match the code (see
   F-09). Should the README be brought in line, or is the README the
   forward-looking spec and the code the not-yet-merged substrate?

6. **Multi-process RocksDB safety.** `batch_spend_in_block` and
   `remove_live_cell` assume the in-process `write_lock` is sufficient
   (F-11). Is multi-process access ever a target? If so, the read-then-
   write pattern must be replaced with a `Transaction` or with
   optimistic-concurrency `merge` operators.

7. **`memmap2` and `indexmap`.** These are declared in `Cargo.toml` but
   never imported. The README cites mmap as a design principle (F-06)
   and `indexmap` is included with a `=2.2.6` pin. Are these forward
   imports, or should they be removed?

8. **`audit_log_commitment` semantics.** The check at `main.rs:3408`
   accepts any 32-byte hex. Should the receipt schema require the
   commitment to be a blake3 hash of a verifiable audit log, or
   reference a known commitment root published elsewhere?

9. **`lru` cache eviction safety.** The cache holds `File` objects; an
   LRU eviction closes the file. If a `SegmentReader` caller is in the
   middle of a `try_clone`+`seek`+`read_exact` (with the cache mutex
   *not* held after the clone), the eviction is safe. If the caller
   holds a reference to the cached file beyond the `files.lock()`
   scope… actually, the reader *does* hold a `&File` reference into the
   LRU after `files.get`, then re-locks-and-reads. The `&File` borrow
   survives only for the duration of the method body, so the eviction
   can't fire mid-method. But the borrow is technically `&` from a
   `Mutex<...>`, and `LruCache::get` returns `&mut V` from `&mut self` —
   this works only because `parking_lot::Mutex` returns a `MutexGuard`
   that deref-coerces. The `&File` borrow from `files.get(&segment_id)`
   is tied to the `MutexGuard` lifetime. Confirmed safe in this method
   but fragile if extracted.

10. **State layer's `unsafe` usage.** No `unsafe { … }` blocks in
    `state/src` (verified by `rg unsafe state/src` → 0 hits). The
    RocksDB FFI is contained within the `rocksdb` crate. This is
    informationally positive: the state crate itself contributes no
    UB risk.
