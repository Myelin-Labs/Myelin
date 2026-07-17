# Continuing the Teeworlds-on-CKB line

> **Release positioning note.** This document states where Myelin sits in the
> research line opened by [xxuejie's *Teeworlds on CKB*](https://xuejie.space/2026_06_16_teeworlds_on_ckb/)
> experiment, what we add on top of it, and what we deliberately do not claim.
> It is a framing document, not a security claim — for the latter, see the
> [claim ladder](../security/claim-ladder.md).

## The line we are continuing

In June 2026, xxuejie published [*Teeworlds on CKB*](https://xuejie.space/2026_06_16_teeworlds_on_ckb/) —
a demonstration that a real-time, 60 Hz multiplayer game can run its core
gameplay loop **inside** CKB-VM, on chain, as a single transaction per chunk.
The thesis was sharp and correct:

- Blockchain VMs are now powerful enough for logic far beyond token transfers.
- Unlocking that power requires co-design: the program must be reworked
  (fixed-point math, custom collision, musl/libcxx ports, libc stripping) and
  the VM must be pushed (cycle budgets, spawn/IPC) at the same time.
- A game chunk must be **self-contained** — verifiable in one transaction, with
  no need to bisect the game state across many transactions.

That experiment proved the *execution* dimension. It also, importantly,
**stopped there**. The author was explicit that the parts outside the VM —
trustless setup, cheating resistance, permissionless operation, how chunks
compose into sessions, how disputes reach a chain — were not his thesis and
were left as future work ("*people have extensively studied this area … state
channel based solutions, or ephemeral rollups*").

Myelin picks up exactly where that demonstration ends. We do not re-litigate
the VM-capability question — xuejie settled it, and we reuse his replayer
binary directly. We build the layer he deliberately deferred: the off-chain
session runtime that sits above a single verified chunk.

## What xuejie proved, and what we reuse

| Dimension | xuejie's contribution | How Myelin uses it |
| --- | --- | --- |
| Complex logic in CKB-VM | Full Teeworlds tick loop, ~15.1M cycles/chunk, RISC-V | Reused verbatim — we run the same `replayer_stripped` ELF through our VM verifier |
| Program-side optimization | Fixed-point math, alpha-max-beta-nomin sqrt, collision rewrite, musl/libcxx, libc strip | Untouched — we treat the VM as a fixed oracle and do no in-VM optimization |
| Chunk philosophy | One chunk = one tx, atomic, no bisection | Adopted unchanged — Myelin's court bundle is built around the single disputed chunk |

We do not compete with, or claim to improve on, the in-VM work. Our
`vm_cycles: 15,139,695` is xuejie's number, produced by xuejie's binary. The
correct framing is **lineage, not substitution**.

## What Myelin adds — the layer above the chunk

The gap xuejie identified and deferred is exactly the layer Myelin is. Our
contribution is the off-chain runtime that turns a sequence of verified chunks
into a finite, finalised, contestable Cell session. Concretely:

### 1. Inter-transaction conflict scheduling (CellDAG)

A single chunk-in-one-tx answers "did this chunk execute correctly?" It does
not answer "how do many independent chunks in a session batch execute
**together**, and which ones can run in parallel?" Myelin's
[`CellDAG`](../operations/concurrency-optimization-plan.md) builds a read/write
dependency graph over the transactions in a session batch and schedules
independent transactions across Rayon topological layers — the first
inter-transaction parallelism on top of the chunk model. The
`scheduler-plan` / `session commit-multi` path exposes the layering and
per-tx cycle accounting.

### 2. CellScript compiler metadata → runtime scheduling

CellScript (the compiler that targets the typed-cell profile) emits per-action
scheduler witnesses: effect class, parallelizable hint, estimated cycles, and
per-access conflict domains. Myelin's runtime now **consumes** this metadata:
the [`witness_bridge`](https://github.com/Myelin-Network/Myelin/blob/main/exec/src/celltx/witness_bridge.rs)
decodes the compiler's witness format and recomputes typed conflict hashes
from the transaction's concrete cells, so the CellDAG's parallel layers are
driven by real compiler analysis, not just OutPoint-level structure.

### 3. Closed-validator finality with dual engines

xuejie's demo was a one-shot VM execution with no notion of a committed block
or a finality boundary. Myelin wraps a batch of verified chunks into a
`MyelinBlock` and finalises it under a pluggable committee: a static closed
committee today, and a Tendermint-style weighted-precommit verifier that is
domain-separated and tested alongside it. This is explicitly
**closed-validator** (see the [security boundary](#what-we-do-not-claim)) —
it is the session-finality layer a benchmarking and pressure-test workload
needs, not a permissionless consensus claim.

### 4. CKB-style projection and the court bundle

This is the connective tissue back to L1. For each chunk, Myelin emits a
`CkbProjectionReport` (is this chunk projectable into a CKB-style
transaction?) and, for a disputed chunk, a self-contained **court bundle**
that packages the witness layout, the molecule transaction, the chunk data,
and the committee finality evidence. The bundle passes 22 verification
checks (6 data-binding + 16 structural) and is the input shape a future
on-chain court verifier would consume. Today the on-chain court script does
not exist (`l1_court_implemented: false`) — but the bundle is real, the
shape is fixed, and the path is documented end to end.

### 5. Data-availability evidence path

xuejie's demo had no DA story. Myelin emits a DA manifest over sealed
segments (Merkle-rooted, parallel leaf hashing), with a replicated-committee
availability evidence layer and a hook for an external DA receipt. The DA
path is local-only today (no external provider), but the commitment shape is
in place.

## Why this layer matters

The Teeworlds-on-CKB result reframes what "a transaction" can be. But a
reframed transaction is not yet a usable system. To go from "one chunk verifies
in the VM" to "a game session (or a metering window, or a settlement batch) is
a contestable off-chain state transition with a path back to L1," you need
exactly the pieces above: a scheduler, a finality boundary, a projection
path, and a dispute bundle. That is the layer Myelin exists to provide, and
it is the layer xuejie named as future work.

The use cases this unlocks all share the *finite Cell session* shape — see
[Use-case positioning](https://github.com/Myelin-Network/Myelin/blob/main/MYELIN_USE_CASE_POSITIONING.md):
game sessions, industrial IoT metering, RFQ / market-maker settlement,
streaming payments. Teeworlds is the first and most thoroughly exercised
reference workload in that class.

## What we do **not** claim

This note is a positioning statement, not a production-readiness claim. The
hard boundaries, all inherited from the [claim ladder](../security/claim-ladder.md)
and the [production rehearsal report](https://github.com/Myelin-Network/Myelin/blob/main/MYELIN_PRODUCTION_REHEARSAL_REPORT.md):

- **Closed-validator finality only.** The committee is static and known.
Permissionless validator entry is out of scope today. Static committee
finality must not be marketed as permissionless L2 security.
- **No on-chain court yet.** The court bundle is the *input shape* for a
future on-chain verifier; the verifier itself is not deployed
(`l1_court_implemented: false`). We sit at **Tier 2** of the claim ladder
(executable disputed-chunk input shape), not Tier 3 (exercised court).
- **No mainnet, no external DA, no custody.** All evidence today is
fixture-backed or local-devnet-backed. No real external DA receipt, no public
testnet final settlement, no threshold-lock deployment is claimed.
- **No in-VM improvement over xuejie.** We reuse the replayer binary
unchanged. Our work is host-side orchestration and evidence, not VM or
program optimization.

## Reproducing the line end to end

The canonical path is the [Teeworlds end-to-end runbook](../tutorials/teeworlds-end-to-end.md):
clone xuejie's fork at the pinned commit, build the RISC-V replayer, run it
through Myelin's VM verifier, build the court bundle, and verify it. Every
step is reproducible and the measured values (`tape_bytes: 2162`,
`vm_cycles: 15,139,695`, `court_checks: 22`) are recorded in
[*Teeworlds reproducibility*](https://github.com/Myelin-Network/Myelin/blob/main/MYELIN_TEEWORLDS_REPRODUCIBILITY.md).

## Credit

The *Teeworlds on CKB* experiment, the RISC-V replayer, the fixture builder,
and the in-VM optimization work are entirely xuejie's. Myelin builds on top of
that artifact and would not be possible without it. This document exists to
make the lineage explicit and the boundary honest.

## Related

- [Claim ladder](../security/claim-ladder.md) — the Tier 0–3 boundary this
release sits within.
- [Concurrency plan](../operations/concurrency-optimization-plan.md) — the
CellDAG / parallel-verification path in detail.
- [Teeworlds end-to-end](../tutorials/teeworlds-end-to-end.md) — the
runbook.
- [*Teeworlds reproducibility*](https://github.com/Myelin-Network/Myelin/blob/main/MYELIN_TEEWORLDS_REPRODUCIBILITY.md) — measured values and provenance.
