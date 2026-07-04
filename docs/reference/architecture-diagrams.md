# Diagram gallery

A copy-paste-friendly collection of the mermaid diagrams used
throughout the Myelin documentation. Each diagram is shown with
its full source, including the CKB-teal/navy theme override, so
you can lift it directly into a slide deck or external doc.

> [!TIP]
> When you copy a diagram, **also copy the `%%{init:{...}}%%`
> block** at the top. Without it, the diagram will use mermaid's
> default theme instead of the CKB palette.

## 1 — Runtime spine (home page)

The full CellScript → CellTx → CellDAG → VM → state root →
evidence pipeline.

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
    "fontFamily": "Inter, system-ui, sans-serif"
  }
}}%%
flowchart LR
    A["CellScript source"]:::source
    B["typed-cell metadata<br/>+ VM artefact"]:::artefact
    C["CellTx<br/>(Myelin)"]:::tx
    D["CellDAG<br/>scheduler"]:::sched
    E["Deterministic<br/>VM verification"]:::vm
    F["Session Cell<br/>state root"]:::state
    G["Evidence bundle<br/>(projection, DA,<br/>court, settle)"]:::evidence

    A --> B --> C --> D --> E --> F --> G
    F --> C

    classDef source   fill:#A5B4FC,stroke:#4F46E5,color:#1E293B;
    classDef artefact fill:#C7D2FE,stroke:#6366F1,color:#1E293B;
    classDef tx       fill:#C7D2FE,stroke:#4F46E5,color:#1E293B;
    classDef sched    fill:#C7D2FE,stroke:#6366F1,color:#1E293B;
    classDef vm       fill:#A5B4FC,stroke:#4F46E5,color:#1E293B;
    classDef state    fill:#C7D2FE,stroke:#6366F1,color:#1E293B;
    classDef evidence fill:#C7D2FE,stroke:#7C3AED,color:#1E293B;
```

## 2 — Cell Model consumption / creation

How a CKB transaction consumes and creates Cells.

```mermaid
%%{init: {
  "theme": "base",
  "themeVariables": {
    "primaryColor": "#A5B4FC",
    "primaryTextColor": "#1E293B",
    "primaryBorderColor": "#4F46E5",
    "lineColor": "#6366F1",
    "secondaryColor": "#C7D2FE",
    "tertiaryColor": "#C7D2FE"
  }
}}%%
flowchart LR
    A1["Cell A<br/>(live)"]:::live
    A2["Cell B<br/>(live)"]:::live
    A3["Cell C<br/>(live)"]:::live

    TX["CKB Transaction<br/>inputs: A, B, C<br/>outputs: D, E"]:::tx

    D["Cell D<br/>(new)"]:::new
    E["Cell E<br/>(new)"]:::new

    A1 -->|consume| TX
    A2 -->|consume| TX
    A3 -->|consume| TX
    TX -->|create| D
    TX -->|create| E

    classDef live fill:#C7D2FE,stroke:#D97706,color:#1E293B;
    classDef new  fill:#A5B4FC,stroke:#4F46E5,color:#1E293B;
    classDef tx   fill:#C7D2FE,stroke:#7C3AED,color:#1E293B;
```

## 3 — Three-layer model

The complete L1 / L2 / off-chain picture.

```mermaid
%%{init: {
  "theme": "base",
  "themeVariables": {
    "primaryColor": "#A5B4FC",
    "primaryTextColor": "#1E293B",
    "primaryBorderColor": "#4F46E5",
    "lineColor": "#6366F1",
    "secondaryColor": "#C7D2FE",
    "tertiaryColor": "#C7D2FE"
  }
}}%%
flowchart TB
    subgraph OFF["Off-chain"]
        P["Producer"]:::off
        W["Witnesses"]:::off
        D["DA store +<br/>external DA receipt"]:::off
    end
    subgraph L2["L2 — Myelin"]
        M["Mempool"]:::l2
        S["CellDAG<br/>scheduler"]:::l2
        V["CKB-VM-style<br/>verifier"]:::l2
        C["Committee<br/>certificate"]:::l2
    end
    subgraph L1["L1 — CKB"]
        A["Asset custody<br/>Cells"]:::l1
        E["DA anchor CellTx"]:::l1
        S2["Settlement CellTx"]:::l1
        CT["Court verifier<br/>(future)"]:::l1
    end

    P --> W --> M --> S --> V --> C
    V --> D
    C --> E
    C --> S2
    A -.lock.-> M
    S2 -.settle.-> A
    E -.fetch.-> CT
    S2 -.dispute.-> CT

    classDef off fill:#C7D2FE,stroke:#D97706,color:#1E293B;
    classDef l2  fill:#A5B4FC,stroke:#4F46E5,color:#1E293B;
    classDef l1  fill:#C7D2FE,stroke:#7C3AED,color:#1E293B;
```

## 4 — CellDAG with dependencies

How two CellTxs become dependent, conflicting, or parallel.

```mermaid
%%{init: {
  "theme": "base",
  "themeVariables": {
    "primaryColor": "#A5B4FC",
    "primaryTextColor": "#1E293B",
    "primaryBorderColor": "#4F46E5",
    "lineColor": "#6366F1",
    "secondaryColor": "#C7D2FE",
    "tertiaryColor": "#C7D2FE"
  }
}}%%
flowchart TB
    A["CellTx A"]:::a
    B["CellTx B"]:::b
    C["CellTx C"]:::a
    D["CellTx D"]:::b
    E["CellTx E"]:::a
    F["CellTx F"]:::c

    A -->|x1 read| B
    B -->|y1 written| F
    B -->|y1 read| D
    C -->|x2 read| E
    E -->|y2 consumed| F

    classDef a fill:#C7D2FE,stroke:#6366F1,color:#1E293B;
    classDef b fill:#A5B4FC,stroke:#4F46E5,color:#1E293B;
    classDef c fill:#C7D2FE,stroke:#7C3AED,color:#1E293B;
```

## 5 — DA ladder

The four readiness levels.

```mermaid
%%{init: {
  "theme": "base",
  "themeVariables": {
    "primaryColor": "#A5B4FC",
    "primaryTextColor": "#1E293B",
    "primaryBorderColor": "#4F46E5",
    "lineColor": "#6366F1",
    "secondaryColor": "#C7D2FE",
    "tertiaryColor": "#C7D2FE"
  }
}}%%
flowchart TB
    L1["local_only"]:::r1
    L2["testnet_beta_ready"]:::r2
    L3["production_ready"]:::r3
    L4["l1_da_published"]:::r4

    L1 --> L2 --> L3 --> L4

    classDef r1 fill:#C7D2FE,stroke:#6366F1,color:#1E293B;
    classDef r2 fill:#A5B4FC,stroke:#4F46E5,color:#1E293B;
    classDef r3 fill:#C7D2FE,stroke:#7C3AED,color:#1E293B;
    classDef r4 fill:#C7D2FE,stroke:#7C3AED,color:#1E293B;
```

## 6 — Five-step readiness chain

Context → economics → inclusion → stability → finality.

```mermaid
%%{init: {
  "theme": "base",
  "themeVariables": {
    "primaryColor": "#A5B4FC",
    "primaryTextColor": "#1E293B",
    "primaryBorderColor": "#4F46E5",
    "lineColor": "#6366F1",
    "secondaryColor": "#C7D2FE",
    "tertiaryColor": "#C7D2FE"
  }
}}%%
flowchart LR
    A["Submit"]:::in
    B["Context"]:::step
    C["Economics"]:::step
    D["Inclusion"]:::step
    E["Stability"]:::step
    F["Finality"]:::step
    G["Readiness"]:::out

    A --> B --> C --> D --> E --> F --> G

    classDef in   fill:#C7D2FE,stroke:#6366F1,color:#1E293B;
    classDef step fill:#A5B4FC,stroke:#4F46E5,color:#1E293B;
    classDef out  fill:#C7D2FE,stroke:#7C3AED,color:#1E293B;
```

## 7 — Claim ladder

The four-tier claim ladder.

```mermaid
%%{init: {
  "theme": "base",
  "themeVariables": {
    "primaryColor": "#A5B4FC",
    "primaryTextColor": "#1E293B",
    "primaryBorderColor": "#4F46E5",
    "lineColor": "#6366F1",
    "secondaryColor": "#C7D2FE",
    "tertiaryColor": "#C7D2FE"
  }
}}%%
flowchart TB
    T0["Tier 0<br/>Designed to stay close to CKB semantics"]:::t0
    T1["Tier 1<br/>Projectable into CKB-style transaction/context"]:::t1
    T2["Tier 2<br/>Executable disputed-chunk input shape"]:::t2
    T3["Tier 3<br/>CKB-aligned adjudication path"]:::t3

    T0 --> T1 --> T2 --> T3

    classDef t0 fill:#C7D2FE,stroke:#6366F1,color:#1E293B;
    classDef t1 fill:#A5B4FC,stroke:#4F46E5,color:#1E293B;
    classDef t2 fill:#C7D2FE,stroke:#7C3AED,color:#1E293B;
    classDef t3 fill:#C7D2FE,stroke:#7C3AED,color:#1E293B;
```

## 8 — Use-case tiers

What's in scope, viable, and out of scope for Myelin.

```mermaid
%%{init: {
  "theme": "base",
  "themeVariables": {
    "primaryColor": "#A5B4FC",
    "primaryTextColor": "#1E293B",
    "primaryBorderColor": "#4F46E5",
    "lineColor": "#6366F1",
    "secondaryColor": "#C7D2FE",
    "tertiaryColor": "#C7D2FE"
  }
}}%%
flowchart LR
    subgraph IN["In scope (Tier 1)"]
        I1["Game sessions<br/>(Teeworlds)"]:::ok
        I2["Industrial IoT metering"]:::ok
        I3["RFQ / market-maker<br/>settlement sessions"]:::ok
        I4["Streaming payments"]:::ok
        I5["AI agent service receipts"]:::ok
    end
    subgraph VIA["Viable (Tier 2 — needs more)"]
        V1["Cross-org IoT"]:::warn
        V2["Batch auctions"]:::warn
        V3["Supply-chain receipts"]:::warn
        V4["Usage-based billing"]:::warn
        V5["Small MP tournament economy"]:::warn
    end
    subgraph OUT["Out of scope (Tier 3)"]
        O1["HFT matching engine"]:::no
        O2["Global public order book"]:::no
        O3["Unbounded MMO world state"]:::no
        O4["Raw sensor firehose as Cells"]:::no
        O5["Day-1 permissionless validator network"]:::no
    end
    classDef ok  fill:#C7D2FE,stroke:#7C3AED,color:#1E293B;
    classDef warn fill:#A5B4FC,stroke:#D97706,color:#1E293B;
    classDef no  fill:#C7D2FE,stroke:#DC2626,color:#1E293B;
```

## 9 — Session lifecycle timeline

Open → chunks → DA → settlement → L1 close.

```mermaid
%%{init: {
  "theme": "base",
  "themeVariables": {
    "primaryColor": "#A5B4FC",
    "primaryTextColor": "#1E293B",
    "primaryBorderColor": "#4F46E5",
    "lineColor": "#6366F1",
    "secondaryColor": "#C7D2FE",
    "tertiaryColor": "#C7D2FE"
  }
}}%%
gantt
    title Session lifecycle
    dateFormat X
    axisFormat %s

    section L1 (CKB)
    Asset custody lock    :l1a, 0, 1
    Final settlement      :l1b, 14, 1

    section L2 (Myelin)
    Session open          :l2a, 1, 1
    Chunk 0 (fast path)   :l2b, 2, 3
    Chunk 1 (fast path)   :l2c, 5, 3
    Chunk 2 (fast path)   :l2d, 8, 3
    DA anchor package     :l2e, 11, 1
    Settlement intent     :l2f, 12, 1
    Settlement package    :l2g, 13, 1

    section Off-chain
    Witnesses             :offa, 0, 14
    DA store sealing      :offb, 2, 9
```

## 10 — Production gate pipeline

The nine-step production gate.

```mermaid
%%{init: {
  "theme": "base",
  "themeVariables": {
    "primaryColor": "#A5B4FC",
    "primaryTextColor": "#1E293B",
    "primaryBorderColor": "#4F46E5",
    "lineColor": "#6366F1",
    "secondaryColor": "#C7D2FE",
    "tertiaryColor": "#C7D2FE"
  }
}}%%
flowchart TB
    A["Step 1<br/>Formatting"]:::step
    B["Step 2<br/>Workspace check"]:::step
    C["Step 3<br/>Clippy"]:::step
    D["Step 4<br/>Focused tests"]:::step
    E["Step 5<br/>Runtime smoke"]:::step
    F["Step 6<br/>Session L2 path"]:::step
    G["Step 7<br/>Both consensus engines"]:::step
    H["Step 8<br/>Dep + stale-surface scan"]:::step
    I["Step 9<br/>Teeworlds acceptance"]:::step

    A --> B --> C --> D --> E --> F --> G --> H --> I

    classDef step fill:#A5B4FC,stroke:#4F46E5,color:#1E293B;
```

## Notes on re-use

- The CKB-teal/navy palette is defined in the
  `%%{init:{...}}%%` block. To use a different palette (e.g. for
  a darker presentation slide), only the colour values need to
  change.
- Mermaid's class definitions can override the theme variables
  per-node, which is how the layer-coloured diagrams work (orange
  for off-chain, teal for L2, cyan for L1).
- Sequence diagrams and gantt charts use the same theme variables
  but render with their own layout engine.