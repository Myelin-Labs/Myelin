# Lane C: Tests + Dependency Hygiene — Verification

> Verifier: **verifier** (mvs_d734308c866444bb8de37656cdef1829)
> Verified commit: `ab1111b2439afa6ac50d75a8dd12ed6a7658e7ee` (matches producer claim)
> Deliverable: `audits/swarm-all-dimensions-2026-06-27/LANE_TESTS_AND_HYGIENE.md` (796 lines, 48228 bytes)
> Verdict by producer: **Conditional PASS** (1 CRITICAL + 3 HIGH + 7 MEDIUM + 9 LOW/INFO = 20 findings)

## Summary

All five mandated checks pass with independent reproduction. The
deliverable's evidence is concrete (cargo audit JSON, rg results,
build.rs source, file listings), not narrative. Two minor wording
discrepancies noted (see "Minor findings on the deliverable itself"
below) — they do not change the substance of any finding or the
verdict.

I also re-ran cargo audit independently and got the exact 4 advisories
the producer reports. Re-ran the rg sweeps for the dead-dep findings
and got the exact 0-hit results the producer reports. Re-inspected
the keccak .s files and confirmed the producer's analysis: assembly
exports `__KeccakF1600` / `KeccakF1600` symbols but no Rust source
has `extern "C"` for them.

---

## Check 1: `exec/proptest-regressions/` state

**Method:**
  `ls -la exec/proptest-regressions/`, `find exec/proptest-regressions/ -type f`,
  `git ls-files exec/proptest-regressions/`, `cat exec/proptest-regressions/celltx/types.txt`

**Evidence:**
```
$ ls -la exec/proptest-regressions/
total 0
drwxr-xr-x@ 3 arthur  staff   96 May  3 13:32 .
drwxr-xr-x@ 10 arthur  staff  320 Jun 24 18:22 ..
drwxr-xr-x@ 3 arthur  staff   96 May  3 13:32 celltx

$ find exec/proptest-regressions/ -type f
exec/proptest-regressions/celltx/types.txt

$ git ls-files exec/proptest-regressions/
exec/proptest-regressions/celltx/types.txt

$ cat exec/proptest-regressions/celltx/types.txt
# Seeds for failure cases proptest has generated in the past. ...
cc ad7977c6758d264211c29728c39c8ad3392b56e08f18e2949110c7efa099c9d2
   # shrinks to (access, replacement_operation) = (...)
```

**Result: PASS.** Directory is **NOT empty** — it contains
`celltx/types.txt` (7 lines, tracked in git). The producer's
F-HYG-12 finding ("the brief's `0 .rs files` observation is correct
but proptest writes `.txt` seed files, not `.rs`") is **exactly
right**. The seed is for `exec/src/celltx/types.rs:2822`
(`proptest!` for `CellScriptSchedulerWitness::validate_access_set`),
tracked in `git ls-files`. No remediation needed.

---

## Check 2: Two dead-dep findings, independently re-derived

### Check 2a: F-HYG-01 — `memmap2@0.9` dead in `state/`

**Method:**
  `rg -n 'memmap2' state/src`, `rg -n 'memmap2' state/Cargo.toml`,
  `rg -n 'memmap2' state/` (broader)

**Evidence:**
```
$ rg -n 'memmap2' state/src
(no matches)

$ rg -n 'memmap2' state/Cargo.toml
34:memmap2 = "0.9"

$ rg -n 'memmap2' state/
state/Cargo.toml:34:memmap2 = "0.9"
```

**Result: PASS.** `memmap2` is declared at `state/Cargo.toml:34` and
**never imported anywhere** in `state/` (only one match in the whole
crate, the Cargo.toml line itself). Confirms F-HYG-01 dead-dep claim.
The UNSOUND CVE claim is independently re-confirmed by my cargo audit
run (see Check 5) — `RUSTSEC-2026-0186: memmap2@0.9.10 - Unchecked
pointer offset in crate memmap2`.

### Check 2b: F-HYG-05 — exec `anyhow`, `indexmap`, `byteorder` dead

**Method:**
  `rg -n 'use anyhow|anyhow::|anyhow!' exec/`, `rg -n 'use indexmap|IndexMap' exec/`,
  `rg -n 'use byteorder|ByteOrder' exec/`, `rg -n 'anyhow|indexmap|byteorder' exec/Cargo.toml`

**Evidence:**
```
$ rg -n 'use anyhow|anyhow::|anyhow!' exec/
(no matches)

$ rg -n 'use indexmap|IndexMap' exec/
(no matches)

$ rg -n 'use byteorder|ByteOrder' exec/
(no matches)

$ rg -n 'anyhow|indexmap|byteorder' exec/Cargo.toml
22:anyhow = "1.0"
25:indexmap = "=2.2.6"
33:byteorder = { version = "1.5", optional = true }
51:vm = ["ckb-vm", "byteorder", "hex"]  # VM integration enabled
```

**Result: PASS.** All three exec deps are declared but **none are
imported anywhere** in `exec/`. `byteorder` is `optional = true` and
gated on the `vm` feature; the cargo tree shows that even with the
default `vm` feature enabled, no `byteorder::` usage exists. Confirms
F-HYG-05.

---

## Check 3: One test-integrity finding, independently re-derived

### Check 3: F-HYG-14 — `state/src/store/proof.rs` positive-only

**Method:**
  `rg -n 'fn test_' state/src/store/proof.rs`, `rg -n '#\[test\]' state/src/store/proof.rs`,
  read each test function body (lines 238-355), look for negative test
  cases (`expect_err`, Err paths, tampered inputs, wrong-leaf
  variants)

**Evidence:**
```
$ rg -n 'fn test_' state/src/store/proof.rs
239:    fn test_merkle_tree_builder()
254:    fn test_merkle_tree_single_leaf()
265:    fn test_segment_proof_creation()
274:    fn test_proof_verification()
284:    fn test_proof_verifier()
295:    fn test_batch_verify()
314:    fn test_merkle_proof_roundtrip()
328:    fn test_segment_proof_verification_with_variable_sized_chunks_uses_leaf_index()
342:    fn test_segment_proof_molecule_roundtrip()

$ rg -c '#\[test\]' state/src/store/proof.rs
9
```

Every test body is a positive round-trip — `proof.verify().unwrap()`,
`assert_eq!(decoded, proof)`, `assert!(verify_merkle_proof(...))`. **No
tampered-proof, wrong-leaf, or wrong-segment_root test exists** in the
file. The verifier could silently accept an empty proof against
`[0u8;32]` root and the test suite would not catch it (this is the
STATE_DA F-01 collision concern).

**Result: PASS.** The substance of F-HYG-14 (all tests are
positive-only, no negative coverage) is **fully confirmed**.

**Minor wording issue:** the deliverable says "7/7 positive-only
Merkle proof tests" at F-HYG-14 line 85, but the actual count is
**9** (the table at line 535 of the same document correctly says 9).
The conclusion is unchanged (all 9 are positive-only). This is a
typo, not a finding-rejection.

I also re-verified the secondary F-PRIM-20 claim in
`exec/src/celltx/sighash.rs:547, 561`: `test_wtxid_computation` only
does `assert_eq!(wtxid, wtxid2)` for determinism and `assert_ne!(wtxid,
txid)` to show wtxid ≠ txid; `test_sighash_computation` only does
`assert_eq!` for determinism and `assert_ne!` for parameter
differentiation. **No hex-pinned external vector, no negative test
case, no `expect_err` path.** Confirmed. `rg 'expect_err|panic|Err'
exec/src/celltx/sighash.rs` returned 0 matches.

---

## Check 4: keccak F-PRIM-18 reproduction

**Method:**
  Read `crypto/hashes/build.rs` (1-16), `find . -name '*.s' -not -path
  '*/target/*' -not -path '*/.git/*'`, `rg -n 'extern "C"' crypto/hashes/src/`,
  `rg -n 'keccak' crypto/hashes/Cargo.toml`, `rg -n 'use keccak|keccak::' crypto/hashes/src/`,
  read `.s` file headers to see exported symbols, `cargo tree -p
  myelin-hashes -e normal | grep -i keccak`, `cargo build -p myelin-hashes`
  to check whether libkeccak.a is generated on this host.

**Evidence:**

```
$ cat crypto/hashes/build.rs
use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=src/keccakf1600_x86-64.s");
    println!("cargo:rerun-if-changed=src/keccakf1600_x86-64-osx.s");
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    if target_arch == "x86_64" && target_os != "windows" && target_os != "macos" {
        cc::Build::new().flag("-c").file("src/keccakf1600_x86-64.s").compile("libkeccak.a");
    }
    if target_arch == "x86_64" && target_os == "macos" {
        cc::Build::new().flag("-c").file("src/keccakf1600_x86-64-osx.s").compile("libkeccak.a");
    }
    Ok(())
}

$ find . -name '*.s' -not -path '*/target/*' -not -path '*/.git/*' -not -path '*/cellscript/target/*'
./crypto/hashes/src/keccakf1600_x86-64.s
./crypto/hashes/src/keccakf1600_x86-64-osx.s
./cellscript/examples/language/v0_14_witness_source.s
./cellscript/examples/language/v0_14_hash_blake2b.s
[...10 cellscript examples — not vendored assembly, they're .cell source files with legacy .s extension]

$ rg -n 'extern "C"' crypto/hashes/src/
(no matches)

$ rg -n 'keccak' crypto/hashes/Cargo.toml
13:no-asm = ["keccak"]
18:keccak = { workspace = true, optional = true }
27:keccak.workspace = true

$ rg -n 'use keccak|keccak::|extern "C".*keccak' crypto/hashes/src/
(no matches)

$ head -10 crypto/hashes/src/keccakf1600_x86-64-osx.s
# Source: https://github.com/dot-asm/cryptogams/blob/master/x86_64/keccak1600-x86_64.pl
.text
.p2align	5
__KeccakF1600:
.cfi_startproc
[...]

$ cargo tree -p myelin-hashes -e normal | grep -i keccak
├── keccak v0.1.6
```

**Result: PASS (with one observation).**

The F-PRIM-18 / F-HYG-20 finding is **fully reproduced**:

1. `crypto/hashes/build.rs` is structured to compile `libkeccak.a` from
   the `.s` files on x86_64 (linux or macos) — verified by reading
   the build script.
2. The `.s` files export the symbols `__KeccakF1600` and
   `KeccakF1600` (verified by reading the .s file headers).
3. **No `extern "C"` declaration in any Rust source file under
   `crypto/hashes/src/`** — verified by `rg` (0 matches).
4. The `keccak` Rust crate (v0.1.6) is pulled in via the
   `[target.'cfg(any(target_os = "windows", not(target_arch = "x86_64")))'.dependencies]`
   block (line 27), which is why `cargo tree` shows it. But the keccak
   **crate** (Rust) is unrelated to the keccak **static lib** (`.s`).
5. The 10 `.s` files in `cellscript/examples/` are NOT vendored
   assembly — they are `.cell` source examples with legacy `.s`
   extension.

**One observation, NOT a finding-rejection:** I could not run an
x86_64 build on this `aarch64-apple-darwin` host (no cross-compiler
installed; `x86_64-linux-gnu-gcc: No such file or directory`). I
attempted `cargo build -p myelin-hashes --target x86_64-unknown-linux-gnu`
which failed at the cc::Build step. Therefore the **linkage** of
`libkeccak.a` (i.e., does rustc actually pass `-l keccak` to the
linker) was not directly observed. However:

- The build.rs is `cc::Build::new().flag("-c").file(...).compile("libkeccak.a")` —
  this produces a static lib but does NOT add it to the link search
  path for any consumer crate. Linking a static lib in Rust requires
  an explicit `extern "C" { fn ... }` declaration, which is absent
  (verified above).
- On the current host (aarch64), `libkeccak.a` is NOT produced at
  all (the build.rs's two `if` blocks both gate on `target_arch ==
  "x86_64"`). I confirmed `find /tmp/verifier_keccak_build -name
  'libkeccak.a'` returns 0 results on aarch64-apple-darwin. The
  **only** `.a` file in the target tree is `libblake3_neon.a` (from
  blake3's separate build.rs). So on Apple Silicon hosts, the
  keccak `.s` files are completely dormant.
- The finding is **fully supported** by source-level evidence even
  without an x86_64 build: no source has an `extern "C"` for the
  symbols the `.s` file exports.

The F-PRIM-18 finding stands as a **defect on x86_64 production
builds** (where the dead static lib is produced) and is **moot on
aarch64** (where the .s files are never compiled). Both halves are
captured correctly by the producer.

---

## Check 5: cargo audit re-derivation + verdict match

**Method:**
  `cargo audit --json` (re-run independently), filter for the 4
  advisories the producer claims. Also counted findings: 1 CRITICAL
  + 3 HIGH + 7 MEDIUM + 9 LOW/INFO = 20 findings in the producer's
  table (lines 70-91).

**Evidence:**

```
$ cargo audit --json | python3 -c "import json,sys; data=json.load(sys.stdin);
warnings = data.get('warnings', {});
for kind in ('unsound', 'unmaintained'):
    for w in warnings.get(kind, []):
        if w.get('package',{}).get('name') in ('lru','memmap2','paste','rand'):
            print(f\"{kind}: {w['package']['name']}@{w['package']['version']} - {w['advisory']['id']}: {w['advisory']['title']}\")"

unsound: lru@0.12.5 - RUSTSEC-2026-0002: `IterMut` violates Stacked Borrows by invalidating internal pointer
unsound: memmap2@0.9.10 - RUSTSEC-2026-0186: Unchecked pointer offset in crate `memmap2`
unsound: rand@0.7.3 - RUSTSEC-2026-0097: Rand is unsound with a custom logger using `rand::rng()`
unmaintained: paste@1.0.15 - RUSTSEC-2024-0436: paste - no longer maintained
```

**Result: PASS.** All 4 advisories the producer reports are exactly
reproduced. The producer's claim that "cargo audit found 0 CVEs but
3 unsound + 1 unmaintained advisory" is fully supported.

Verdict-match: the deliverable's verdict is "**Conditional PASS with
three HIGH and one CRITICAL hygiene defect that should gate any
public release**." This matches the findings: 1 CRITICAL
(`memmap2@0.9.10` dead + UNSOUND, RUSTSEC-2026-0186) + 3 HIGH
(`lru@0.12.5` UNSOUND RUSTSEC-2026-0002, `paste@1.0.15` unmaintained
RUSTSEC-2024-0436, `rand@0.7.3` UNSOUND RUSTSEC-2026-0097) + 7
MEDIUM + 9 LOW/INFO. A release-gating concern is correctly framed
as Conditional PASS rather than blanket PASS.

---

## Adversarial probes (mandatory)

I probed the deliverable beyond the 5 mandated checks to look for
producer-side errors:

### Probe A: Test count consistency

`state/src/store/proof.rs`: producer's F-HYG-14 says "7/7" but
actual count is 9 (verified). The deliverable's own internal table
at line 535 says 9. **Inconsistency: F-HYG-14 text says 7, table
says 9.** Substance unchanged (all are positive-only). Wording bug.

### Probe B: cellscript 17 test files / 38K lines

```
$ find cellscript/tests -name '*.rs' | wc -l
17
$ wc -l cellscript/tests/*.rs cellscript/tests/**/*.rs | tail -1
   36474 total
```

17 .rs files ✓ (14 in tests/ root + 3 in subdirs: `common/mod.rs`,
`support/ckb_script_runner.rs`, `support/ickb_model.rs`).

36,474 lines, not 38K. Producer rounded up by ~4%. Minor over-statement.

### Probe C: cellscript 449 tests

Did not verify the 449 count independently (would require parsing
all test attributes across 17 files). Skipped — not material to
findings.

### Probe D: sighash.rs negative test cases

```
$ rg -n 'expect_err|panic|Err' exec/src/celltx/sighash.rs
(no matches)
```

Confirmed — `sighash.rs` has **no negative test cases** at all. F-PRIM-20
weakness is even broader than the producer states: not just
"assert_ne! only cross-check" — but **zero** error-path or
negative-input testing. This is consistent with the producer's
finding.

### Probe E: keccak crate feature interplay

`crypto/hashes/Cargo.toml:13` defines `no-asm = ["keccak"]` (a
fallback that uses the keccak Rust crate instead of the assembly).
This feature is correctly identified by the producer as a "failsafe
for non-x86_64 platforms" but no other crate enables it (no other
Cargo.toml sets `features = ["no-asm"]`). The keccak crate is
**used** on aarch64/wasm32/windows via the conditional dependency
at line 27. On x86_64 with default features, the keccak crate is
NOT pulled in (the conditional line 27's predicate
`any(target_os = "windows", not(target_arch = "x86_64"))` is false)
and the build.rs compiles the .s files. So the producer's claim
that the .s files are dead on x86_64 is correct: the keccak Rust
crate is the alternative path on other arches, and on x86_64 neither
the Rust crate nor any extern "C" consumes the assembly.

### Probe F: workspace dead-dep count sanity

The producer claims 70 of 132 workspace-deps never reach cargo
metadata's resolve graph. I did not independently re-derive the
exact 70-dep list (would require running `cargo metadata --format-version=1`
and diffing against the workspace Cargo.toml's
`[workspace.dependencies]`). I spot-checked 5 of the 70 listed
deps: `aes`, `argon2`, `bech32`, `bitcoin`, `bs58` — all absent
from any member crate's `[dependencies]`. The category-level
finding is correct, even if the exact 70 count is hard to fully
re-derive without repeating the producer's exact workflow. **This
is a quantity finding, not a correctness finding** — the
"70-of-132 never resolved" claim is approximate, and the
deliverable is honest about that ("the truly safe-to-remove subset
is ~64").

---

## Minor findings on the deliverable itself

1. **F-HYG-14 wording:** "7/7 positive-only Merkle proof tests" —
   should be "9/9" (the table at line 535 correctly says 9; the
   inline text at line 85 says 7). Substance is unchanged.
2. **cellscript LOC:** "38K lines" rounded up; actual is 36,474
   lines. Within 4%, not material.
3. **cellscript "17 test files" is correct** when counting the 3
   subdirectory modules (common/mod.rs, support/*.rs).

These are all **wording/numerical rounding** issues, not
**finding substance** issues. They do not change the verdict.

---

## Verdict synthesis

- All 5 mandated checks PASS with independent reproduction.
- All 4 cargo audit advisories reproduce exactly.
- The keccak F-PRIM-18 finding is fully supported by source-level
  evidence (no `extern "C"` for `__KeccakF1600` / `KeccakF1600`).
- The test integrity findings (F-HYG-14 positive-only proof, F-PRIM-20
  sighash cross-check, F-HYG-11 cellscript no-proptest) are
  independently re-derived.
- The dead-dep findings (F-HYG-01 memmap2, F-HYG-05 exec deps) are
  independently re-derived.
- The proptest-regressions state is correctly characterized
  (correctly populated by-design, not a missing artifact).
- The "Conditional PASS" verdict is well-supported by the findings
  (1 CRITICAL, 3 HIGH release-gating concerns + otherwise sound
  hygiene).
- Two minor wording/numerical issues noted above; neither changes
  the substance of any finding or the verdict.

The deliverable is **high-quality**: every finding has a concrete
file:line citation, reproducible evidence (rg output, cargo audit
JSON, file listings), and cross-references to prior audit lanes. It
correctly extends STATE_DA F-06/F-17/F-25 and LANE_PRIMITIVES
F-PRIM-18/F-PRIM-20/F-PRIM-32/F-PRIM-37 without duplicating them.
The fixture-vs-implementation alignment analysis (cellscript
`ckb_compat_runner.rs` cross-checks model vs fixture) is the
strongest single claim in the deliverable and is well-supported.

OVERALL: PASS
VERDICT: PASS
