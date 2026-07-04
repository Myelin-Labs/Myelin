# Tutorials

Hands-on walkthroughs. Each tutorial takes you from "I've read the
concepts" to "I've actually run this on a real session and have a
report on disk."

## Pages

<div class="grid cards" markdown>

-   [Your first CellTx](first-celltx.md)

    ---

    Build, verify, and project a real CellTx through Myelin with
    step-by-step Rust snippets. The shortest path from zero to a
    `semantic_profile = "ckb-compatible"` report.

-   [Teeworlds end-to-end](teeworlds-end-to-end.md)

    ---

    The full reference workload: scripted tape → CKB replayer →
    chunk → projection → court bundle → settlement. The path that
    exercises every layer in Myelin.

</div>

## What "tutorial" means here

These pages are not API reference. They're **stories with code** —
narrative walkthroughs that explain *why* each step exists, not
just *what* to type. If you want a flag reference, see
[CLI reference](../operations/cli.md).

## What's assumed

- You've installed the toolchain per
  [Install the toolchain](../getting-started/install.md).
- You know what a Cell is (see [What is CKB?](../concepts/what-is-ckb.md)).
- You have a few minutes of focused attention per tutorial.

The tutorials are designed to be runnable on a laptop with no
external dependencies beyond the Myelin workspace.