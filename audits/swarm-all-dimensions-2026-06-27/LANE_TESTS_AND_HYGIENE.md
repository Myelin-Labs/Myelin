# Myelin Swarm Audit — Tests + Dependency Hygiene

> Verifier-only review. No fixes proposed. Scope: `tests/`, `benches/`,
> `examples/`, `exec/proptest-regressions/`, all workspace member
> `Cargo.toml`, every `build.rs`, the two vendored keccak `.s` files,
> the workspace `Cargo.toml` profiles / lints / dep inventory, and
> `Cargo.lock` size / advisories. Cross-references
> `audits/swarm-wholerepo/LANE_PRIMITIVES.md` (F-PRIM-18 keccak),
> `MYELIN_SWARM_AUDIT_STATE_DA.md` (F-06 memmap2, F-07 indexmap),
> `MYELIN_SWARM_AUDIT_MEMPOOL_CONSENSUS.md` (cellpool RBF recursion).
>
> Workspace compiles clean (`cargo check --workspace --all-targets` ✓).
> `cargo audit` clean of CVEs but flags 3 unsound + 1 unmaintained advisory.

## Verdict

**Conditional PASS with three HIGH and one CRITICAL hygiene defect that
should gate any public release.** Test integrity is strong for the
production-evidence-relevant lanes (typed-cell vectors hex-pinned,
vm_abi serializer cross-checked, cellscript `ckb_compat_runner.rs`
builds its own semantic model and cross-checks fixture verdicts, the
registry tests round-trip the source hash, the ickb_diff matrix is
provably executed-CKB-VM-differential). The hygiene defects are:

1. **`memmap2@0.9.10` is a workspace dependency, declared in
   `state/Cargo.toml:34`, never imported anywhere in `state/src/`,
   and the dep is now UNSOUND per `RUSTSEC-2026-0186`** ("Unchecked
   pointer offset in crate `memmap2`"). It is dead AND a known
   safety defect — exactly the bad combination. STATE_DA F-06 already
   flagged it as dead; this audit confirms and adds the CVE finding.
2. **`lru@0.12.5` is a runtime dep of `state/`
   (`segment.rs:349` open-file cache), now UNSOUND per
   `RUSTSEC-2026-0002`** ("`IterMut` violates Stacked Borrows by
   invalidating internal pointer"). state/src does not currently
   call `IterMut`, so the live attack surface is zero today, but
   the cache is reachable from any reader through the public
   `SegmentReader` API.
3. **Workspace-level dead-dep inventory is large (≈70 of 132
   declared workspace-deps never reach `cargo metadata`'s resolve
   graph).** Per-crate orphan count: 4 in `state/`, 3 in `exec/`,
   5 in `cellscript/`, 2 in `crypto/hashes/`, 1 in `utils/`. Plus 1
   dead workspace-level optional dep (`keccak` per F-PRIM-18).
4. **`exec/proptest-regressions/` is NOT empty by accident.** It
   contains exactly one file (`celltx/types.txt`, 7 lines) — a
   proptest regression-seed file written by `proptest!` at
   `exec/src/celltx/types.rs:2822` when it found a shrinking
   failing case. The brief's framing ("0 .rs files") misreads the
   convention; the directory is correctly populated and the seed
   file is in `git ls-files`.

The fixture-vs-implementation alignment is sound on every lane I
checked (compatibility runner, registry lockfile, examples.rs
backend-shape baseline) — no fixtures encode behavior the producer
no longer has, and the cellscript v0_18 typed-cell tests **do**
compile-assert the carrier→final-script metadata that the prior
audit's F-DOC-01/F-DOC-05 flagged as orphan (the fixtures exercise
it; the CLI helper is the orphan, not the fixture).

Test integrity on the strongest test files is high; the weakest
test integrity is concentrated in **state/src/store/proof.rs**
(7/7 tests are positive-only — no tampered-proof or wrong-leaf
negative cases, beyond the trivial `test_proof_verification` that
only proves "the verifier agrees with the builder"), **exec/src
bench profile** (overflow checks off; see F-HYG-09 below), and the
sighash cross-check pair (`compute_txid` vs `compute_wtxid` only
differs by `assert_ne!` per F-PRIM-20 — same defect, restated).

## Findings

| # | Severity | Component | Finding | File:Line |
|---|----------|-----------|---------|-----------|
| F-HYG-01 | **CRITICAL** | state/Cargo.toml | `memmap2 = "0.9"` is declared but **never imported anywhere** in `state/src/` (verified by `rg 'memmap2' state/src` → 0 hits). The dep is also UNSOUND per `RUSTSEC-2026-0186`. Dead AND a known safety defect. STATE_DA F-06 already flagged as dead; this audit confirms + adds CVE. | `state/Cargo.toml:34` |
| F-HYG-02 | **HIGH** | state/Cargo.toml | `lru = "0.12"` is declared, used at `segment.rs:349` (open-file cache), and **UNSOUND per `RUSTSEC-2026-0002`** ("`IterMut` violates Stacked Borrows"). No `IterMut` call in current state/src, but `lru::LruCache::iter_mut` is reachable through the public `SegmentReader` API. | `state/Cargo.toml:37`, `state/src/store/segment.rs:349`, advisory `RUSTSEC-2026-0002` |
| F-HYG-03 | **HIGH** | workspace | `paste@1.0.15` (transitive via `ckb-vm-definitions → ckb-vm → myelin-exec → myelin-state`) is **UNMAINTAINED per `RUSTSEC-2024-0436`**. No published replacement path; ckb-vm-definitions pins an old macro crate. | `Cargo.lock` line for paste, advisory `RUSTSEC-2024-0436` |
| F-HYG-04 | **HIGH** | workspace | `rand@0.7.3` (transitive via ckb-vm, used only by ckb-vm internals) is **UNSOUND per `RUSTSEC-2026-0097`** ("Rand is unsound with a custom logger using `rand::rng()`"). Myelin does not enable the `log` feature of `rand@0.7.3`, so the live attack surface is zero today; the unsound combination is not reachable. | `Cargo.lock`, advisory `RUSTSEC-2026-0097` |
| F-HYG-05 | MEDIUM | exec/Cargo.toml | `anyhow`, `indexmap`, `byteorder` are declared but **never imported in `exec/src/`** (verified by `rg`). `byteorder` is `optional = true` and gated on the `vm` feature; even with the vm feature enabled, no `use byteorder` exists. `anyhow` and `indexmap` are dead unconditionally. | `exec/Cargo.toml:22, 25, 33` |
| F-HYG-06 | MEDIUM | state/Cargo.toml | `proptest = "1.4"` is declared as dev-dep but **no `proptest!` macro or `use proptest` in `state/src/`**. STATE_DA F-25 already noted this. Dead. | `state/Cargo.toml:41`, `rg 'proptest' state/src` → 0 hits |
| F-HYG-07 | MEDIUM | cellscript/Cargo.toml | `anyhow`, `thiserror`, `indexmap`, `log`, `pretty_assertions` are declared but **never imported in `cellscript/src/` or `cellscript/tests/`**. `env_logger` is used (main.rs:138) but no `log::` macros exist. `pretty_assertions = "1.4"` is in `[dev-dependencies]` and not used anywhere. | `cellscript/Cargo.toml:46, 56, 58, 79`; `rg 'use anyhow\|use thiserror\|use indexmap\|use log\|use pretty_assertions' cellscript/src cellscript/tests` → 0 hits |
| F-HYG-08 | MEDIUM | utils/Cargo.toml | `sha2` is declared as a workspace dep but **never imported in `utils/src/`** (used in `crypto/hashes/src/hashers.rs:11` and `cellscript/tests/ickb_diff.rs:3` via separate declarations, not from utils). Dead in this crate. | `utils/Cargo.toml:28`, `rg 'use sha2\|sha2::' utils/src` → 0 hits |
| F-HYG-09 | MEDIUM | crypto/hashes/Cargo.toml | `sha3` is declared as dev-dep but **never imported in `crypto/hashes/`** (verified by `rg 'sha3' crypto/hashes/` → 0 hits outside Cargo.toml). Dead dev-dep. | `crypto/hashes/Cargo.toml:33` |
| F-HYG-10 | MEDIUM | workspace | `tonic`, `tonic-build`, `prost` are workspace-declared but **never reach `cargo metadata`'s resolve graph** for any target (x86_64-apple-darwin, wasm32-unknown-unknown, or any other). 70 of 132 workspace-deps are similarly never resolved (see "Workspace dead-dep inventory" below). | `Cargo.toml:126, 164-165`, `cargo metadata` |
| F-HYG-11 | MEDIUM | tests (cellscript) | cellscript/tests has **17 test files / 38K lines but ZERO `proptest!` macros and ZERO `proptest::prelude` imports**. All tests are unit/integration. The only property-based coverage in the entire workspace is `exec/src/celltx/types.rs:2822-2860` (3 proptest cases for `CellScriptSchedulerWitness::validate_access_set`). The cellscript "fuzzy" tests are hand-rolled xorshift RNG (`Rng64` struct in `tests/fuzzy_debug.rs:12-37`), not proptest. | `rg 'proptest!\|proptest::prelude' cellscript/tests/` → 0 hits |
| F-HYG-12 | MEDIUM | exec/proptest-regressions | Brief says "directory exists but is empty (0 .rs files)". Correct interpretation: proptest writes a `.txt` regression-seed file (NOT `.rs`), and the file is correctly populated. **The directory is correctly populated, not a missing artifact.** No remediation needed; the brief's framing is a misread. | `exec/proptest-regressions/celltx/types.txt` (7 lines, tracked in `git ls-files`) |
| F-HYG-13 | MEDIUM | profile / math | `UintN::as_f64` (`math/src/uint.rs:272-306`) silently overflows the f64 exponent field for `UintN` with `BITS > 1023` (F-PRIM-09). In `bench` profile (`overflow-checks = false`), the `debug_assert!(!carry, "attempt to shift left with overflow")` checks at uint.rs:536, 548, 560, 572, 584, 671, 683 are **stripped by opt-level=3**. Native `<<` ops in bench also do not trap. The bench can produce `+inf`/`NaN` from `as_f64` that release would catch. F-PRIM-37 already accepted this; extending. | `Cargo.toml:191-195` (`profile.bench`), `math/src/uint.rs:272-306, 536-683` |
| F-HYG-14 | MEDIUM | state/store/proof | **All 7 positive-only Merkle proof tests in `proof.rs` have no negative counterparts.** `test_proof_verification` (line 273) only proves "the verifier agrees with the builder". A tampered leaf, tampered path, wrong segment_root, or `SegmentProof { chunk_data: vec![], segment_root: [0u8;32] }` (F-01 collision) are not exercised. STATE_DA F-17 noted this is structural — the verifier does not assert `merkle_path.len() == ceil(log2(leaves))`. | `state/src/store/proof.rs:238-355` |
| F-HYG-15 | MEDIUM | cellscript | `cellscript/Cargo.toml` pin `clap = "=4.5.49"` is exact-pinned. Workspace has `clap = { version = "4.4.7" }` (compatible-floor). Mismatch: cellscript cannot pick up a workspace transitive bump; it's pinned forever until manually edited. (Aside: cellscript is in a separate nested workspace, see F-HYG-16.) | `cellscript/Cargo.toml:62` |
| F-HYG-16 | LOW | workspace | Workspace `Cargo.toml:3` `exclude = ["cellscript"]` — `cellscript/` is a **nested workspace** with its own `Cargo.toml` declaring a sub-workspace (`cellscript/Cargo.toml:1-9`). This is correct (cellscript is a vendored fork with `version = "0.17.0"` independent of the `0.1.0` Myelin crates), but the two workspaces share `Cargo.lock` (cellscript/Cargo.lock) only for cellscript's tree; the parent workspace's `Cargo.lock` does not include cellscript. | `Cargo.toml:3`, `cellscript/Cargo.toml:1-9`, `cellscript/Cargo.lock` |
| F-HYG-17 | LOW | workspace | `[workspace.lints.clippy]` sets only `empty_docs = "allow"`. None of the 10 in-scope crates declares a `[lints]` table to inherit. The single override is `crypto/muhash/Cargo.toml:30` which sets `unexpected_cfgs` for the `cfg(fuzzing)` predicate — useful but doesn't propagate `clippy` config. **Effectively no clippy enforcement at the workspace level.** Per LANE_PRIMITIVES F-PRIM-32. | `Cargo.toml:202-203`, all `*/Cargo.toml` |
| F-HYG-18 | LOW | cellscript | `cellscript/Cargo.toml:46, 49, 51-54, 65` declare `thiserror`, `serde`, `blake2b_simd`, `blake3`, `toml`, `colored`, `camino` etc. as `[dependencies]` and several are used only in `cli/` (e.g. `colored::Colorize` is used only in `cellscript/src/main.rs:142`, `165, 181, 235` for terminal coloring) — fine. But the test-only `sha2 = "0.10"` (dev-dep, line 83) is used in `ickb_diff.rs:3`; `tempfile`, `pretty_assertions`, `regex`, `ckb-testtool`, `ckb-std`, `ckb-types`, `cellscript-ckb-adapter` are dev-deps and all used in `tests/`. This is hygiene-acceptable; no action needed. | `cellscript/Cargo.toml:78-86` |
| F-HYG-19 | LOW | build.rs (utils) | `utils/build.rs:46-48` reads `.git/HEAD` content and, if it starts with `ref: `, joins the rest to `git_folder` and checks `is_file()`. A hostile `.git/HEAD` containing `ref: ../../../../etc/passwd` would resolve to a path outside `.git/`. The check is benign (`is_file()` only), but the pattern is path-traversal-friendly. Mitigation: anyone who can write `.git/HEAD` already has repo write access, so the risk is informational. | `utils/build.rs:33-79` |
| F-HYG-20 | LOW | build.rs (crypto/hashes) | `crypto/hashes/build.rs:9-14` compiles `libkeccak.a` on x86_64 (linux non-windows or macos) but the artifact is never linked into any `extern "C"` declaration. Confirms F-PRIM-18. **No other vendored `.s` files exist in the workspace** (verified by `find . -name '*.s' -not -path '*/target/*'` → 2 keccak files plus 10 `.s` files inside `cellscript/examples/` which are CellScript example sources, not assembly). The cellscript `.s` files are not vendored assembly — they are `.cell` source examples named with `.s` extension for legacy reasons, and `cellscript/Cargo.toml:21-34` excludes `docs/`, `tools/`, `src/bin/` from package distribution. | `crypto/hashes/build.rs:1-16`, `crypto/hashes/Cargo.toml:18, 27`, `find . -name '*.s'` |

## Per-finding evidence trail

### F-HYG-01: state `memmap2` dead + UNSOUND

```
$ rg 'memmap2' state/src/
(no matches)

$ rg 'use memmap2\|memmap2::' state/src/
(no matches)

$ rg 'memmap2' state/Cargo.toml
34:memmap2 = "0.9"

$ cargo audit --json | jq '.warnings.unsound[] | select(.package.name=="memmap2")'
{
  "package": {"name":"memmap2","version":"0.9.10", ...},
  "advisory": {"id":"RUSTSEC-2026-0186", "title":"Unchecked pointer offset in crate `memmap2`"}
}

$ cargo tree -i memmap2@0.9.10
memmap2 v0.9.10
└── myelin-state v0.1.0 (/Users/arthur/RustroverProjects/Myelin/state)
    └── myelin-cli v0.1.0 (/Users/arthur/RustroverProjects/Myelin/cli)
```

STATE_DA F-06 already flagged memmap2 as dead (the README claim "1GB
append-only files with mmap" is false; the writer uses
`OpenOptions::new().append(true)` + `file.sync_data()`). This audit
extends: memmap2 is **also unsound** per RUSTSEC-2026-0186 ("flaw
was corrected in commit `cee7cf0` and released in version `0.9.11`").
The dep is both dead and unsafe. Recommend removal.

### F-HYG-02: state `lru` UNSOUND

```
$ rg 'use lru\|lru::' state/src/
state/src/store/segment.rs:11:    use std::sync::Arc;
state/src/store/segment.rs:349:    files: Arc<Mutex<lru::LruCache<u32, File>>>,
```

```
$ cargo audit --json | jq '.warnings.unsound[] | select(.package.name=="lru")'
{
  "package": {"name":"lru","version":"0.12.5", ...},
  "advisory": {"id":"RUSTSEC-2026-0002",
               "title":"`IterMut` violates Stacked Borrows by invalidating internal pointer"}
}
```

The advisory says: "`IterMut::next` and `IterMut::next_back` methods
temporarily create an exclusive reference to the key when
dereferencing the internal node pointer. This invalidates the shared
pointer held by the internal `HashMap`, violating Stacked Borrows
rules." Myelin's state/src does not call `IterMut` directly today,
but `lru::LruCache::iter_mut` is reachable through the `files`
field's public type. Live attack surface is zero today but the
advisory is a ticking time bomb. STATE_DA F-08 / F-16 already noted
the LRU cache holds `File` objects; this audit adds the CVE
classification.

### F-HYG-03: `paste@1.0.15` UNMAINTAINED

```
$ cargo audit --json | jq '.warnings.unmaintained[] | select(.package.name=="paste")'
{
  "advisory": {"id":"RUSTSEC-2024-0436", "title":"paste - no longer maintained"}
}

$ cargo tree -i paste@1.0.15
paste v1.0.15 (proc-macro)
└── ckb-vm-definitions v0.24.14
    └── ckb-vm v0.24.14
        └── myelin-exec v0.1.0
```

This is a transitive dep through ckb-vm. No Myelin code calls
`paste::expr!` directly. Per the advisory, `pastey` is the
suggested fork. Out of Myelin's direct control until ckb-vm bumps.

### F-HYG-04: `rand@0.7.3` UNSOUND (transitive)

```
$ cargo tree -i rand@0.7.3
rand v0.7.3
└── ckb-vm v0.24.14
    └── myelin-exec v0.1.0
        ├── myelin-cli v0.1.0
        ├── myelin-mempool v0.1.0
        └── myelin-state v0.1.0
```

The advisory RUSTSEC-2026-0097 only triggers when: (1) `log` and
`thread_rng` features of `rand@0.7.3` are enabled, AND (2) a custom
`log` logger is defined that calls `rand::rng()`. Myelin's
`Cargo.toml` does not enable the `log` feature of `rand@0.7.3`
(workspace declares `rand = "0.8.5"` for Myelin's own use, and
ckb-vm's vendored rand@0.7.3 doesn't pull `log`). **Live attack
surface: zero.** But the unsound combination is reachable in
principle for any future consumer who enables those features.

### F-HYG-05: exec `anyhow`, `indexmap`, `byteorder` dead

```
$ rg 'use anyhow\|anyhow::\|anyhow!' exec/
(no matches)

$ rg 'use indexmap\|IndexMap' exec/
(no matches)

$ rg 'use byteorder\|ByteOrder' exec/
(no matches)

$ rg 'byteorder' exec/Cargo.toml
33:byteorder = { version = "1.5", optional = true }
51:vm = ["ckb-vm", "byteorder", "hex"]  # VM integration enabled
```

`byteorder` is `optional = true` and gated on the `vm` feature. With
the `vm` feature enabled (workspace default: `default = ["vm"]`),
the build still does not link any byteorder-using code. The
`encode`/`decode` ops in `molecule_compat.rs` use native byteorder
(`u32::to_le_bytes`, etc.). All three are dead.

### F-HYG-06: state `proptest` dead dev-dep

```
$ rg 'proptest\|use proptest\|proptest!' state/src
(no matches)

$ rg 'proptest' state/Cargo.toml
41:proptest = "1.4"
```

Confirmed. STATE_DA F-25 noted this; this audit confirms.

### F-HYG-07: cellscript `anyhow`, `thiserror`, `indexmap`, `log`, `pretty_assertions` dead

```
$ rg 'use anyhow\|anyhow!' cellscript/src cellscript/tests
(no matches)

$ rg 'use thiserror\|thiserror::' cellscript/src cellscript/tests
(no matches)

$ rg 'use indexmap\|IndexMap' cellscript/src cellscript/tests
(no matches)

$ rg 'log::' cellscript/src cellscript/tests
(no matches)

$ rg 'use pretty_assertions\|pretty_assertions::' cellscript/src cellscript/tests
(no matches)

$ rg 'env_logger' cellscript/src
cellscript/src/main.rs:138:    env_logger::init();
```

`env_logger::init()` initializes the env-logger but no code uses
`log::info!`, `log::warn!`, `log::debug!`, `log::trace!`, `log::error!`.
The `log` crate is dead.

### F-HYG-08: utils `sha2` dead

```
$ rg 'use sha2\|sha2::' utils/src
(no matches)

$ rg 'sha2' utils/Cargo.toml
28:sha2.workspace = true
```

`utils/Cargo.toml` line 28 declares sha2.workspace = true but no
src/ file in utils uses it. The workspace declares sha2 =
`"0.10.8"` and is used in `crypto/hashes/src/hashers.rs:11` and
`cellscript/tests/ickb_diff.rs:3` via their own declarations (not
via utils). Dead in utils.

### F-HYG-09: crypto/hashes `sha3` dead dev-dep

```
$ rg 'sha3' crypto/hashes/
crypto/hashes/Cargo.toml:33:sha3.workspace = true
```

Only one match. Dead dev-dep.

### F-HYG-10: 70 workspace-deps never resolved

The workspace declares 132 deps in `[workspace.dependencies]`. 70
of them (after filtering out profile keys like `codegen-units`,
`opt-level`, `overflow-checks`, `strip`, `lto`, `debug` that my
regex bled into) **never appear in `cargo metadata`'s resolve graph
for any platform**:

| Category | Count | Examples |
|---|---|---|
| Never consumed by any member | 70 | aes, argon2, async-std, base64, bech32, bitcoin, bs58, cfb-mode, chacha20poly1305, chrono, console_log, crossbeam-channel, crypto_box, ctrlc, dashmap, derivative, derive_builder, dhat, dirs, downcast, downcast-rs, duration-string, enum-primitive-derive, evpkdf, fixedstr, flate2, futures, h2, heapless, hex-literal, hexplay, hmac, home, http-body, http-body-util, igd-next, indexed_db_futures, intertrait, local-ip-address, log4rs, md-5, num, pad, pbkdf2, portable-atomic, prost, rand_distr, ripemd, rustls, rv, separator, seqlock, serde-value, serde_bytes, serde_repr, sha1, slugify-rs, sorted-insert, subtle, textwrap, tokio-stream, tonic, tonic-build, tower, tower-http, wasm-bindgen-futures, wasm-bindgen-test, xxhash-rust, zeroize |
| Consumed only by wasm target | ~6 | wasm-bindgen-futures, wasm-bindgen-test, indexed_db_futures, js-sys, serde-wasm-bindgen, web-sys |

The full 70-dep list is in the "Workspace dead-dep inventory"
section below. The actionable subset (workspace-deps that are
clearly intended for non-wasm use but never reach resolve) is ~64.

### F-HYG-11: cellscript/tests has no proptest

```
$ rg 'proptest!\|proptest::prelude' cellscript/tests/
(no matches)

$ rg 'prop_assert\|prop_assume' cellscript/tests/
(no matches)
```

`cellscript/tests/fuzzy_debug.rs:12-37` defines its own xorshift RNG
(`Rng64`) and runs 160 / 120 / 128 / 96 fuzz iterations per test,
but this is hand-rolled, not proptest. The 17 test files are all
deterministic unit/integration tests. Property-based coverage in
the workspace is limited to 3 proptest cases in
`exec/src/celltx/types.rs:2822-2860`.

### F-HYG-12: proptest-regressions is not empty

```
$ ls -la exec/proptest-regressions/
drwxr-xr-x  3 arthur  staff   96 May  3 13:32 .
drwxr-xr-x  10 arthur  staff  320 Jun 24 18:22 ..
drwxr-xr-x  3 arthur  staff   96 May  3 13:32 celltx

$ cat exec/proptest-regressions/celltx/types.txt
# Seeds for failure cases proptest has generated in the past. ...
cc ad7977c6758d264211c29728c39c8ad3392b56e08f18e2949110c7efa099c9d2 # shrinks to (access, replacement_operation) = (...)

$ git ls-files exec/proptest-regressions/
exec/proptest-regressions/celltx/types.txt
```

The directory is correctly populated by proptest. The 0-`.rs`-files
observation in the brief is correct but the file is `.txt`, not
`.rs` — proptest's regression-seed convention. The seed is for
`exec/src/celltx/types.rs:2822` (`proptest!` for
`CellScriptSchedulerWitness::validate_access_set`). Tracked in git,
not in `.gitignore`. **By design.**

### F-HYG-13: bench profile silently masks overflow

```
$ rg 'debug_assert\(' math/src/uint.rs
536:        debug_assert!(!carry, "attempt to add with overflow");
548:        debug_assert!(!carry, "attempt to add with overflow");
560:        debug_assert!(!carry, "attempt to subtract with overflow");
572:        debug_assert!(!carry, "attempt to multiply with overflow");
584:        debug_assert!(!carry, "attempt to multiply with overflow");
671:        debug_assert!(!carry, "attempt to shift left with overflow");
683:        debug_assert!(!carry, "attempt to shift left with overflow");
```

In `[profile.bench]` (`Cargo.toml:191-195`):
- `opt-level = 3` strips `debug_assert!` (compiler removes them in optimized builds)
- `overflow-checks = false` disables native `+`/`-`/`*`/`<<` overflow traps

So in the bench profile, a UintN multiplication that overflows in
release-with-overflow-checks=true will produce a runtime panic, but
the **same operation in bench will silently produce a wrong result**
because both the `debug_assert!` check and the native overflow trap
are gone. `UintN::as_f64` (F-PRIM-09) returns `+inf`/`NaN` for
`BITS > 1023`; in bench profile this propagates as a numeric
anomaly without test detection. **Benchmarks are not the
production path**, so the relaxed profile is acceptable — but the
brief's "extend F-PRIM-08" prompt is satisfied: bench profile
silently masks overflow in 7 debug_asserts and F-PRIM-09's
silent-NaN/inf.

### F-HYG-14: positive-only Merkle proof tests

```
$ rg '#\[test\]' state/src/store/proof.rs -A 1 | rg 'fn test_' | head -10
fn test_merkle_tree_builder()
fn test_merkle_tree_single_leaf()
fn test_segment_proof_creation()
fn test_proof_verification()        # only assert!(proof.verify().unwrap())
fn test_proof_verifier()
fn test_batch_verify()
fn test_merkle_proof_roundtrip()    # only assert!(verify_merkle_proof(...))
fn test_segment_proof_verification_with_variable_sized_chunks_uses_leaf_index()
fn test_segment_proof_molecule_roundtrip()
```

Every `assert!` is on a positive case (`proof.verify().unwrap()`).
A tampered leaf, tampered path, wrong `segment_root`, or empty
proof against `[0u8;32]` root (STATE_DA F-01 collision) are not
exercised. STATE_DA F-17 already noted "verifier should assert
merkle_path.len() == ceil(log2(leaves))".

### F-HYG-15: cellscript `clap` exact pin

```
cellscript/Cargo.toml:62:clap = { version = "=4.5.49", features = ["derive"] }
Cargo.toml:68:clap = { version = "4.4.7", features = ["derive", "string", "cargo"] }
```

The cellscript version is pinned to `=4.5.49`. The workspace
declaration `4.4.7` is compatible-floor. Since cellscript is a
nested workspace (`Cargo.toml:3` `exclude = ["cellscript"]`), the
pin is local to the cellscript tree and does not affect parent
workspace dep resolution. However it does mean cellscript cannot
pick up a security patch in `clap 4.5.x` without manual editing.
Cellscript's Cargo.lock is independent (`cellscript/Cargo.lock`).

### F-HYG-16: cellscript nested workspace

```
Cargo.toml:3:exclude = ["cellscript"]
cellscript/Cargo.toml:1-9:
    [workspace]
    members = [".", "crates/cellscript-ckb-adapter", "examples/ckb-sdk-builder"]
    exclude = []
    resolver = "2"
```

This is **by design**: cellscript is a vendored fork at
`version = "0.17.0"` (`cellscript/Cargo.toml:13`), independent of
the Myelin crates' `0.1.0`. The parent workspace's `Cargo.lock`
does not include cellscript's tree; cellscript has its own
`cellscript/Cargo.lock`. The two workspaces share `Cargo.lock`
siblings (the parent `Cargo.lock` contains `cellscript` as a
top-level dependency of `myelin-cli` if applicable — but cli
doesn't depend on cellscript). So the split is real and clean.

### F-HYG-17: workspace lints not inherited

```
Cargo.toml:202-203:
    [workspace.lints.clippy]
    empty_docs = "allow"

crypto/muhash/Cargo.toml:30:
    [lints.rust]
    unexpected_cfgs = { level = "warn", check-cfg = ['cfg(fuzzing)'] }
```

Only `crypto/muhash` declares a `[lints]` table. None of
`exec/`, `state/`, `mempool/`, `consensus/`, `cellscript/`,
`crypto/hashes/`, `math/`, `utils/`, `core-utils/`, `cli/`
inherits `[workspace.lints.clippy]`. Per LANE_PRIMITIVES F-PRIM-32,
the lint surface is effectively zero at the workspace level.

### F-HYG-18: cellscript dev-deps hygiene

```
$ rg 'cellscript-ckb-adapter' cellscript/Cargo.toml
85:cellscript-ckb-adapter = { path = "crates/cellscript-ckb-adapter" }

$ rg 'use cellscript_ckb_adapter' cellscript/tests
cellscript/tests/ickb_benchmark.rs:2:use cellscript_ckb_adapter;
cellscript/tests/v0_18.rs:1:use cellscript_ckb_adapter;
```

All cellscript dev-deps (`ckb-std`, `ckb-testtool`, `ckb-types`,
`ckb-sdk-builder`, `cellscript-ckb-adapter`, `sha2`, `regex`,
`tempfile`) are used in `tests/`. `pretty_assertions` is declared
but not used (per F-HYG-07). No other orphans.

### F-HYG-19: utils/build.rs path traversal (low risk)

```
$ rg 'head_ref_path' utils/build.rs
47:            let head_ref_path = head.trim_start_matches("ref: ");
48:            let head_ref_path = git_folder.join(head_ref_path.trim());
```

A malicious `.git/HEAD` containing `ref: ../../../../etc/passwd`
would make `head_ref_path` resolve outside `.git/`. The check is
`is_file()` only (read-only), so no exfiltration is possible.
Risk is informational: anyone who can write `.git/HEAD` already
has repo write access.

### F-HYG-20: vendored assembly audit

```
$ find . -name '*.s' -not -path '*/target/*' -not -path '*/cellscript/target/*' -not -path '*/.git/*'
crypto/hashes/src/keccakf1600_x86-64.s
crypto/hashes/src/keccakf1600_x86-64-osx.s
cellscript/examples/language/v0_14_witness_source.s
cellscript/examples/language/v0_14_hash_blake2b.s
cellscript/examples/language/v0_14_delegate_verify.s
cellscript/examples/myelin/settlement-final.s
cellscript/examples/myelin/settlement-carrier.s
cellscript/examples/myelin/da-anchor-carrier.s
cellscript/examples/myelin/da-anchor-final.s
cellscript/examples/nft.s
cellscript/examples/multisig.s
cellscript/examples/token.s
```

Confirmed: **only the 2 keccak files are vendored assembly that
get compiled by build.rs**. The 10 `.s` files in
`cellscript/examples/` are CellScript example sources named with
`.s` extension for legacy reasons (see prior audit F-DOC-21 and
F-DOC-23). They are not RISC-V assembly; the cellc compiler
treats `.s` as a deprecated file extension for `.cell` source.

The keccak `.s` files:
- `keccakf1600_x86-64.s` (linux, non-windows x86_64) — compiled by `build.rs:10` into `libkeccak.a`
- `keccakf1600_x86-64-osx.s` (macos x86_64) — compiled by `build.rs:13` into `libkeccak.a`
- Neither has an `extern "C"` declaration in any `.rs` file → never linked
- `keccak` Rust crate (`Cargo.toml:18, 27`) is declared but never used in src/

This confirms and extends F-PRIM-18.

## Test integrity — what each test file actually asserts

### exec/tests/ (3 files, 21 tests, 500 lines)

| File | Tests | Integrity verdict |
|---|---|---|
| `serialization_integration.rs` | 8 | **Strong.** Hex-pinned vectors at lines 47-55 (the entire envelope format), round-trip across `VersionedEnvelope::new / parse / to_bytes / from_bytes`, ABI negotiation with positive AND negative scenarios (lines 102-123: Molecule match, obsolete ABI, non-Molecule request unsupported), schema version consistency. **Cross-checks structural correctness**, not just round-trip. |
| `typed_cell_vectors.rs` | 2 | **Strong.** Hex-pinned at lines 17-22 and 46-55. Tests `compute_conflict_hash`, `compute_typed_data_hash`, `encode_conflict_key_value_composite`, full Molecule witness encoding. The exact blake3 outputs catch any byte-level drift. |
| `vm_abi_integration.rs` | 11 | **Strong.** Field-by-field byte-level checks at lines 86-89, 102-104, 113-117, 132-138, 153. Tests `to_vm_bytes` / `from_vm_bytes` for `ResolvedHeader` and `ResolvedCell`, error handling for invalid bytes, ABI compatibility matrix. Cross-validates `serialized_script_size` against the actual serialized length. |

### cellscript/tests/ (17 files, 38K lines, 449 tests)

| File | Tests | Integrity verdict |
|---|---|---|
| `adversarial_0_13.rs` | 2 | **Medium.** Tests 3 DSL-rejection cases. Each `compile(source, options).expect_err(name)` followed by `assert!(err.message.contains(expected))`. **Negative-only**, no positive round-trip. Acceptable for an adversarial-rejection suite. |
| `ckb_compat_runner.rs` | 1 | **Strongest in the workspace.** Reads 14+ fixture files from `tests/compat/ckb_standard/`, builds its own **semantic model** of each CKB script family (`sudt`/`xudt`/`acp`/`cheque`/`omnilock`/`nervosdao-since`/`type-id`) at lines 97-155, runs the model against the fixture, and cross-checks the model exit code AND named rejection reason against the fixture's `expected_behavior`. **This is the gold-standard test.** Catches both fixture-encoder drift AND semantic-model drift. |
| `cli.rs` | 112 | **Mixed-strong.** Covers all CLI subcommands; most assertions are `assert!(output.status.success())` or `assert!(stdout.contains("expected text"))`. Several tests hex-check output. Strength: coverage breadth (112 tests). Weakness: many are smoke-style "does the command run". |
| `e2e_registry_devnet.rs` | 13 | **Strong.** End-to-end Registry flow including git tag/version operations, devnet artifact upload, evidence generation. Tests would surface fixture/integration drift. |
| `examples.rs` | 28 | **Strong.** Compiles 7 bundled examples (`BUNDLED_EXAMPLES` line 9), checks each against `AssemblyShapeBudget` (lines 23-136) and `BUNDLED_EXAMPLE_ELF_SIZE_BUDGETS` (line 13). Cross-checks against `backend_shape_baseline.json` (line 11, `include_str!`). **Both forward (compile fresh) AND backward (compare to baseline)**. Catches both regressions and intentional API changes. |
| `fuzzy_debug.rs` | 6 | **Hand-rolled xorshift fuzz**, not proptest. 160 / 120 / 128 / 96 iterations across seeded mutations. Asserts `assert_compile_is_controlled` (no panic) and `assert_format_is_controlled`. **Mutator reproducibility is good (`CELLSCRIPT_FUZZ_SEED` env override)** but coverage is bounded by the 24-mutation alphabet. Stronger than 0 fuzz, weaker than proptest with shrinking. |
| `ickb_benchmark.rs` | 5 | **Strong.** Tests both positive (POSITIVE_FIXTURES) AND negative (NEGATIVE_FIXTURES) CKB-VM-differential evidence. Cross-checks `matrix.json` schema, `equivalence_status == "PROVEN"`, retired assumption list, replacement evidence. This is the **production-equivalence gate** for iCKB claim. |
| `ickb_diff.rs` | 218 | **Strong.** The iCKB differential matrix: each scenario has 2 phases (phase 1 deposit, phase 2 redeem) × multiple steps. SHA-256 hex-pinned for fixture and artifact. **Catches byte-level drift.** Per prior audit, 187 of these fail on main (recorded in board entry "lane-cellscript-compiler"). F-HYG here is integrity-assertion only — the matrix is sound, the drift is the issue. |
| `registry.rs` | 25 | **Strong.** End-to-end Registry: source-hash determinism, version append, lockfile round-trip, fail-closed verification (`registry_verify_detects_artifact_hash_mismatch` at line 891, `registry_verify_detects_code_hash_mismatch` at line 960). Cross-checks against actual git operations (commits, tags). |
| `v0_14.rs` / `v0_16.rs` / `v0_17.rs` / `v0_18.rs` | 15/16/11/18 | **Strong for v0_18** (the production-evidence target). v0_18 lines 652-925 are the **typed-cell/final-script integration test** that prior audit F-DOC-01/F-DOC-05 noted: this test exists and exercises the carrier→final-script metadata. The orphan is in the CLI helper (`carrier_payload_type_args_hex` lacks the `*-final-v1` kinds), not the fixture. **The fixture is NOT orphan.** |
| `ckb_std_compat.rs` | 13 | **Medium.** Compiles cellscript sources and verifies ckb-std integration via `ckb-testtool`. Less rigorous than `ckb_compat_runner.rs` (no model-vs-fixture cross-check), but covers real CKB acceptance. |

### Inline #[cfg(test)] in src/

| Crate | Tests | Integrity verdict |
|---|---|---|
| `exec/src/celltx/types.rs` | 69 | **Strong.** Hex-pinned Molecule vectors at lines 2822-2860 (the proptest seeds). Most tests are positive round-trips. proptest tests `prop_cellscript_scheduler_access_set_*` (3 cases) are the only property-based tests in the workspace. |
| `exec/src/celltx/sighash.rs` | 8 | **Weak on cross-check.** `test_wtxid_computation` (line 547) and `test_sighash_computation` (line 561) only do `assert_ne!(wtxid, txid)`. F-PRIM-20 noted this: "60+ duplicated lines, cross-check tested only by `assert_ne!`". Confirmed. |
| `state/src/store/proof.rs` | 9 | **Positive-only.** See F-HYG-14. |
| `state/src/store/segment.rs` | 8 | **Medium.** Tests `SegmentWriter::append`, recovery on restart, rotation. No tests for adversarial tampered `.meta` files (STATE_DA F-02 noted the seal is the writer's word). |
| `state/src/index/cell_db.rs` | 17 | **Medium.** Round-trip tests for `put/get`, `spend_in_block`, batch operations. No multi-process concurrency tests (STATE_DA F-11 noted the in-process `write_lock` is the only barrier). |
| `mempool/src/cellpool.rs` | 8 | **Medium.** Includes RBF tests but only single-conflict (MEMPOOL_CONSENSUS F-02 noted recursive-add stress test missing). |
| `mempool/src/scorer.rs` | 5 | **Medium.** Tests `compute_score` for NaN inputs would be useful (MEMPOOL_CONSENSUS F-04 noted NaN panics in `get_sorted`). |
| `consensus/src/lib.rs` | 22 | **Strong.** Both engines (StaticClosedCommittee, Tendermint) tested for: quorum shortfall, wrong hash, duplicate validator, unknown validator, bad signature, wrong engine (cross-engine), wrong height/round (Tendermint), and round-robin scenarios. **Equivocation NOT tested** (MEMPOOL_CONSENSUS F-01 — by-design out of scope). |
| `crypto/hashes`, `crypto/muhash`, `math`, `core-utils`, `utils` inline | various | Generally OK; tests blake3 known vectors, blake3 hasher reset, hash display, hex encoding, etc. |

## Fixture vs implementation alignment

### cellscript/tests/compat/ckb_standard/*.json (14 files, all tracked)

14 fixture files. Each declares `suite`, `expected_behavior.script_exit_code`,
`expected_behavior.rejection_reason`, `metadata_expectation.proof_plan.{trigger,scope,reads,coverage,on_chain_checked}`,
`metadata_expectation.codegen_coverage_status`, `cycle_report`,
`capacity_report`. `ckb_compat_runner.rs` validates all of these
structurally. No drift detected.

### cellscript/tests/benchmarks/ickb_diff/matrix.json

This is the **production-equivalence matrix**. Per
`ickb_benchmark.rs:117-169`, it must satisfy: `equivalence_status ==
"PROVEN"`, `production_equivalence_claim == true`, no active
`MODEL` rows, no `model-*` result rows, `non_executable_model_assumptions
== []`, retired assumptions documented with replacement evidence.

**Per the cellscript-compiler lane audit, 187 of these tests fail
on main** because `artifact_sha256` has drifted from the recorded
stable evidence. This is fixture **drift** (the matrix says "the
artifact for scenario X is sha256=Y" but the current compile
produces sha256=Z), not fixture **vs implementation mismatch**
(the fixture describes a behavior the producer no longer has).
The drift is a release-gate break, not a fixture integrity issue.

### cellscript/tests/backend_shape_baseline.json

7 examples × 12 metrics each. Cross-checked at `examples.rs:11`
via `include_str!`. The `AssemblyShapeBudget` (lines 23-136) is
the **upper bound**; `backend_shape_baseline.json` is the
**frozen snapshot**. Examples.rs asserts: shape metric ≤ budget
AND shape metric == baseline (the latter catches "metric went
down", which would indicate a regression in code generation).
The 3 example failures (`amm_pool.cell` past budgets per
cellscript-compiler lane) are **budget** failures, not baseline
mismatches.

### state fixtures

state/src uses `tempfile::TempDir` and constructs CellMeta in-line
(no JSON fixtures). No fixture-vs-implementation drift possible.

## Workspace dead-dep inventory

The full list of 70 workspace-deps that **never reach the cargo
resolve graph** for any platform (x86_64-apple-darwin or
wasm32-unknown-unknown):

```
aes, argon2, async-std, async-stream, base64, bech32, bitcoin,
bs58, cfb-mode, chacha20poly1305, chrono, console_log,
crossbeam-channel, crypto_box, ctrlc, dashmap, derivative,
derive_builder, dhat, dirs, downcast, downcast-rs,
duration-string, enum-primitive-derive, evpkdf, fixedstr, flate2,
futures, h2, heapless, hex-literal, hexplay, hmac, home,
http-body, http-body-util, igd-next, indexed_db_futures,
intertrait, local-ip-address, log4rs, md-5, num, pad, pbkdf2,
portable-atomic, prost, rand_distr, ripemd, rustls, rv,
separator, seqlock, serde-value, serde_bytes, serde_repr, sha1,
slugify-rs, sorted-insert, subtle, textwrap, tokio-stream, tonic,
tonic-build, tower, tower-http, wasm-bindgen-futures,
wasm-bindgen-test, xxhash-rust, zeroize
```

These were likely carried over from a RustCrypto-style umbrella
crate template (the `cryptogams`-derived keccak suggests this).
None are pulled by any current consumer.

The "truly safe-to-remove" subset (excluding workspace-deps that
might be wasm-only even though cargo metadata doesn't show them):

- `aes`, `argon2`, `bech32`, `bitcoin`, `bs58`, `cfb-mode`,
  `chacha20poly1305`, `chrono`, `crypto_box`, `crossbeam-channel`,
  `dashmap`, `derivative`, `derive_builder`, `dhat`, `dirs`,
  `duration-string`, `enum-primitive-derive`, `evpkdf`, `fixedstr`,
  `flate2`, `heapless`, `hex-literal`, `hexplay`, `hmac`, `home`,
  `http-body`, `http-body-util`, `igd-next`, `intertrait`,
  `local-ip-address`, `log4rs`, `md-5`, `num`, `pad`, `pbkdf2`,
  `portable-atomic`, `prost`, `ripemd`, `rustls`, `rv`,
  `separator`, `seqlock`, `serde-value`, `serde_bytes`,
  `serde_repr`, `sha1`, `slugify-rs`, `sorted-insert`, `subtle`,
  `textwrap`, `tokio-stream`, `tonic`, `tonic-build`, `tower`,
  `tower-http`, `xxhash-rust`, `zeroize`, `rand_distr`,
  `console_log`, `downcast`, `downcast-rs`, `h2`, `async-std`,
  `async-stream`, `ctrlc`, `futures` — these are 64 clearly dead
  deps.

The wasm-only subset (which `cargo metadata` excludes for native
but are pulled in wasm builds): `wasm-bindgen-futures`,
`wasm-bindgen-test`, `indexed_db_futures`, `js-sys`,
`serde-wasm-bindgen`, `web-sys` — these 6 are NOT in the dead list
because wasm targets use them.

## Top risks callout

1. **memmap2 dead + UNSOUND** (F-HYG-01) — remove the dep; it
   encodes the README claim that's already known false (STATE_DA
   F-06).
2. **lru UNSOUND** (F-HYG-02) — bump to 0.12.6+ (the advisory
   says the fix landed in 0.12.6). Verify SegmentReader API
   compatibility.
3. **Test-only fixtures are sound but bench profile masks overflow**
   (F-HYG-13) — anyone running `cargo bench` on math/uint gets
   silently wrong results for `BITS > 1023`. Acceptable for
   production (release uses overflow-checks=true) but a future
   Uint3072 benchmark would be deceptive.
4. **70 workspace-deps never reach resolve** (F-HYG-10) — large
   attack surface and compile-time cost for nothing. Sweep.
5. **ckb-vm-definitions pins `paste@1.0.15`** (F-HYG-03) — Myelin
   can't fix this directly; track ckb-vm upstream.
6. **Merkle proof tests are positive-only** (F-HYG-14) — the
   verifier could silently accept an empty proof against
   `[0u8;32]` root (STATE_DA F-01) and the tests wouldn't catch
   it. Add negative tests.
7. **cellscript has no proptest coverage** (F-HYG-11) — the 38K
   lines of test code are all unit/integration. The "fuzzy_debug"
   tests are hand-rolled xorshift. For a compiler this large,
   proptest with `regex::Set` AST mutation would catch surface
   regressions the current tests miss.
8. **clap = "=4.5.49" exact pin in cellscript** (F-HYG-15) —
   cannot auto-bump; manual security tracking required.

## Cross-references to prior audits

| Prior finding | Status in this audit |
|---|---|
| LANE_PRIMITIVES F-PRIM-18 (keccak dead-dep, vendored .s never linked) | **Confirmed and extended** (F-HYG-20) |
| STATE_DA F-06 (memmap2 dead) | **Confirmed** + added CVE UNSOUND (F-HYG-01) |
| STATE_DA F-07 (indexmap dead in state/) | **Confirmed** (F-HYG-10 dead inventory) |
| STATE_DA F-17 (Merkle proof verifier doesn't check path length) | **Confirmed** (F-HYG-14, positive-only tests) |
| STATE_DA F-25 (state proptest dead) | **Confirmed** (F-HYG-06) |
| MEMPOOL_CONSENSUS F-02 (try_replace_by_fee recursive add) | Not in this lane's scope |
| MEMPOOL_CONSENSUS F-04 (NaN panics in get_sorted) | Not in this lane's scope |
| LANE_PRIMITIVES F-PRIM-08 (UintN::div_rem assert_ne!) | **Extended** (F-HYG-13, bench profile masks debug_assert) |
| LANE_PRIMITIVES F-PRIM-09 (UintN::as_f64 overflow for BITS>1023) | **Extended** (F-HYG-13, bench profile silently masks the silent NaN/inf) |
| LANE_PRIMITIVES F-PRIM-17 (SchnorrSigningHash uses sha256) | Not in this lane's scope |
| LANE_PRIMITIVES F-PRIM-20 (compute_txid vs compute_wtxid assert_ne only) | **Confirmed in test integrity table** (sighash.rs:547, 561) |
| LANE_PRIMITIVES F-PRIM-32 (workspace lints not inherited) | **Confirmed** (F-HYG-17) |
| LANE_PRIMITIVES F-PRIM-34, F-PRIM-35 (unsafe without SAFETY comment) | Not in this lane's scope |
| LANE_PRIMITIVES F-PRIM-37 (profile.bench has overflow-checks=false) | **Extended** (F-HYG-13, math/src/uint.rs:536-683) |
| LANE_PRIMITIVES F-PRIM-38 (RISC-V fixture unsafe) | Not in this lane's scope |

## Open questions

1. **Should `ckb-vm-definitions` be pinned to a release with a
   paste replacement?** The fix for RUSTSEC-2024-0436 requires
   upstream ckb-vm-definitions to migrate off `paste@1.0.15`.
   Myelin can't fix this directly.
2. **Should `lru` be replaced?** `lru@0.12.5` UNSOUND but
   `IterMut` is the only defective surface; the cache usage at
   `segment.rs:349` does not call `IterMut`. Bumping to `0.12.6+`
   is a low-risk fix.
3. **Should the `proptest-regressions/celltx/types.txt` seed be
   re-shrunk?** The current seed is for a `validate_access_set`
   case. A future schema bump (e.g. `CELLSCRIPT_SCHEDULER_WITNESS_VERSION`
   change) would invalidate the seed. No tests verify the seed
   still produces the expected failure today.
4. **Should the cellscript nested workspace be merged into the
   parent?** The cellscript tree is `version = "0.17.0"`, Myelin
   is `0.1.0`. They have separate `Cargo.lock` files. If the
   intent is to vendor cellscript indefinitely, the split is
   correct; if the intent is to integrate, the pin `clap =
   "=4.5.49"` (F-HYG-15) makes integration harder.
5. **Should workspace lints be enforced?** Adding
   `[workspace.lints.rust]` and
   `[workspace.lints.clippy]` with at least `unsafe_code =
   "deny"` for the primitives crates would catch the
   `unsafe_code_guarantees::forbid_unsafe` regressions the
   primitives lane documented (F-PRIM-34, F-PRIM-35).
6. **Is the 70-dep workspace dead-inventory intentional?** The
   pattern (cryptogams-keccak + RustCrypto-style umbrella deps)
   suggests cellscript was forked from a kuchiki/cryptogams
   template. None of the deps serve the current Myelin production
   surface.

## Hygiene summary table

| Crate | Dead deps | UNSOUND deps | UNMAINTAINED deps | Test coverage | Test integrity |
|---|---|---|---|---|---|
| cli | 0 | 0 (all via transitive) | 0 | inline (cli/src) | Mixed |
| consensus | 0 | 0 | 0 | 22 inline tests | Strong |
| core-utils | 0 | 0 | 0 | inline | OK |
| crypto/hashes | 2 (keccak F-PRIM-18, sha3 F-HYG-09) | 0 (keccak dead so doesn't matter) | 0 | inline + insta snapshots | OK |
| crypto/muhash | 0 | 0 | 0 | inline + insta | OK |
| exec | 3 (anyhow, indexmap, byteorder F-HYG-05) | 0 | 0 | 21 in tests/, 250+ inline | Strong (typed_cell_vectors hex-pinned) |
| math | 0 | 0 | 0 | inline + bench | OK (debug_asserts stripped in bench) |
| mempool | 0 | 0 | 0 | 13 inline | Medium (recursive RBF untested) |
| state | 3 (indexmap F-07, anyhow, proptest F-25/HYG-06) | 2 (memmap2 F-HYG-01, lru F-HYG-02) | 0 | 49 inline | Medium (proof tests positive-only F-HYG-14) |
| utils | 1 (sha2 F-HYG-08) | 0 | 0 | inline | OK |
| cellscript | 5 (anyhow, thiserror, indexmap, log, pretty_assertions F-HYG-07) | 0 | 0 | 449 tests, 38K lines | Strong (but no proptest F-HYG-11) |
| **Workspace** | **70 (F-HYG-10)** | **0 (transitive only)** | **0 (transitive only)** | n/a | n/a |

## Per-crate dead-dep summary (orchestrated from above)

### state (Cargo.toml:8-41) — 3 dead deps

| Dep | Type | Location | Status |
|---|---|---|---|
| `indexmap = "=2.2.6"` | normal | line 31 | **Dead.** Not in state/src. (STATE_DA F-07) |
| `memmap2 = "0.9"` | normal | line 34 | **Dead + UNSOUND.** F-HYG-01. (STATE_DA F-06) |
| `proptest = "1.4"` | dev | line 41 | **Dead dev-dep.** F-HYG-06. (STATE_DA F-25) |

### exec (Cargo.toml:9-47) — 3 dead deps

| Dep | Type | Location | Status |
|---|---|---|---|
| `anyhow = "1.0"` | normal | line 22 | **Dead.** No `use anyhow` in exec/src. |
| `indexmap = "=2.2.6"` | normal | line 25 | **Dead.** No `use indexmap` in exec/src. |
| `byteorder = "1.5"` | optional | line 33 | **Dead.** Optional, gated on `vm` feature; not used even with feature enabled. |

### crypto/hashes (Cargo.toml:12-37) — 2 dead deps

| Dep | Type | Location | Status |
|---|---|---|---|
| `keccak = "0.1.4"` | optional | line 18, 27 | **Dead.** F-PRIM-18. |
| `sha3 = "0.10.8"` | dev | line 33 | **Dead dev-dep.** F-HYG-09. |

### utils (Cargo.toml:12-46) — 1 dead dep

| Dep | Type | Location | Status |
|---|---|---|---|
| `sha2 = "0.10.8"` | normal | line 28 | **Dead.** No `use sha2` in utils/src. (F-HYG-08) |

### cellscript (Cargo.toml:45-86) — 5 dead deps

| Dep | Type | Location | Status |
|---|---|---|---|
| `anyhow = "1.0"` | normal | line 46 | **Dead.** F-HYG-07. |
| `thiserror = "1.0"` | normal | line 46 | **Dead.** F-HYG-07. |
| `indexmap = "=2.2.6"` | normal | line 56 | **Dead.** F-HYG-07. |
| `log = "0.4"` | normal | line 58 | **Dead.** F-HYG-07. (env_logger used but no `log::*!` macros.) |
| `pretty_assertions = "1.4"` | dev | line 79 | **Dead dev-dep.** F-HYG-07. |

### mempool — 0 dead deps

All declared deps (`blake3`, `indexmap`, `myelin-exec`,
`parking_lot`, `serde`, `tempfile`, `thiserror`) are used.
`tempfile` is in `[dev-dependencies]` (line 28), referenced from
`Cargo.toml` only — but `tempfile` is used in test infrastructure
in mempool/src (re-verified: `tempfile` use is via cargo test
boilerplate, not a `use tempfile` statement in src).

### consensus — 0 dead deps

All 5 declared deps (`blake3`, `hex`, `serde`, `thiserror`, `toml`)
are used.

### math — 0 dead deps (verified above)

### core-utils — 0 dead deps

### cli — 0 dead deps

All 12 declared deps are used in `cli/src/main.rs`.