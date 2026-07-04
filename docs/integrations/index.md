# Integrations

Myelin doesn't live in isolation. This section covers the
integrations that matter today — and the integrations that are
designed for but not yet wired.

## Pages

<div class="grid cards" markdown>

-   [Fiber Network bridge](fiber.md)

    ---

    The recommended boundary for connecting Myelin to Fiber's
    payment-channel network: external funding, payment-hash
    bridges, compact commitment metadata.

</div>

## What "integration" means here

An integration is a **boundary component** — code that lets Myelin
talk to another system without leaking Myelin's runtime details
into that system, or vice versa.

For Fiber specifically, the integration is:

- A standalone bridge controller.
- Calls Myelin CLI/session APIs to produce deterministic session
  artefacts.
- Calls Fiber JSON-RPC APIs to open channels, submit funding
  transactions, create invoices, settle invoices, send payments.
- Maintains an explicit mapping between Myelin session IDs and
  Fiber channel IDs.
- Carries only compact commitments through Fiber payment metadata.

## What isn't here

- **A wallet integration.** Myelin doesn't manage user keys; that
  belongs on the L1 side (CCC, etc.).
- **An explorer integration.** The CKB explorer handles L1
  inspection; Myelin produces reports on disk.
- **A smart-contract dApp integration.** Myelin isn't a
  smart-contract platform; see
  [What is Myelin?](../concepts/what-is-myelin.md) for the
  positioning.