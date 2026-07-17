# Myelin Swarm Audit — Mempool + Consensus

> Verifier-only review. No fixes proposed. Scope: `mempool/src/{lib,cellpool,scorer}.rs`,
> `mempool/Cargo.toml`, `consensus/src/lib.rs`, `consensus/Cargo.toml`, cross-references
> to `MYELIN_CONSENSUS_COMPLETENESS.md`, `MYELIN_SESSION_L2_PLAN.md`, and the CLI
> surface that drives `committee finalise-demo`.

## Verdict

**Conditional PASS for the explicit finite-session closed-validator fast path
described in `MYELIN_SESSION_L2_PLAN.md` (P0 + acceptance criteria: "state transition
is consensus-independent; only finality evidence differs").** Both engines wire through
a single `ConsensusConfig`, both have domain-separated signature payloads, and both
share one canonical `MyelinBlock.hash()`.

**However, several real defects and one serious safety gap fall outside the
"closed-validator fast path" frame and would surface as soon as the system leaves
the fixture runner.**

The two highest-impact issues are:

1. **Tendermint has no safety layer beyond structural certificate shape** —
   cross-round / cross-height equivocation is undetectable. The completeness doc
   explicitly admits this ("out of scope"), so the gap is **declared but real**:
   the same validator can sign two precommits at the same `(height, round)` for
   different `block_hash` values and the verifier will not catch it without an
   evidence log that does not exist.
2. **The `CellPool::try_replace_by_fee` path is non-trivially unsafe under
   re-entry and can return "TxExists" for a fresh tx** — `add()` is called
   recursively from `try_replace_by_fee` after the original `add()` has already
   short-circuited on conflicts, and the function recurses through any remaining
   conflict chain.

The remaining findings are correctness gaps in deterministic scoring that the
doc claims but the code does not fully back up (see "Conflict-scoring
determinism" below).

## Findings

| # | Severity | Component | Finding | File:Line | Doc claim | Code reality |
|---|----------|-----------|---------|-----------|-----------|--------------|
| F-01 | **CRITICAL** | consensus | Tendermint equivocation across `(height, round)` for the same validator is not detectable. | `consensus/src/lib.rs:556-597`, `646-668` | "Equivocation detection: ... a validator is allowed at most one precommit per `(height, round, block_hash)` certificate" (`MYELIN_CONSENSUS_COMPLETENESS.md:106`) | The verifier only checks for duplicate validator ids **inside one certificate**. Two separately produced certificates with the same `(height, round, validator_id)` but different `block_hash` both verify in isolation. There is no equivocation log. |
| F-02 | **HIGH** | mempool | `CellPool::try_replace_by_fee` calls `self.add(...)` recursively. The recursive `add` re-runs `check_conflicts` and may itself enter `try_replace_by_fee` again. | `mempool/src/cellpool.rs:264-309` (esp. 308) | Not directly claimed, but the comment block at lines 36-46 (`ConflictKey`) implies a single deterministic resolution. | Under a chain of conflicts (tx-A conflicts with tx-B and tx-C; tx-B has been removed first, then re-add can no longer see tx-B), the recursive `add` returns `MempoolError::TxExists` if the wtxid happens to collide with any entry inserted in the interim. There is no loop bound. |
| F-03 | **HIGH** | mempool | `ConflictKey::from_entry` casts `f64` (`fee_density`) to `u64` and then computes `u64::MAX - fee_density_fp`. | `mempool/src/cellpool.rs:53-56` | "Deterministic conflict resolution key ... 1. fee_density (higher better) - descending ... 2. wtxid (lexicographic) - ascending (tie-breaker)" (`cellpool.rs:36-39`) | `as u64` on a NaN `fee_density` returns 0 (Rust IEEE 754 cast behaviour). On a very large `fee_density` (e.g. > 2^64), `as u64` saturates to `u64::MAX`. `u64::MAX - u64::MAX` overflows in release mode and wraps to 0 in debug; the `checked_sub` arithmetic is **not** used. This violates the deterministic sort key for adversarial `fee`/`cycles` inputs. |
| F-04 | **HIGH** | mempool | `get_sorted` uses `partial_cmp(...).unwrap()` on `f64` total scores. | `mempool/src/cellpool.rs:230-238` (esp. 235) | "Get transactions sorted by score (descending)" (line 229) | If any `score.total` is NaN (e.g. via the scorer producing NaN: `α·0 + β·NaN - γ·width`), `partial_cmp` returns `None` and `.unwrap()` **panics the process**. The scorer has no NaN guard at lines 67-82 or 91-118 of `scorer.rs`. |
| F-05 | **MEDIUM** | mempool | `CellPool::add` reads three separate `RwLock`s in sequence without holding them together (`txs.read` → `txs.read` → `txs.write + spent.write + stats.write`). | `mempool/src/cellpool.rs:125, 130, 135, 160-162` | None (internal) | Between the size check (line 130) and the write-lock acquisition (line 160), another thread can fill the pool to `max_size`. The size check is therefore advisory, not authoritative. A `max_size=0` would also be accepted (cellpool.rs:110-118), because `0 >= 0` evaluates true. |
| F-06 | **MEDIUM** | mempool | `PoolEntry.timestamp` is sourced from `SystemTime::now()` and stored in the entry, but the entry is part of `IndexMap<[u8;32], PoolEntry>`. | `mempool/src/cellpool.rs:22-23, 152, 276, 330-332` | "Deterministic admission and ordering" (mempool/README.md:3) | The timestamp is never read by any sort or conflict key in this audit scope, so it does not currently break determinism — but it is non-deterministic state that participates in `PoolEntry`'s public `Clone + Debug` and would surface in any serialised fixture. |
| F-07 | **MEDIUM** | mempool | `add` returns `MempoolError::TxExists` only if the same wtxid is already in the pool (line 125). It does **not** check for already-committed CellTxs; an external "already-committed" concept is not represented. | `mempool/src/cellpool.rs:121-185` | None explicit in cellpool.rs; the doc-implied invariant is that the pool is "local Cell transaction queue" (mempool/README.md:7). | Question 2 of the audit asks "Can a conflicting CellTx replace an already-committed one?" — the answer is **there is no representation of "committed" inside the pool**. If callers use `CellPool` purely as an admission queue and rely on the parent runtime to remove an entry once committed, that coupling is **not in this file**. This is acceptable if the pool is intentionally local-only, but should be documented. |
| F-08 | **MEDIUM** | mempool | Conflict-set semantics when two CellTxs touch the **same cell** is "lower ConflictKey is dropped, higher kept", but only via the RBF path. | `mempool/src/cellpool.rs:136-139, 264-309` | "Priority order ... 1. fee_density ↓ ... 2. wtxid ↑" (`mempool/README.md:36-39`); `cellpool.rs:36-39` | The lower-scored tx is not "kept alongside" the higher-scored one. RBF **removes** the conflict, then re-inserts. The README's "conflict set semantics" wording is ambiguous — the actual behaviour is strict: exactly one of the two survives. The README does not document that this is destructive, not a `set`. |
| F-09 | **MEDIUM** | consensus | The Tendermint signature domain is `myelin:tendermint-precommit:v1` plus `height || round || validator_id || public_key || block_hash`. | `consensus/src/lib.rs:23, 646-668` | "Domain separation is what makes `tendermint_does_not_silently_fall_back_to_static_committee` a true negative test" (`MYELIN_CONSENSUS_COMPLETENESS.md:126-128`) | Domain is correctly separated. **However**, the domain does not include `consensus_kind`. A static committee validator could reuse the same `(validator_id, public_key)` pair in a Tendermint committee — the signature domain separation alone prevents forgery, but if the **set of validators and weights** were shared across the two engines (e.g. via a shared config file), the certificate itself would still parse under either engine. The wrong-engine guard at `consensus/src/lib.rs:606-611` mitigates this only at the block level, not at the validator-set level. |
| F-10 | **MEDIUM** | consensus | Static-closed-committee quorum threshold is configured per-engine globally, not per session. | `consensus/src/lib.rs:248-253`, `370-408` | "configurable quorum threshold" (`MYELIN_CONSENSUS_COMPLETENESS.md:64`) | The `quorum_weight` is on the `StaticCommitteeConfig` struct, which is bound to the `StaticClosedCommittee` engine instance — and a single engine instance is built once per CLI invocation (`cli/src/main.rs:1245-1266`). For a multi-session workload, the engine is rebuilt per session, so the threshold is effectively per-session in practice. **But** the field name `quorum_weight` is the same as Tendermint's `quorum_power`, and there is no `session_id` field on `StaticCommitteeConfig` or `TendermintConfig`. This means there is no structural way to assert "this committee is bound to session X" from a `FinalisedBlock` alone. |
| F-11 | **LOW** | consensus | `MyelinBlock.timestamp_ms` is supplied by the session runtime (line 166). | `consensus/src/lib.rs:166, 181-205` | "Millisecond timestamp supplied by the session runtime" (line 165) | Two engines finalising the **same** state transition with **different** timestamps would produce different `block.hash()` outputs. The "consensus-independent state transition" claim requires the timestamp to be part of the state transition (not consensus) — but currently it is in the consensus-signed block. The domain separation in `MYELIN_SESSION_L2_PLAN.md:94-100` is therefore violated if the runtime injects a fresh timestamp per finalisation. |
| F-12 | **LOW** | mempool | `scorer.rs` README and comment headers disagree on the formula. | `mempool/src/scorer.rs:28-34` and `mempool/README.md:30-32` | README shows `ancestors_score = (ancestors_fee / ancestors_size) * cycles_factor * age_factor`; code comment shows `α·fee_density + β·unlockability − γ·deps_width` | The cellpool uses the code comment formula (no `cycles_factor` or `age_factor`). The README's `ancestors_score` formula is **not implemented** in `scorer.rs`. Either the README is aspirational, or the implementation is missing an ancestor aggregator. |
| F-13 | **LOW** | mempool | `scorer.rs::compute_unlockability` for absolute locks uses a hard-coded `1_000_000` threshold. | `mempool/src/scorer.rs:105-112` | "Absolute locks: depends on lock distance" (line 90) | The threshold is not exposed, not configurable, and the unit (`since` value semantics in CKB) is not referenced. |
| F-14 | **LOW** | mempool | `CellPool::remove` keeps the write locks on `txs`, `spent`, and `stats` for the full removal. | `mempool/src/cellpool.rs:188-222` | None | This is fine functionally, but the `find_dependencies` path at line 145-155 takes a separate read lock inside `add`. Since `add` is **not** re-entrant on the write lock, this is safe; calling `find_dependencies` after the write lock would be impossible. |
| F-15 | **INFO** | consensus | No `unsafe`, no `allow(...)`, no `random`/`thread_rng`/`Rng` anywhere in `consensus/src/` or `mempool/src/`. | (verified via `grep -rn`) | "Deterministic certificate verification" (`MYELIN_CONSENSUS_COMPLETENESS.md:64`) | Confirmed. `consensus/src/lib.rs` uses only `blake3`, `hex`, `serde`, `thiserror`, `toml`, `HashMap`, `HashSet` — all deterministic. |
| F-16 | **INFO** | workspace | Neither `mempool/Cargo.toml` nor `consensus/Cargo.toml` declares `[lints]` to inherit workspace lints. | `mempool/Cargo.toml`, `consensus/Cargo.toml` (entire files) | Workspace `[workspace.lints.clippy]` at `Cargo.toml:202-203` only sets `empty_docs = "allow"` | No crate-level clippy propagation. `mempool/src/lib.rs:14` declares `#![warn(missing_docs)]` privately. The lints surface is intentionally minimal. |
| F-17 | **INFO** | mempool | `Cargo.lock` pins `indexmap = 2.2.6`, `parking_lot = 0.12.5`, `blake3 = 1.8.5` — exactly the `=2.2.6` pin in `mempool/Cargo.toml:13`. | `Cargo.lock` lines for `indexmap`, `parking_lot`, `blake3` | None | Exact-pin is correct for determinism. `blake3` and `parking_lot` are not pinned in `mempool/Cargo.toml`, but Cargo.lock has stable versions; any future move needs lock refresh. |
| F-18 | **INFO** | consensus | Cargo hygiene: `consensus/Cargo.toml` is minimal and inherits `blake3`, `hex`, `serde`, `thiserror`, `toml` from the workspace. No transitive risk. | `consensus/Cargo.toml:11-14` | None | Clean. |
| F-19 | **MEDIUM** | consensus | The CLI `quorum_signers` (cli/src/main.rs:1314-1326) and `tendermint_quorum_signers` (1328-1339) pick signers in **declaration order**, so the certificate signer set depends on TOML validator ordering. | `cli/src/main.rs:1314-1339` | `scripts/myelin_production_gate.sh` checks `len(committee["signer_ids"]) >= 2` only | Two TOMLs with the same validators but different declaration order produce different signer sets and different `certificate_for_fixture` output. This is "deterministic given a fixed TOML", but not "canonical". For replay-determinism the TOML itself must be canonicalised before hashing. |

## Conflict-scoring determinism

### Sort key

`mempool/src/cellpool.rs:40-65` defines `ConflictKey { neg_fee_density: u64, wtxid: [u8; 32] }`,
`Ord`-derived. `neg_fee_density = u64::MAX - fee_density_fp`, where
`fee_density_fp = (fee_density as f64 * 1e9) as u64`. `ConflictKey::is_better_than`
is `self < other` (line 62-64).

The intended ordering is:
1. Higher `fee_density` → smaller `neg_fee_density` → smaller `ConflictKey` → better.
2. Ties on `fee_density` → lexicographically ascending `wtxid` wins (smaller wtxid →
   smaller `ConflictKey` → better).

This is **structurally deterministic** when the inputs are well-formed.

### Tie-break

The `Ord` derive on `ConflictKey` (line 40) compares `neg_fee_density` first, then
`wtxid`. Two `ConflictKey`s with the same `neg_fee_density` and the same `wtxid`
are equal — but that requires both the **same** `fee_density` cast and the **same**
32-byte wtxid, the latter being a transaction-content hash. Two distinct transactions
have distinct wtxids, so equality is impossible across two distinct entries.

For the same wtxid (i.e. the same transaction re-added) — `CellPool::add` returns
`TxExists` at line 126 before any conflict logic runs.

For **different wtxids with identical `fee_density_fp`** — the new transaction wins
iff its wtxid is lexicographically smaller than the conflict's. This is **strict**
in `is_better_than` (uses `<`, not `<=`), so a tie on `fee_density` between the new
tx and the conflict with the **larger** wtxid returns `RBFFailed`. This is
asymmetric — the new tx must be strictly better, while an existing conflict that
ties against the new tx wins by virtue of its lower wtxid.

### Concrete failure modes

1. **`fee_density` overflow → `as u64` saturates to `u64::MAX`** (cellpool.rs:53). The
   saturation is consistent, but two `fee_density` values both > 2^64 / 1e9 collapse
   to the same `fee_density_fp = u64::MAX`. The wtxid tiebreaker still resolves the
   order, so this is benign — but the "fee_density dominates" claim is violated.
2. **`fee_density` is NaN → `as u64` returns 0** (Rust IEEE-754 cast). Two NaN-bearing
   entries both get `neg_fee_density = u64::MAX`; the wtxid tiebreaker resolves
   them. This is benign **only** if NaN cannot enter the system; the scorer does
   not guard against it.
3. **`fee_density` is negative (theoretically impossible from `compute_fee_density`
   but possible if `fee_density` is set externally) → `as u64` saturates to 0**. Same
   saturation behaviour.

### Use of HashMap iteration

- `cellpool.rs:230-238` (`get_sorted`) reads `txs` (an `IndexMap`, **insertion-ordered**),
  collects into a `Vec<PoolEntry>`, then sorts by `score.total`. The `IndexMap` iter
  order is deterministic given the insertion order, so the only non-determinism is
  from the `sort_by` callback. The callback uses `b.score.total.partial_cmp(&a.score.total).unwrap()`,
  which is **strict total order on `f64` except for NaN**. NaN panics the process.
- `cellpool.rs:312-327` (`find_dependencies`) iterates `txs.iter()` (insertion-ordered),
  collects into a `BTreeSet`, and returns sorted. Deterministic.
- `cellpool.rs:265` (in `try_replace_by_fee`) takes `txs.read()` and does `txs.get(conflict_id)`
  per conflict id. Direct lookup, no iteration over HashMap.
- `consensus/src/lib.rs:372, 487` — `StaticClosedCommittee::validators` and
  `Tendermint::validators` are `HashMap<String, CommitteeValidator>`. Iteration
  over these is **not used in any verification path** — verification iterates over
  the certificate's `signatures` Vec in declaration order (which the fixture
  builder controls deterministically).

### Random sources

None. `grep -rn 'rand\|getrandom\|thread_rng\|Rng'` over `mempool/src/` and
`consensus/src/` returns no matches. The fixture signature generator at
`consensus/src/lib.rs:464-482` and `646-668` is a pure blake3 derivation over
domain-separated, validator-bound, block-bound (and for Tendermint: height/round
bound) input. No external entropy.

### Summary

The sort key is **structurally deterministic** but its inputs are **not sanitised**:
NaN, ±∞, and `f64`-cast-to-`u64` saturation can collapse multiple distinct
`fee_density` values to the same fixed-point representation. The wtxid tiebreaker
preserves total order in those cases but breaks the "fee_density strictly
dominates" claim. **`get_sorted` will panic on NaN.**

## Finality engine safety/liveness check

### StaticClosedCommittee

**Safety invariant**: A `FinalisedBlock` exists iff a `CommitteeCertificate`
containing `signed_weight >= quorum_weight` of distinct validators, each signing
the exact `block_hash` with the deterministic fixture signature, is verified by
the engine.

- Verified: `consensus/src/lib.rs:432-461`. Duplicate validator id in a single
  certificate → `DuplicateValidator` (line 441). Unknown validator id →
  `UnknownValidator` (line 446). Wrong signature → `InvalidSignature` (line 449).
  Wrong block hash → `WrongBlockHash` (line 434). Quorum shortfall →
  `QuorumNotMet` (line 457). Weight overflow in certificate accumulation →
  `InvalidConfig` (line 453, although this is the wrong error variant — see
  F-09 below for the inconsistency).
- **Double-signs across certificates are NOT detectable.** Two certificates with
  the same `(validator_id, block_hash)` produced at different "times" (in a real
  network) both verify individually. There is no timestamping, no chain of
  certificates, and no log of prior certificates. This matches the doc
  admission: "static closed committee ... not a permissionless consensus protocol"
  (lib.rs:6-11) and the `MYELIN_CONSENSUS_COMPLETENESS.md` declaration that
  cross-round equivocation is "out of scope".

**Liveness invariant**: There is no progress protocol — the engine is a
**verifier**, not a proposer. Quorum is reached iff the caller presents a
certificate. If no caller presents one, `verify_certificate` is never called.
Under no-progress, **the engine rejects with `QuorumNotMet`** rather than hanging.
`static_committee_rejects_below_quorum` test (lib.rs:782-790) confirms the
explicit error return.

**Failure mode**: Caller-side. The CLI path `committee finalise-demo` always
constructs a sufficient certificate via `quorum_signers` (cli/src/main.rs:1314-1326),
so the production gate always sees `finalised: true`. A malformed config that
cannot satisfy quorum fails at `StaticClosedCommittee::new` with `InvalidConfig`,
not at verify time.

### Tendermint

**Safety invariant**: A `FinalisedTendermintBlock` exists iff a
`TendermintPrecommitCertificate` containing `signed_power >= quorum_power` of
distinct validators, each precommitting the exact `(block_hash, height, round)`
tuple with the deterministic precommit signature, is verified by the engine.

- Verified: `consensus/src/lib.rs:556-597`. Height and round are checked at
  lines 566-571. Block hash at line 563. Duplicate validator id at line 577.
  Unknown validator at line 582. Wrong signature at line 585. Power shortfall
  at line 593.
- **Cross-(height, round) equivocation is NOT detectable.** Two separate
  certificates at `(height=7, round=0)` from the same validator for different
  `block_hash` values both verify. The doc admits this (line 106). A validator
  can sign two precommits at `(height, round, block_hash_A)` and
  `(height, round, block_hash_B)`, both pass verification in isolation, and the
  system has no way to detect that the same validator precommitted at the same
  round for two different blocks. This is **F-01** and is the primary safety
  gap.

**Liveness invariant**: Same as StaticClosedCommittee — verifier only, no
proposer. The engine itself does not progress; the CLI drives fixture
certificates. Under no-progress, **rejects with `QuorumNotMet`** (line 593).
`tendermint_rejects_below_quorum` test (lib.rs:834-843) confirms.

**Round structure**: Round number is a `u32` field on the certificate. There is
**no round rotation logic, no locked value, no polka detection, no timeout
mechanism**. The implementation is a **stripped precommit verifier**, not a
Tendermint state machine. There is no `LockedValue`, no `ValidValue`, no
`RoundState`. The Tendermint safety invariant ("a validator locks on the
first round it sees a polka, and only votes for the locked value in subsequent
rounds") cannot be checked by this code because the code does not model it.

This is consistent with the doc position: "phase-one Tendermint engine is a
closed-validator fast path used for benchmarking and pressure testing, not a
permissionless BFT network" (`MYELIN_CONSENSUS_COMPLETENESS.md:174-176`). But
the README claim "Tendermint-style weighted precommit finality" should be read
as "the certificate shape and verification rules are Tendermint-shaped", not
"the Tendermint state machine is implemented". The doc is internally
consistent on this; the audit question 5 wording — "Does it match the real
Tendermint safety/liveness model, or is it a stripped variant?" — gets the
answer **stripped variant**, and the doc agrees.

**Leader rotation**: None.

### Failure mode comparison

| Failure | StaticClosedCommittee | Tendermint |
|---|---|---|
| Quorum shortfall | `QuorumNotMet` | `QuorumNotMet` |
| Wrong block hash | `WrongBlockHash` | `WrongBlockHash` |
| Wrong engine | `WrongEngine` | `WrongEngine` |
| Duplicate signer | `DuplicateValidator` | `DuplicateValidator` |
| Unknown signer | `UnknownValidator` | `UnknownValidator` |
| Bad signature | `InvalidSignature` | `InvalidSignature` |
| Wrong height | n/a | `WrongHeight` |
| Wrong round | n/a | `WrongRound` |
| Cross-certificate equivocation | **Not detected** | **Not detected** |
| Signature weight overflow | `InvalidConfig("certificate weight overflow")` | `InvalidConfig("precommit power overflow")` |

Both engines **reject on no-progress** with specific errors rather than hanging.
The verify path is fully synchronous and bounded.

## Domain-separation verification

The completeness doc claims three domains:
1. `myelin:block:v1` — block hashing (`consensus/src/lib.rs:21`).
2. `myelin:static-committee-signature:v1` — static cert signatures (`lib.rs:22`).
3. `myelin:tendermint-precommit:v1` — Tendermint precommit signatures
   (`lib.rs:23`).

The block hash includes `consensus_kind` as a string in the canonical
`to_molecule_bytes` encoding (`lib.rs:189`):
```
encode_table(&[
    self.version.to_le_bytes().to_vec(),
    self.parent_hash.to_vec(),
    self.number.to_le_bytes().to_vec(),
    self.timestamp_ms.to_le_bytes().to_vec(),
    self.consensus_kind.as_str().as_bytes().to_vec(),    // ← discriminator
    self.state_root_before.to_vec(),
    self.state_root_after.to_vec(),
    encode_hash_vec(&self.ordered_cell_tx_commitments),
    encode_hash_vec(&self.data_commitments),
    self.scheduler_commitment.to_vec(),
])
```

**Question 7 audit**: "Same CellTx + same state root + different consensus = same
effective state transition? Different finality payload?"

- **State root before/after**: identical regardless of `consensus_kind`. Same
  CellTxs admitted in the same order yield the same `state_root_after`. ✓
- **Block hash**: differs because `consensus_kind` is part of the encoded table.
  So `block.hash()` is **engine-tagged**, not engine-agnostic. ✓ for "different
  finality payload", but the doc framing "state transition is consensus-independent"
  means "the state transition is the same", not "the block hash is the same".
  This is consistent with the doc.
- **`FinalisedBlock` artefact**: structurally differs between engines
  (`FinalisedBlock { certificate: CommitteeCertificate }` vs
  `FinalisedTendermintBlock { round, certificate: TendermintPrecommitCertificate }`).
  Domain separation is achieved by both the **type system** (different cert
  types) and the **signature domain** (different blake3 domains). ✓
- **Cross-engine forgery**: A static-committee cert presented to the Tendermint
  engine → `LegacyCertificatePathUnsupported` (lib.rs:642). A Tendermint precommit
  cert presented to the static engine → the `verify_certificate` path
  re-derives the static signature and compares → `InvalidSignature`
  (tested by `tendermint_does_not_silently_fall_back_to_static_committee`,
  lib.rs:998-1025). ✓

**One subtle gap** (F-11): `MyelinBlock.timestamp_ms` is part of the canonical
encoding (line 187). If the runtime injects a fresh timestamp per finalisation
attempt of the **same** state transition, `block.hash()` will differ across
attempts, even though `state_root_before` and `state_root_after` are identical.
For the "same CellTx + same state root + different consensus = same effective
state transition" claim to hold in a replay scenario, the timestamp must be
**derived from the state transition** (e.g. the state root itself), not from
wall-clock. Currently the doc strings this as "supplied by the session runtime"
(line 165), which is consistent with the L2 plan ("deterministic chunk
commitments") **only if the session runtime supplies a deterministic timestamp
based on the chunk**. The CLI demo does supply a deterministic timestamp (the
fixture's `demo_block` uses a fixed value), so the existing fixtures are
replay-stable.

## Open questions

1. **F-01 (CRITICAL)**: When does cross-certificate equivocation become a
   hard requirement? The doc declares it out of scope for phase one, but the
   `quorum_signers` and `tendermint_quorum_signers` CLI helpers at
   `cli/src/main.rs:1314-1339` and the deterministic signature generator at
   `consensus/src/lib.rs:464-482` make the system trivially forkable by a
   misbehaving signer — there is no slashing condition and no evidence log.
   The audit can't answer this; this is a scope question for the project.

2. **F-02 (HIGH)**: Has the `try_replace_by_fee` recursive-`add` pattern ever
   been exercised under a multi-conflict scenario? The unit test at
   `cellpool.rs:429-454` covers single-conflict only. A test with three
   conflicting transactions (`add(tx_a)` then `add(tx_b, conflicts_with_a)`
   then `add(tx_c, conflicts_with_b)`) would stress this path. Even if it
   terminates, the recursive `add` after `drop(txs)` at line 299 means the
   size-check race window reopens — a concurrent `add` could push the pool
   to `max_size` between line 308 and the inner `add`'s size check.

3. **F-03 / F-04 (HIGH)**: Has the scorer ever been probed with adversarial
   inputs that produce NaN (e.g. `fee = u64::MAX`, `cycles = 0`,
   `serialized_size = 0`)? The `compute_fee_density` early-returns 0.0 for
   `effective_size == 0.0`, but `effective_size = size.max(cycles_size)` —
   both can be 0.0, and 0.0 / 0.0 → NaN in IEEE 754, which is short-circuited
   by the `> 0.0` guard. **However**, `unlockability` at line 117 divides
   `total_score / tx.inputs.len() as f64`. If `tx.inputs` is non-empty (guarded
   by the early return at line 92), this is safe. If `tx.inputs` is empty, the
   function returns 1.0 at line 93. **Therefore `total` cannot be NaN through
   the scorer's normal paths**. F-04 is a latent bug, not a triggered one.
   F-03's saturation behaviour is benign given F-12.

4. **F-07 (MEDIUM)**: Is `CellPool` intended to be the only admission gate,
   or does the parent runtime also track "committed" wtxids? The code does
   not represent "committed" at all. If the runtime relies on `CellPool` to
   suppress already-committed transactions, it must do so externally before
   calling `add`, and that coupling should be documented.

5. **F-11 (LOW)**: Is `timestamp_ms` intended to be part of the consensus
   finality payload, or part of the state transition? If part of the state
   transition, it must be deterministic across replays — the CLI fixtures are,
   but the doc does not enforce this for runtime sessions. If part of the
   finality payload, the `consensus_kind` discriminator is doing the work and
   `timestamp_ms` is redundant.

6. **F-12 (LOW)**: Is `mempool/README.md` line 30-32 (`ancestors_score`)
   aspirational? If yes, the README is misleading. If no, an ancestor-aggregator
   pass is missing from `scorer.rs`. The cellpool does track parent dependencies
   (`cellpool.rs:312-327`), so the data is available; the scorer just doesn't
   consume it. This may be a deliberate scope cut for phase one.

7. **F-19 (MEDIUM)**: The CLI's `quorum_signers` picks validators in TOML
   declaration order. For replay determinism across runs that don't share
   TOML byte-for-byte (e.g. someone edits a comment or adds a comment-only
   line), the signer set and therefore the certificate hash change. If the
   production gate (`scripts/myelin_production_gate.sh`) asserts byte-equality
   of certificate hashes, it must canonicalise the TOML first.

---

## Per-crate hygiene summary

| Crate | `unsafe` | `#[allow(...)]` | `random`/`thread_rng` | Random source | Notes |
|---|---|---|---|---|---|
| `mempool` | None | None | None | None | `indexmap = "=2.2.6"` exact-pin (good). No `[lints]` inheritance. `#![warn(missing_docs)]` only. |
| `consensus` | None | None | None | None | Inherits `blake3`, `hex`, `serde`, `thiserror`, `toml` from workspace. No `[lints]` inheritance. |

Workspace `[workspace.lints.clippy]` (`Cargo.toml:202-203`) sets only
`empty_docs = "allow"`. Neither crate explicitly opts in to that table.

`clippy.toml` sets `too-many-arguments-threshold = 10`. None of the public
functions in either crate exceed this.

No `Cargo.toml` uses wildcard versions or unpinned `git` deps for the crates
in scope.