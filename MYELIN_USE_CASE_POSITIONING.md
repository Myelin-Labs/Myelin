# Myelin Use-Case Positioning

> This document refines the high-frequency and IoT positioning of the
> standalone Myelin runtime. It builds on `MYELIN_SESSION_L2_PLAN.md`
> (which establishes Myelin as a CKB-isomorphic finite Cell session
> L2) and on the runtime-spine work in `0acc4b5`, which wires
> `myelin-mempool`, `myelin-state`, and both consensus engines into
> a single `myelin-cli runtime smoke` path.
>
> Two distinctions govern this document:
>
> 1. **Architecture fit** is a structural question about the protocol
>    surface (Cell model, deterministic VM, committee finality, CKB
>    projection, court path). It can be argued from design alone.
> 2. **Production evidence** is an empirical question about the
>    runtime, the workload, and the acceptance numbers. It cannot be
>    argued from design alone.
>
> Throughout this document, "suitable" / "fits" / "is appropriate" is
> a claim of type (1). "Validated" / "shown" / "measured" is a
> claim of type (2). The two are deliberately kept apart.

## 1. Product identity, restated

Myelin is:

> A CKB-isomorphic finite Cell session L2 with deterministic
> off-chain execution, committee-mediated finality, and a CKB
> projection path for disputed-chunk court verification.

It is a session ledger, not a matching engine and not a public
order book. Its state model is a finite Cell set with explicit
transaction-induced state-root transitions, not a global account
store. Its execution model is a deterministic VM (CKB-VM plus a
small set of Myelin-only syscall extensions), not a co-located
matching loop.

The system optimises for **bounded sessions that produce
challengeable settlement artifacts**, not for the
microsecond-scale market microstructure that public exchanges
serve.

## 2. What Myelin is structurally NOT suitable for

These are claims of type (1). They follow from the protocol
surface, not from missing benchmarks.

1. **Microstructure-level HFT matching.** Co-located, kernel-bypass
   or FPGA-mediated public order books with strict price-time
   priority and sub-millisecond matching loops. The required
   latency, ordering guarantees, and per-event hardware path are
   outside the protocol design.
2. **Global public order books.** The state model is a finite Cell
   set, the consensus is a closed committee or Tendermint-style
   BFT, and the dispute path is court-projected to CKB. None of
   those choices are aimed at global permissionless matching.
3. **Public permissionless validator network at day one.** Both
   supported consensus engines (StaticClosedCommittee, Tendermint)
   assume a known validator set. A permissionless entry path is
   not part of the design and would require additional work on
   staking, slashing, and identity.
4. **Unbounded MMO-style world state.** The Cell model is finite
   per session. Long-running worlds with continuous spatial state
   and free-form mutations are a poor match.
5. **Raw sensor firehose as individual Cells.** One device reading
   per Cell is structurally possible but economically
   indefensible: every Cell carries capacity overhead, and the
   storage cost of N devices × Hz × days is prohibitive.

## 3. What Myelin IS structurally suitable for

The protocol surface supports the following use cases. The list
is ordered by fit; the boundary between tiers is judgement, not
measurement.

### Tier 1 — best fit

Common shape: bounded session, pre-funded custody, many
off-chain updates, few on-chain settlements, clear dispute
chunks.

| Use case | Why it fits |
|---|---|
| Game sessions (current Teeworlds acceptance is the reference workload) | Bounded tick-driven session, deterministic replay, single-chunk dispute surface, CKB court path for disputed frames. |
| Industrial IoT metering | Bounded session per device fleet, gateway-aggregated updates, settlement deltas as challengeable cells. |
| RFQ / market-maker settlement sessions | Off-chain quote negotiation, signed receipts, deterministic settlement rule, dispute path via cell projection. |
| Streaming payments | Bounded session per payer-payee pair, pre-funded capacity, deterministic close. |
| AI agent service receipts | Bounded service session, signed task receipts, deterministic settlement per task. |

### Tier 2 — viable, but require additional infrastructure

These are structurally suitable but require work outside the
current Myelin surface (data availability, identity, gateway,
audit, dispute mechanism). They are real product opportunities,
not architectural claims.

| Use case | What is missing |
|---|---|
| Cross-organisation IoT | Cross-org identity, gateway federation model, dispute rule for sensor disagreements. |
| Batch auctions | Encrypted-order mechanism, batch-merkle witness format, anti-censorship rule. |
| Supply-chain receipts | Identity binding (org ↔ cell), revocation path, selective disclosure. |
| Usage-based billing | Metering standard, batch aggregation rule, dispute rule for off-by-one windows. |
| Small multiplayer tournament economy | Anti-collusion rule, prize custody, refund path on disputed match. |

### Tier 3 — explicitly out of scope

These are cases the protocol design does not aim at and where
adding Myelin would dilute the positioning:

- True HFT matching engine.
- Global public order book.
- Unbounded MMO world state.
- Raw sensor firehose as individual Cells.
- Public permissionless validator network at day one.

## 4. Cell-modeling guidance for financial use

A common objection to using a Cell model for finance is "every
order would be a Cell, so hash / capacity / cell overhead is
unaffordable." This is a real concern if the modeling naively
maps order book events to Cells, but it is not a property of
the model — it is a property of the mapping.

The Cell model supports at least the following non-naive
financial shapes:

| Cell type | Purpose | Cost shape |
|---|---|---|
| `OrderCell` | One order = one Cell. Naive; rarely optimal. | Per-order overhead. |
| `OrderBatchCell` | Many orders in one Cell, with merkle / MMR commitments in the witness. | Per-batch overhead. |
| `OrderBookShardCell` | One Cell per (instrument, price band, session). State transitions reflect depth changes. | Per-shard overhead. |
| `FillReceiptCell` | Only the executed fills are typed as Cells. Orders are off-chain signed messages that reference a FillReceiptCell. | Per-fill overhead. |
| `NetSettlementCell` | Only the netted settlement is on-chain. The off-chain session produces a signed net result that consumes pre-funded custody Cells. | Per-session overhead. |

The design rule that follows is:

> Cell modeling in Myelin should be biased towards settlement,
> receipt, and netting artifacts, not towards replaying an
> exchange order book.

This rule is a design recommendation, not a runtime constraint.
The runtime will accept any well-typed Cell; the recommendation
is about which Cell shapes are economical.

## 5. IoT positioning: gateway aggregation, not light client

The IoT fit is real, but the device-side architecture should be
chosen deliberately.

### 5.1 Roles

The intended Myelin IoT profile is:

```text
device       = signer + data source (sensor / meter / actuator)
edge gateway = aggregator + submitter + session participant
myelin node  = verifier + finaliser + state root publisher
CKB          = long-term anchor + court path
```

The device does **not** verify the Myelin state root in the
first profile. It only:

- signs the reading,
- includes a monotonic counter,
- includes a timestamp / epoch,
- binds the reading to a session / gateway id.

The gateway aggregates N readings, computes a deterministic
aggregation rule, signs the aggregate, and produces one
`TelemetryBatchCell` per epoch.

### 5.2 What the Cell looks like

A `TelemetryBatchCell` witness carries:

```text
device_signatures    : set of signatures over (counter, timestamp, value)
timestamp_window     : (epoch_start, epoch_end)
gateway_signature    : signature over the batch commitment
reading_root         : merkle / MMR / accumulator root over the readings
aggregation_rule     : identifier + parameters (sum, mean, threshold, etc.)
settlement_delta     : capacity / token movement produced by this batch
```

The Myelin verifier checks signatures, recomputes the
`reading_root` from the supplied readings, and confirms the
`aggregation_rule` was applied. The CKB projection path then
publishes the batch commitment as a court-verifiable artifact.

### 5.3 Why not the light client first

A `no_std` / C light client SDK for Myelin is a long-term
direction, but not the first step. The first step is the
gateway-aggregation profile, because:

- It exercises the same consensus, state, and CKB projection
  paths that the production gate already validates.
- It produces a real dispute chunk (the batch) that can be
  challenged at the CKB court path.
- It does not require a new client surface; the device only
  needs a signing key, which it already has for firmware
  attestation or TLS client auth.

A light client is a later optimisation for cases where devices
must verify without trusting the gateway.

## 6. Production evidence: what is shown, what is not

The boundary between architecture fit and production evidence
should be drawn explicitly.

### 6.1 What is shown by the current production gate

The current production gate (`scripts/myelin_production_gate.sh`)
demonstrates:

- The runtime spine works end-to-end: `mempool -> state -> block
  -> finality -> report` is exercised by `myelin-cli runtime smoke
  --consensus {static-closed-committee,tendermint}`.
- The first Session L2 spine works end-to-end:
  `session open-fixture -> commit-fixture -> court-bundle ->
  verify-court-bundle` runs for both static-closed-committee and
  Tendermint. The gate asserts that the session id, CellTx
  commitments, scheduler commitment, and state roots are
  consensus-independent while finality/block domains remain
  separated. The CLI also supports descriptor-driven `session open`
  and non-zero chunk `session commit` paths, covered by unit tests.
- The CKB projection path produces `semantic_profile =
  "ckb-compatible"` for the Teeworlds workload.
- The Teeworlds acceptance shows 16 court-bundle data-binding
  checks, 15,139,695 VM cycles, and a single 2162-byte tape
  chunk (one block, one chunk).
- The state root mutates between `state_root_before` and
  `state_root_after` on the smoke path; the two engines agree on
  the same `cell_tx_id`, `cell_wtxid`, and state roots, and
  differ only in the `certificate_hash` (signature domain).

### 6.2 What is NOT yet shown

The following production claims are not yet backed by a
release-gate run:

- Throughput numbers under sustained load.
- Latency numbers for either consensus engine.
- An IoT-shaped acceptance (gateway aggregation, batch
  verification, gateway dispute path).
- An RFQ / netting-shape acceptance (signed quote,
  net-settlement cell, dispute path).
- A real non-Teeworlds workload with external input data.

### 6.3 What is and is not safe to put in marketing

- **Safe to claim:** "Myelin is a CKB-isomorphic finite Cell
  session L2 with deterministic off-chain execution, committee
  finality, and a CKB projection path. The runtime spine is
  exercised by the production gate; the built-in session fixture
  proves the protocol spine; the Teeworlds workload is the
  reference external acceptance."
- **Not safe to claim:** specific throughput numbers, specific
  latency numbers, "supports IoT at scale", "supports
  high-frequency finance", "production-ready" for any
  vertical beyond the Teeworlds reference.

## 7. Recommended acceptance plan

The next acceptance work follows from the positioning.

### 7.1 Runtime spine acceptance (already done)

`myelin-cli runtime smoke --consensus {static-closed-committee,
tendermint}` is now a release-gate step. It locks down:

- `myelin-mempool` admits the CellTx, `pool_size_after = 1`.
- `myelin-state` applies the CellTx, the state root mutates.
- Both engines agree on txid / wtxid / state roots and differ
  on the certificate hash.
- The CKB projection report is structurally present.

### 7.2 Teeworlds acceptance (already done)

`scripts/myelin_teeworlds_acceptance.sh` plus
`scripts/build_myelin_teeworlds_repro.py` lock down the
game-session reference workload: tape bytes 2162, VM cycles
15,139,695, court-bundle 16 checks, semantic profile
`ckb-compatible`.

### 7.3 IoT metering acceptance (proposed)

A small, deterministic IoT acceptance that does **not** require
a light client:

- 100 virtual sensors, each emitting 1 reading per second.
- 10 epochs, 10 seconds per epoch, 1,000 readings per epoch.
- One edge gateway per 100 sensors, aggregating the readings
  into one `TelemetryBatchCell` per epoch.
- The Myelin runtime verifies device signatures, recomputes
  the `reading_root`, confirms the `aggregation_rule`, and
  finalises the batch.
- The CKB projection path emits a court bundle for the batch.
- The production gate asserts: signatures valid, root matches,
  settlement delta matches the rule, court bundle verifies.

The acceptance is small enough to run in CI and is the smallest
credible evidence that the IoT positioning is real, not just a
slide.

### 7.4 Financial settlement acceptance (proposed, later)

A second acceptance, to be designed after the IoT acceptance is
green:

- An RFQ session: 1 client, 2 market makers, N rounds of
  off-chain signed quotes.
- One `NetSettlementCell` per session, derived deterministically
  from the off-chain quote log.
- CKB projection emits a court bundle for the net settlement.
- The production gate asserts: signed-quote count matches,
  net matches the deterministic settlement rule, court bundle
  verifies.

This acceptance would back the "high-frequency financial
settlement" positioning with evidence, not just architecture
fit.

## 8. Summary

Myelin's positioning is best stated as:

> Court-verifiable high-frequency sessions: games, metering, and
> financial settlement.

Or in one sentence:

> Do not build a matching engine; build high-frequency
> financial settlement. Do not build a raw IoT firehose; build
> aggregated, challengeable telemetry.

The architecture supports the first half of each clause
already. The second half of each clause (financial settlement
session runtime, IoT gateway aggregation runtime) is what the
proposed acceptances exist to demonstrate.

Production claims should be scoped to what the production gate
shows. Architecture-fit claims can be broader, as long as they
are kept separate from production-evidence claims.
