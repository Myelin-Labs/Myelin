# Getting started

This section takes you from "I've heard of Myelin" to "I just ran a
CellTx through it and got a CKB-projected report." It assumes nothing
about prior CKB knowledge — but if you have worked with CKB before, you
can probably skip straight to [Anatomy of a Myelin CellTx](anatomy.md).

## Path through this section

<div class="grid cards" markdown>

-   [Install the toolchain](install.md)

    ---

    Rust toolchain, the `myelin-cli` workspace, and (optionally) a local
    CKB devnet for the projection smoke test.

-   [First run](first-run.md)

    ---

    The shortest path to a real `CellTx → execution report → CKB
    projection report` JSON on disk.

-   [Anatomy of a Myelin CellTx](anatomy.md)

    ---

    What is actually inside a CellTx, why it has both Myelin fields and
    CKB fields, and what a "projection" really does to it.

</div>

## Prerequisites at a glance

You will need:

- A Rust toolchain that matches the workspace `Cargo.toml` (stable, plus
  the `riscv64imac-unknown-none-elf` target if you want to compile CKB
  scripts locally).
- Python 3 — used by validation scripts under `scripts/`.
- A few gigabytes of free disk for build artefacts and the optional
  local CKB devnet.
- *Optional*: a local CKB devnet (OffCKB or `ckb init --testnet`) if you
  want to run the live carrier submission smoke.

## The 60-second mental model

Myelin is built on **five primitives** and one **output**:

| Primitive | What it is |
| --- | --- |
| **Cell** | A unit of state — capacity, optional data, lock script, optional type script. |
| **CellTx** | A transition — consumes Cells, creates Cells, carries witnesses and dep references. |
| **CellDAG** | A static conflict graph the scheduler uses to admit and parallelise CellTxs. |
| **State root** | The 32-byte commitment to the live Cell set before and after each chunk. |
| **Evidence bundle** | Everything needed to reconstruct or dispute a chunk — payload, projection, DA, court, settlement. |
| **Output** | A finalised Myelin block with a committee certificate over the deterministic block hash. |

Everything else in this documentation is detail about how these five
primitives are produced, ordered, projected, and proven.