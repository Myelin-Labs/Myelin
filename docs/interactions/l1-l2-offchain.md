# The three-layer model

This page is the single diagram you should print and pin to the wall.
It shows who owns what, who computes what, who can dispute what, and
how the future court path fits in. Every other page in this section
zooms into one part of this picture.

## The full picture

```mermaid
%%{init: {
  "theme": "base",
  "themeVariables": {
    "primaryColor": "#EEF2FF",
    "primaryTextColor": "#1E293B",
    "primaryBorderColor": "#4F46E5",
    "lineColor": "#6366F1",
    "secondaryColor": "#E0E7FF",
    "tertiaryColor": "#C7D2FE",
    "fontFamily": "Inter, system-ui, sans-serif",
    "fontSize": "13px"
  },
  "flowchart": { "curve": "basis", "padding": 14 }
}}%%
flowchart TB
    subgraph OFF["Off-chain (producer / witness / DA)"]
        direction TB
        PR["Producer"]:::off
        WT["Witnesses<br/>(game tape, signed quote,<br/>sensor batch, agent task receipt)"]:::off
        DA["DA store<br/>+ optional external DA receipt"]:::off
    end

    subgraph L2["L2 — Myelin session runtime"]
        direction TB
        MP["Mempool<br/>(admission queue)"]:::l2
        SC["CellDAG scheduler"]:::l2
        EX["CKB-VM-style verifier<br/>(script groups, syscalls)"]:::l2
        ST["State store<br/>(live / consumed / created Cells<br/>+ state root)"]:::l2
        DM["DA manifest<br/>(Merkle SegmentProof)"]:::l2
        CO["Committee<br/>(static or Tendermint)"]:::l2
        BL["MyelinBlock<br/>+ committee certificate"]:::l2
        CB["Court bundle<br/>(per-chunk)"]:::l2
        SP["Settlement package<br/>(per-session)"]:::l2
        AP["Anchor package<br/>(per-DA-manifest)"]:::l2
    end

    subgraph L1["L1 — CKB mainnet / testnet / devnet"]
        direction TB
        CU["Asset custody Cells<br/>(session lock + participants)"]:::l1
        DA_L1["DA anchor CellTx<br/>(published on L1)"]:::l1
        ST_L1["Settlement CellTx<br/>(published on L1)"]:::l1
        CV["Court verifier<br/>(future CKB type script)"]:::l1
        VERDICT["Verdict<br/>(accept or slash)"]:::l1
    end

    %% Off-chain to L2
    PR --> WT --> MP
    MP --> SC --> EX --> ST
    ST --> BL
    CO --> BL
    EX --> DM
    DM --> AP
    AP --> DA_L1
    DM --> CB
    CB --> SP
    SP --> ST_L1

    %% L2 to L1 (custody + dispute)
    CU -. lock at session open .-> MP
    ST_L1 -. release / settle .-> CU
    DA_L1 -. fetch chunk payload .-> CV
    CB  -. dispute submission .- CV
    CV --> VERDICT

    classDef off fill:#E0E7FF,stroke:#D97706,color:#1E293B;
    classDef l2  fill:#EEF2FF,stroke:#4F46E5,color:#1E293B;
    classDef l1  fill:#C7D2FE,stroke:#7C3AED,color:#1E293B;
```

The three layers are colour-coded:

- <span class="badge off">Off-chain</span> — orange. Producer,
  witnesses, DA store, external DA provider.
- <span class="badge l2">L2 (Myelin)</span> — teal. Mempool,
  scheduler, verifier, state, committee, block, evidence packages.
- <span class="badge l1">L1 (CKB)</span> — cyan. Asset custody, DA
  anchor CellTx, settlement CellTx, future court verifier.

## Who owns what

| Concern | Owner | Where it lives |
| --- | --- | --- |
| **Long-lived asset custody** | Producer + participants | CKB Cells (L1) |
| **In-session state** | Myelin runtime | `myelin-state` (L2) |
| **Finality of Myelin blocks** | Committee (configured set) | `myelin-consensus` (L2) |
| **Chunk payload availability** | DA store + external DA provider | Off-chain, with anchor CellTx (L1) |
| **Dispute resolution** | Future CKB court verifier | L1 type script |

## Who computes what

| Computation | Layer | What it produces |
| --- | --- | --- |
| CellTx execution | L2 (verifier) | `MyelinExecutionReport` (cycles, state root transition) |
| CKB-style projection | L2 (verifier) | `CkbProjectionReport` (with `ckb_style_tx_hash`) |
| Block finality | L2 (committee) | `FinalisedBlock` with committee certificate |
| DA sealing | L2 (`myelin-state`) | DA manifest with `SegmentProof` |
| Anchor package | L2 (CLI) | CKB-compatible CellTx package with `l1_da_published = false` |
| Settlement package | L2 (CLI) | CKB-compatible CellTx package for disputed close |
| Court verification | L1 (future) | Accept / slash verdict |

## Who can dispute what

A dispute happens when **any participant** believes a finalised
Myelin block contains an invalid state-root transition. To dispute:

1. **Fetch** the disputed chunk payload from the DA store, or from
   the DA anchor CellTx on L1.
2. **Replay** the chunk in a CKB-VM-style verifier (off-chain, or on
   the future L1 court type script).
3. **Compare** the computed `state_root_after` against the one in
   the finalised block.

If they disagree, the disputer submits the **court bundle** to the
L1 court verifier. The court replays the chunk, and the verdict is
either "accept" (the committee was right, dispute bond refunded) or
"slash" (the committee was wrong, dispute bond awarded, committee
bond slashed).

The exact economics are in
[Settlement package](../interactions/submission-flow.md) — but the
shape is the same: deterministic replay, deterministic verdict.

## Where each piece lives across the layers

```mermaid
%%{init: {
  "theme": "base",
  "themeVariables": {
    "primaryColor": "#EEF2FF",
    "primaryTextColor": "#1E293B",
    "primaryBorderColor": "#4F46E5",
    "lineColor": "#6366F1",
    "secondaryColor": "#E0E7FF",
    "tertiaryColor": "#C7D2FE"
  }
}}%%
flowchart LR
    A["CellTx"]:::l2
    B["MyelinExecutionReport"]:::l2
    C["CkbProjectionReport"]:::l2
    D["DA manifest<br/>(payload_hash, segment_root,<br/>segment_proof)"]:::l2
    E["Anchor package<br/>(DA CellTx)"]:::l2
    F["Settlement package<br/>(disputed-close CellTx)"]:::l2
    G["Court bundle<br/>(chunk payload + CKB Molecule tx<br/>+ projection + cert)"]:::l2

    H["DA anchor CellTx<br/>(published)"]:::l1
    I["Settlement CellTx<br/>(published)"]:::l1
    J["Court verifier<br/>(replay + verdict)"]:::l1

    E --> H
    F --> I
    G --> J
    D --> E
    D --> G

    classDef l2 fill:#E0E7FF,stroke:#4F46E5,color:#1E293B;
    classDef l1 fill:#C7D2FE,stroke:#7C3AED,color:#1E293B;
```

Solid arrows: produced on L2, optionally published to L1. Dotted
arrows: an L1 verifier reads the L2 artefact from the published
package.

## Why this matters

Three patterns come up over and over in Myelin docs:

1. **L1 is custody and court, not real-time execution.** The CKB
   chain is where the assets live and where disputes are resolved.
   It is not where the high-throughput work happens.
2. **L2 is finite, not infinite.** Every Myelin session has an
   open, a sequence of finalised blocks, and a close. There is no
   "always-on global state." This is what makes the CellDAG and
   the state root meaningful.
3. **Off-chain is what carries the bulk data.** Game tapes, signed
   quotes, sensor batches, agent task receipts — none of these
   belong on-chain by default. They live in the DA store, with an
   anchor CellTx on L1 to prove their availability.

That three-layer split is what makes Myelin a *CKB-aligned* session
runtime rather than a CKB re-implementation or a generic
sidechain.

## What's not in the picture

- **No P2P layer.** The current Myelin kernel is a single-process
  runtime driven by the CLI. The committee is a configured set; it
  is not a gossip network.
- **No wallet.** Wallets are L1 concerns. Myelin talks to CKB
  through the JSON-RPC; it doesn't manage keys beyond committee
  validator keys.
- **No public RPC.** The CKB node provides the public RPC; Myelin
  consumes it.

## Where to go next

- [Session lifecycle](session-flow.md) — see one full session walk
  through the layers.
- [Court path](court-path.md) — the dispute-resolution deep dive.
- [Claim ladder](../security/claim-ladder.md) — what each piece of
  evidence actually proves about the L2.