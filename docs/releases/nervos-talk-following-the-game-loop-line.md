# Following the game-loop line: what xuejie's OHOL and Archipelago posts mean for Myelin

> **Draft Nervos Talk post.** This is a standalone article intended for the
> Nervos Talk forum. It continues the research line opened in
> [*Continuing the Teeworlds-on-CKB line*](teeworlds-lineage.md) by engaging
> with three subsequent posts by xuejie:
> [*Fat transactions, thin transactions*](https://xuejie.space/2026_06_24_fat_transactions/),
> [*Porting One Hour One Life's game loop to CKB*](https://xuejie.space/2026_06_29_porting_one_hour_one_life_game_loop_to_ckb/),
> and [*Archipelagos: exploring on-chain game design for OHOL*](https://xuejie.space/2026_06_30_archipelago/).
> Each post surfaces a design pressure that maps directly onto Myelin's
> off-chain session runtime, and together they sharpen a concrete roadmap.

---

## The line so far

In June, xuejie published a sequence of posts that, read together, trace a
single arc:

1. **Teeworlds on CKB** — a MOBA-style real-time game runs its tick loop
   inside CKB-VM as one chunk per transaction.
2. **Fat transactions, thin transactions** — a general framework for what a
   transaction is allowed to carry: witness, input/output, and data are
   independent *expansion axes*, and different chains pick different points
   in that space. CKB's model is "fat on data, explicit on witness."
3. **Porting One Hour One Life** — a *different* game genre (persistent,
   crafting-heavy, large world) runs on CKB-VM. OHOL needs a *world state
   hash* and a *transit cell* that crosses chunks, not just a replay tape.
4. **Archipelagos** — when a single world cannot fit in one chunk, shard it
   into islands with independent rule sets, linked by a customs/signature
   border, and let cross-island movement flow through a transit cell.

The arc moves from "can the VM do it at all?" → "what is a transaction
allowed to contain?" → "how does a persistent world survive across chunks?"
→ "how do we partition a world that does not fit?".

Myelin's thesis is that these are exactly the questions an off-chain session
runtime must answer, and that answering them *off-chain* — with a path back
to L1 — is cheaper and more expressive than answering them on-chain. This
post maps each of xuejie's design pressures to a part of Myelin's
architecture, marks what we can already support, what we should borrow, and
what becomes a roadmap item.

## 1. Fat / thin transactions → Myelin's CellTx already separates the axes

xuejie's [*fat / thin* post](https://xuejie.space/2026_06_24_fat_transactions/)
argues that a transaction's "weight" is not one number but three independent
expansion axes:

- **witness weight** (signatures, proofs),
- **input/output weight** (how many cells consumed/created),
- **data weight** (the payload carried in the cells).

CKB is deliberately *fat on data* (cells carry arbitrary bytes) and *thin on
implicit state* (no hidden account storage). The post's practical takeaway:
when you design a verifiable computation, you get to choose *where* the
evidence lives — in witnesses, in consumed cells, or in created cells — and
that choice determines what a court must re-read to adjudicate a dispute.

### How Myelin maps

Myelin's `CellTx` already models all three axes independently, mirroring CKB:

```text
CellTx {
    inputs:      Vec<CellInput>,     // input/output axis (OutPoint → consumed cell)
    outputs:     Vec<CellOutput>,    //   + capacity + lock + type script
    outputs_data: Vec<Vec<u8>>,      // DATA axis (the fat payload)
    witnesses:   Vec<Vec<u8>>,       // WITNESS axis (signatures, scheduler witnesses)
    cell_deps:   Vec<CellDep>,       // read-only code/data refs
    header_deps: Vec<[u8;32]>,       // header refs
}
```

This is not accidental — we inherited CKB's transaction shape precisely so
that a Myelin CellTx is *projectable* into a CKB-style transaction. The fat/thin
framing validates that decision and gives us a vocabulary for the
[`CkbProjectionReport`](../security/claim-ladder.md): when we say a chunk is
"projectable," we mean its witness/input/data axes all have a CKB-compatible
shape.

### What we borrow

The post's sharpest insight for us: **the witness is where scheduler policy
belongs, not the data**. Myelin's CellScript scheduler witnesses
(effect class, conflict hash, parallelizable hint) ride in the *witness*
axis, keeping the *data* axis (the actual game/world payload) clean for court
replay. The cellscript→Myelin [witness bridge](https://github.com/Myelin-Network/Myelin/blob/main/exec/src/celltx/witness_bridge.rs)
respects this split: it translates compiler metadata from the witness without
touching the data payload. Going forward, any new "what did this chunk touch?"
evidence should land in the witness axis, not the data axis, so a future court
verifier can read policy without re-loading the full world.

## 2. One Hour One Life → a world-state-hash and a transit cell

The [OHOL port](https://xuejie.space/2026_06_29_porting_one_hour_one_life_game_loop_to_ckb/)
is the most architecturally revealing of the three posts. OHOL is not a
replayable tape like Teeworlds; it is a *persistent crafting world* where
each chunk must produce a **world state hash** that the next chunk consumes.
The design that emerged:

- Each chunk transition is `prev_world_hash → new_world_hash`, with the full
  world state serialised in a created cell.
- To let a *player* (not the whole world) cross chunk boundaries without
  re-serialising the world, xuejie introduced a **transit cell**: a small,
  self-contained cell that carries just the player's state across the chunk
  border.

This is, in miniature, the exact problem a session runtime solves: how to
chain finite executions into a persistent state thread, and how to move
fine-grained state (a player, an order, a meter reading) across the boundary
without re-committing the whole world each time.

### How Myelin maps — and where OHOL exposes a gap

Myelin already has the *world-state-hash* half:

| OHOL concept | Myelin analogue | Status |
| --- | --- | --- |
| `world_state_hash` per chunk | `CellStateTree` root (incremental MuHash, O(1) per op) | **Implemented** |
| chunk = `prev_hash → new_hash` | `session commit` produces `state_root_before → state_root_after` | **Implemented** |
| consumed/created cells per chunk | `CellTx` inputs/outputs + `CellDAG` scheduling | **Implemented** |
| **transit cell** (fine-grained state crossing a boundary) | a typed output cell consumed by the next tx | **Supported by CellTx, but not yet a first-class pattern** |

The gap OHOL exposes: Myelin's `session commit` today commits a *chunk* and
rolls the full state root, but we do not yet have an explicit, compiler-aware
**transit-cell pattern** — a declared, typed cell whose purpose is to carry a
slice of session state (one player, one order book shard, one IoT gateway
batch) from one chunk to the next without forcing the whole session state to
re-serialise. The CellTx shape supports it (any output can be consumed by a
later input), and CellScript's typed-cell profile can express the type, but
the *pattern* is not named or validated.

### Roadmap value: high

This is a concrete, near-term addition that costs little and unlocks a class
of use cases (persistent game worlds, sharded order books, multi-tenant IoT)
that the full-state-roll approach makes expensive. Specifically:

- Name a `TransitCell` convention in the typed-cell profile: a typed output
  whose `conflict_key` is scoped to a single participant/shard, so the
  CellDAG can schedule it independently of the world-root commit.
- Validate transit-cell chains in the court bundle (the bundle already
  packages consumed/created cells; adding a "this output is the next chunk's
  input" binding is a witness-axis addition).
- The witness bridge already recomputes `conflict_hash` from the cell's
  type script, so a transit cell's conflict domain is automatically correct.

## 3. Archipelagos → typed-cell islands and a customs border

[Archipelagos](https://xuejie.space/2026_06_30_archipelago/) is xuejie's
answer to "the world does not fit in one chunk." Shard the world into
**islands**, each with:

- its own **rule set** (type script / code),
- its own world state,
- a **customs/signature border** for cross-island movement,
- **transit cells** as the only way state crosses.

The post explores how islands can run different logic, how a player hopping
islands is really a sequence of transit-cell spends, and how this keeps each
chunk verifiable while the *archipelago* as a whole is the composition.

### How Myelin maps — almost exactly

This is where Myelin's typed-cell design pays off. An "island" maps directly
onto a **typed-cell domain**:

| Archipelago concept | Myelin analogue | Status |
| --- | --- | --- |
| island = independent rule set | `TypedCellDecl` per type script; `TypedCellStore::get_decl` | **Implemented** |
| different code per island | distinct type script `code_hash` per island | **Implemented** |
| cross-island movement | a CellTx consuming an output of island A and creating one of island B | **Supported by CellTx** |
| shared, contested resources | `conflict_hash` + `AccessMode` Read/Write in `CellDAG` | **Implemented** |
| customs/signature admission | lock-script group verification (`ScriptGroupType::Lock`) | **Implemented** |

In other words: Myelin's runtime already models a multi-rule-set,
multi-domain world where each domain has its own type script, its own conflict
domain, and a lock-script admission gate. The archipelago pattern is *almost*
a free consequence of the typed-cell model — a session that touches two
islands is just a session whose txs carry two different type scripts, and the
CellDAG already schedules them by conflict domain.

### What we borrow — and the one gap

The archipelago post's most useful idea for us is the **customs border as an
explicit, verifiable boundary**. Today Myelin verifies lock groups and type
groups, but it does not yet frame the *composition* (which islands a session
spans, which transit cells crossed which borders) as a first-class,
court-visible artefact. Borrowing the framing:

- A **session archipelago manifest** — a witness-axis summary of which typed
  domains a session touched and which outputs crossed domain boundaries —
  would make multi-island sessions auditable as a unit, without re-running
  every tx. This is the same shape as the existing court bundle (which
  packages one disputed chunk) but lifted to the *composition* level.
- Because the witness bridge already binds each access to a type script, the
  data for such a manifest already exists in the scheduler witnesses — it
  just needs to be aggregated and committed.

### Roadmap value: medium-high

Not blocking for the first use cases (single-domain sessions like a Teeworlds
match or an IoT metering window), but it is the natural shape for the
multi-tenant, multi-domain workloads in our
[use-case positioning](https://github.com/Myelin-Network/Myelin/blob/main/MYELIN_USE_CASE_POSITIONING.md)
(cross-org IoT, sharded order books, tournament economies). And it costs
little because the substrate (typed cells, conflict domains, lock admission)
is already built.

## Synthesis: what the three posts change about Myelin's roadmap

Read as a sequence, the three posts sharpen three things Myelin should do
*next*, all of which build on existing substrate rather than requiring new
foundations:

1. **First-class the transit-cell pattern.** OHOL proved that persistent worlds
   need a fine-grained state carrier across chunk boundaries. Myelin's CellTx
   and typed-cell profile already support it; we should name it, validate it,
   and let the CellDAG schedule transit cells independently of the world-root
   commit. *(Near-term; high value.)*

2. **Keep policy in the witness axis, payload in the data axis.** The fat/thin
   framing gives us a discipline: anything a future court must read to decide
   a dispute (scheduler policy, conflict domains, archipelago borders) belongs
   in the witness, not the data. The witness bridge already follows this; we
   should make it an explicit design rule for all new evidence.

3. **Lift the court bundle to a composition manifest for multi-domain sessions.**
   Archipelagos shows that the interesting unit of dispute is not always a
   single chunk but the *set of domain crossings* a session performed. The
   data for this already flows through the scheduler witnesses; aggregating
   it into a session-level manifest is a witness-axis, court-visible addition.

None of these require changing CKB-VM, the CellTx shape, or the typed-cell
profile. They are naming, validating, and aggregating patterns the runtime
already supports — exactly the kind of work that belongs in the off-chain
session layer xuejie named as future work.

## What this does not claim

Consistent with our [claim ladder](../security/claim-ladder.md):

- We have not run OHOL or an archipelago workload through Myelin. The
  mappings above are architectural (the substrate supports the pattern), not
  measured evidence.
- Transit-cell and archipelago-manifest support is *proposed roadmap*, not
  shipped. The Teeworlds reference workload remains the only end-to-end
  exercised path (Tier 2).
- On-chain court adjudication of any of these patterns is Tier 3 / future
  work; the court bundle is the input shape, not a deployed verifier.

## Credit

The fat/thin framework, the OHOL port, the transit-cell idea, and the
archipelago design are entirely xuejie's. This post maps them onto Myelin's
architecture to make the research line and the roadmap explicit; it does not
claim to improve on the original designs.

## Related

- [*Continuing the Teeworlds-on-CKB line*](teeworlds-lineage.md) — the first
  post in this lineage, covering the Teeworlds experiment.
- [Concurrency plan](../operations/concurrency-optimization-plan.md) — the
  CellDAG / typed conflict scheduling substrate the transit-cell and
  archipelago patterns would build on.
- [Claim ladder](../security/claim-ladder.md) — the Tier 0–3 boundary.
