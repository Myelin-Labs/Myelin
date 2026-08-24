---
title: Myelin
description: A CKB-aligned off-chain session runtime for finite Cell execution
hide:
  - navigation
---

<div class="hero" markdown>

# Myelin

**A CKB-aligned session runtime for finite Cell execution.**

Myelin runs high-throughput Cell transitions off-chain, keeps them finite and
typed, and emits canonical CKB wire data plus staged evidence for context,
script, node, commitment, and configured-depth checks. A future CKB court may
consume a disputed-chunk bundle; no such court is deployed today.

[Get started :material-rocket-launch:](getting-started/index.md){ .md-button .md-button--primary }
[Read the architecture :material-graph-outline:](architecture/overview.md){ .md-button }
[GitHub :fontawesome-brands-github:](https://github.com/Myelin-Labs/Myelin){ .md-button }

</div>

<div class="grid cards" markdown>

-   :fontawesome-brands-github:{ .lg .middle } **Repository**

    ---

    [`Myelin-Labs/Myelin`](https://github.com/Myelin-Labs/Myelin) — source, issues, CI, releases.

-   :material-book-open-page-variant-outline:{ .lg .middle } **License**

    ---

    MIT. See [`LICENSE`](https://github.com/Myelin-Labs/Myelin/blob/main/LICENSE) for the full text.

-   :material-bug-outline:{ .lg .middle } **Report an issue**

    ---

    Found a bug or a docs error? [Open an issue](https://github.com/Myelin-Labs/Myelin/issues/new) — please include the page URL and the exact command / code that misbehaved.

-   :material-rocket-launch-outline:{ .lg .middle } **Deploy this site**

    ---

    This site auto-deploys to GitHub Pages from `main` via `.github/workflows/pages.yml`. Configure in **Settings → Pages → Source: GitHub Actions**.

</div>

## What Myelin actually is

Myelin is **not** a CKB full-node fork, **not** a new L1, and **not yet** a
finished permissionless L2. It is an off-chain finite-Cell runtime for testing
deterministic execution, durable session history, closed-validator finality,
and evidence paths that can be checked against CKB.

<div class="grid cards" markdown>

-   :material-cube-outline:{ .lg .middle } **Cell-native state**

    ---

    State is a finite Cell set. There is no global account store and no
    mutable contract storage hidden behind an address. Every transition
    consumes and creates Cells.

-   :material-cpu-64-bit:{ .lg .middle } **Deterministic CKB-VM execution**

    ---

    Session and court paths run script groups under `CkbStrict`. Each group
    gets its own CKB-VM instance, while the transaction shares one cycle
    budget.

-   :material-graph:{ .lg .middle } **Typed conflict scheduling**

    ---

    The CellDAG scheduler uses typed conflict hashes and read/write
    domains to admit transactions, parallelise independent ones, and
    reject anything that cannot be reasoned about statically.

-   :material-shield-check-outline:{ .lg .middle } **CKB-style projection**

    ---

    The pure projector stops at canonical Molecule bytes and the CKB raw
    transaction hash. Higher stages require linked adapter receipts for the
    exact transaction.

-   :material-scale-balance:{ .lg .middle } **Single-chunk court path**

    ---

    Myelin packages one disputed chunk for verification. The on-chain court
    verifier and its economics are future work.

-   :material-timeline-clock-outline:{ .lg .middle } **Continuous operation**

    ---

    `Instant`, `Interval`, lazy `Open`, and `Never` close bounded batches.
    One candidate advances from the durable head at a time, with checkpoint
    recovery and a transactional outbox.

</div>

## How it relates to CKB

CKB is the **semantic reference** for Myelin. Same vocabulary (Cell, CellTx,
witness, dep group, script group), same execution environment (CKB-VM +
RISC-V), and the same projection layer at the boundary. The differences are
about **where** work happens, not **what** work means:

| Aspect | CKB (L1) | Myelin (off-chain session) |
| --- | --- | --- |
| Where state lives | Every full node | Finite session set |
| Block finality | Nakamoto PoW consensus | Selectable: static committee, rotating PoA, or finite-session Tendermint |
| Throughput target | ~1 block / tens of seconds | Many chunks / second inside one session |
| Execution | CKB-VM, fully on-chain | CKB-VM under `CkbStrict` for session/court paths, off-chain |
| Dispute path | Replay on chain | Single-chunk court bundle; L1 court deployment is future work |
| Asset custody | CKB Cells natively | Optional funding attachment after exact finalised-CKB receipt verification |

## A first look at the runtime spine

```mermaid
%%{init: {
  "theme": "base",
  "themeVariables": {
    "primaryColor": "#A5B4FC",
    "primaryTextColor": "#1E293B",
    "primaryBorderColor": "#4F46E5",
    "lineColor": "#6366F1",
    "secondaryColor": "#C7D2FE",
    "tertiaryColor": "#C7D2FE",
    "fontFamily": "Inter, system-ui, sans-serif",
    "fontSize": "14px"
  },
  "flowchart": { "curve": "basis", "padding": 12 }
}}%%
flowchart LR
    A["CellScript source"]:::source
    B["typed-cell metadata<br/>+ VM artefact"]:::artefact
    C["CellTx<br/>(Myelin)"]:::tx
    D["Producer<br/>one reserved candidate"]:::sched
    E["CellDAG + CKB-VM<br/>verification"]:::vm
    F["Genesis-bound<br/>finality"]:::state
    G["Atomic block · latest checkpoint<br/>· head · outbox"]:::state
    H["Evidence bundle<br/>(projection, DA,<br/>court, settle)"]:::evidence

    A --> B --> C --> D --> E --> F --> G --> H
    G --> C

    classDef source   fill:#A5B4FC,stroke:#4F46E5,color:#1E293B;
    classDef artefact fill:#C7D2FE,stroke:#6366F1,color:#1E293B;
    classDef tx       fill:#C7D2FE,stroke:#4F46E5,color:#1E293B;
    classDef sched    fill:#C7D2FE,stroke:#6366F1,color:#1E293B;
    classDef vm       fill:#A5B4FC,stroke:#4F46E5,color:#1E293B;
    classDef state    fill:#C7D2FE,stroke:#6366F1,color:#1E293B;
    classDef evidence fill:#C7D2FE,stroke:#7C3AED,color:#1E293B;
```

The spine is implemented by the external CellScript adapter plus
`myelin-exec`, `myelin-state`, `myelin-mempool`, `myelin-consensus`,
`myelin-session`, `myelin-session-producer`, `myelin-session-runtime`, the
RocksDB store, and the evidence adapters. The next pages break those parts
apart.

## Where to go next

<div class="grid cards" markdown>

-   :material-book-open-variant: **New to Myelin?**

    ---

    Read [What is CKB?](concepts/what-is-ckb.md) and [What is Myelin?](concepts/what-is-myelin.md) first. They assume nothing.

-   :material-tools: **Want to run it?**

    ---

    Skip to [Install the toolchain](getting-started/install.md), then
    [First run](getting-started/first-run.md) for the shortest path to a
    CKB-projected CellTx report.

-   :material-graph-outline: **Want to understand the design?**

    ---

    The [System overview](architecture/overview.md) is the architectural
    truth of the project. Pair it with the [L1 / L2 / off-chain
    interactions](interactions/l1-l2-offchain.md) diagram.

-   :material-timeline-check-outline: **Building a continuous service?**

    ---

    Start with the [session lifecycle](interactions/session-flow.md), then see
    how the [independent Veloren research fork](integrations/veloren-research-fork.md)
    journals and recovers application events.

-   :material-shield-alert-outline: **Skeptical about the security claim?**

    ---

    Read the [Claim ladder](security/claim-ladder.md) first. It lists the
    receipt required for every evidence stage and the claims that stage does
    not support.

</div>
