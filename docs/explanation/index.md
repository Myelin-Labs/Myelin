# Explanation

This section is for the questions that don't fit a narrative
chapter: the **why** behind design choices, and the answers to
the questions that come up most often.

## Pages

<div class="grid cards" markdown>

-   [Architecture decisions](architecture-decisions.md)

    ---

    The why behind the key design choices: RISC-V, Molecule,
    static committee first, single-chunk court path, no
    permissionless validators today.

-   [FAQ](faq.md)

    ---

    Answers to the questions that come up most often, from
    "what's a Cell?" to "is Myelin production-ready?"

</div>

## What "explanation" means here

The Myelin docs have a **layered structure**:

- **Concepts** — the primitives (Cell, CKB-VM, Myelin).
- **Architecture** — how Myelin is built.
- **Interactions** — how Myelin talks to L1 / L2 / off-chain.
- **Operations** — how to run Myelin.
- **Security** — what Myelin proves.

The Explanation section sits **across** those layers. It's for
context that doesn't fit one place — the design rationale, the
common misconceptions, the answers that take more than a few
sentences.