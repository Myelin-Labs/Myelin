# Pattern: IoT metering

**Shape.** A bounded session per device fleet. A gateway collects
sensor readings, aggregates them into chunks, and submits each
chunk as a CellTx. Settlement is the total at session close.

## Why this fits Myelin

Industrial IoT metering has a very specific shape:

```text
- bounded session per fleet (one metering period, e.g. a day)
- many off-chain updates (sensor readings every N seconds)
- few on-chain settlements (one settlement CellTx at session close)
- clear dispute chunks (an aggregated hour, a day, a week)
```

That shape matches Myelin's design:

- **Bounded session** — Myelin sessions have an open and a close.
- **Many off-chain updates** — the fast path is exactly designed for
  this throughput profile.
- **Few on-chain settlements** — only the open and close touch L1.
- **Clear dispute chunks** — the gateway's aggregation is
  deterministic; the chunks are verifiable.

It does **not** fit:

- Raw sensor firehose as individual Cells. The capacity overhead
  per Cell would be prohibitive. Aggregation is the answer.

## The session shape

```text
session_id          -> fleet_id || metering_period_start_ms
participants        -> [gateway, customer]
escrow              -> pre-paid capacity Cells from the customer
max_chunk_bytes     -> ~256 KB (one hour of aggregated readings)
max_cycles          -> VM budget per aggregation script
```

A typical session:

```text
open at T0 (customer locks, e.g., 1000 CKB worth of capacity)
chunk 0..N-1 between T0 and T1 (gateway submits aggregated readings)
close at T1 (settlement CellTx transfers capacity per the rule)
```

## What a chunk looks like

Each chunk represents one aggregation window. Inside the chunk:

```text
witness[0]          -> signature from the gateway
witness[1]          -> aggregated reading batch
witness[2]          -> prior chunk's state root (chained)
witness[3]          -> per-device nonce list (anti-replay)
```

The aggregation script:

1. Verifies the gateway signature.
2. Replays the aggregated reading batch through the metering rule.
3. Updates the session state root.
4. Emits a `MyelinExecutionReport` with the per-device totals.

## Conflict domain keying

The conflict key for an IoT session looks like:

```text
conflict_key("iot/fleet/{fleet_id}/chunk/{index}")
```

This guarantees that two gateways cannot double-write the same
chunk, and that the customer cannot submit conflicting settlement
intents for the same chunk.

## The dispute path

If the customer disputes a chunk:

```text
dispute   -> court-bundle for chunk K
replay    -> Myelin VM probe runs the same aggregation script
compare   -> chunk_K.state_root_after matches the disputed state root?
verdict   -> accept (gateway was right) or slash (gateway was wrong)
```

The aggregation script is deterministic — given the same witness
batch, it always produces the same state root. This is what makes
single-chunk verification possible.

## A reference implementation sketch

```rust
use myelin_exec::celltx::{CellTx, CellTxBuilder};
use myelin_exec::witness::{Witness, WitnessLayout};

// The gateway builds a chunk CellTx
fn build_chunk(fleet_id: [u8; 32], index: u64, batch: Vec<u8>, signature: [u8; 64]) -> CellTx {
    CellTxBuilder::new()
        .witnesses(vec![
            Witness::signature(signature),
            Witness::data(batch),                       // aggregated readings
            Witness::data(prev_chunk_state_root),       // chain
            Witness::data(per_device_nonces),           // anti-replay
        ])
        .cell_deps(vec![metering_script_dep()])
        .build()
        .expect("iot metering chunk CellTx build")
}
```

The `metering_script_dep()` returns the cell_dep reference for
the aggregation script — a CKB type script that runs in Myelin's
verifier with `vm_profile = "ckb-strict-basic"`.

## Anti-replay per device

Each device has a nonce that's incremented on every accepted
reading. The aggregation script checks:

```text
for each (device_id, reading) in batch:
    if reading.nonce != device_state[device_id].last_nonce + 1:
        reject("stale reading")
    device_state[device_id].last_nonce = reading.nonce
    device_state[device_id].total += reading.value
```

This prevents a malicious gateway from re-submitting an old
reading batch.

## Where the boundary is honest

Myelin can produce:

- ✅ A gateway aggregation script that runs deterministically.
- ✅ Per-chunk CellTx reports with projection status.
- ✅ A court bundle for any disputed chunk.

Myelin does **not** ship:

- The gateway runtime — that's a separate component.
- The metering standard — Myelin supports whatever rule you
  encode in the aggregation script.
- The device attestation — that's a separate identity concern.

If you have a specific metering standard in mind, encode it in
the aggregation script and the per-chunk CellTx will follow.

## What this looks like in production evidence

For a typical 1000-device fleet metering over a one-day session:

```text
session duration         : 24 hours
chunks per session       : 24 (one per hour)
cells per chunk          : ~1000 device updates + 1 settlement delta
chunks to L1 (DA)        : 24 (one per hour)
settlement to L1          : 1 (at close)
```

The L1 footprint is small — one open, 24 DA anchors, one close.
The off-chain workload is large. That's the Myelin sweet spot.

## Where to go next

- [Pattern: streaming payments](streaming-payments.md) — similar
  shape, different content.
- [Session lifecycle](../interactions/session-flow.md) — the
  session primitives in detail.
- [What is Myelin?](../concepts/what-is-myelin.md) — the broader
  positioning.