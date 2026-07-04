# Patterns

The patterns section shows how different real-world use cases map
onto a Myelin session. Each pattern is a **shape** — a particular
configuration of the session primitives that suits a particular
class of application.

## Pages

<div class="grid cards" markdown>

-   [IoT metering](iot-metering.md)

    ---

    Gateway-aggregated sensor updates, settlement deltas as
    challengeable Cells. The canonical industrial-IoT pattern.

-   [RFQ / market-maker settlement](rfq-settlement.md)

    ---

    Off-chain quote negotiation, signed receipts, deterministic
    settlement rule, dispute path via cell projection.

-   [Streaming payments](streaming-payments.md)

    ---

    Bounded session per payer-payee pair, pre-funded capacity,
    deterministic close. The natural pairing with Fiber.

</div>

## Why patterns, not apps

Myelin doesn't ship applications. It ships a **runtime** that
applications can be built on. The patterns here are the
intermediate layer — common shapes that real applications take,
mapped onto Myelin's primitives.

If you're building an application that maps cleanly onto one of
these patterns, Myelin is structurally appropriate. If your
application needs a pattern that isn't here, see
[What is Myelin?](../concepts/what-is-myelin.md) for whether
Myelin fits at all.

## What all patterns share

Every pattern follows the same shape:

```text
asset custody       -> canonical CKB-style Cells
session entry       -> lock or commit Cells into a session
fast path           -> static-committee Myelin session runtime
DA path             -> publish chunk commitments
court path          -> one disputed chunk is CKB-VM-style verifiable
exit path           -> final state unlocks or materialises Cells
```

What differs is the *content* inside the fast path — the chunk
payload, the witnesses, the script group, the conflict domain
key. The patterns here show what those differences look like for
common cases.