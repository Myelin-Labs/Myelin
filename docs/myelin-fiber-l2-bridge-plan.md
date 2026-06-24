# Myelin-Fiber L2 Bridge Plan

## Purpose

This document records the proposed path for connecting Myelin to Fiber from the
Layer 2 side. It is based on a local read-only review of this repository and the
sibling Fiber checkout at:

```text
../fiber
```

The conclusion is deliberately narrow:

```text
Myelin can integrate with Fiber through a bridge/controller layer over CKB-shaped
transactions, Fiber RPC, payment hashes, and compact commitments.
```

It should not be described as a finished trustless shared custody layer until
Myelin's live L1 court, DA publication, deployed scripts, signing, inclusion,
and finality path have been exercised on a public CKB network.

## Current Position

Myelin is a CKB-style finite Cell session L2. Its present session path can open a
session, commit a chunk, emit a court bundle, emit a DA manifest, produce DA and
settlement packages, and verify those artefacts locally. The important boundary
is that the current projection layer proves whether a Myelin `CellTx` can be
represented as a CKB-style transaction/context; it does not by itself prove that
the transaction has been accepted, committed, or adjudicated by CKB.

Fiber is a CKB-based payment and channel network. It provides RPCs for opening
channels, opening channels with external funding, submitting signed funding
transactions, creating invoices, settling invoices, sending payments, listing
channels, and inspecting payment state. It also has a cross-chain hub precedent
for binding payment flows through a shared preimage and careful expiry budgeting.

There is no existing direct Myelin-Fiber adapter in either checkout. Integration
therefore needs a new boundary component.

## Recommended Boundary

The recommended first boundary is a standalone bridge controller.

The controller should:

- call Myelin CLI/session APIs to produce deterministic session artefacts;
- call Fiber JSON-RPC APIs to open channels, submit funding transactions, create
  invoices, settle invoices, and send payments;
- maintain an explicit mapping between `session_id`, Fiber `channel_id`, Fiber
  channel outpoint, payment hash, payment preimage, DA root, and court bundle
  hash;
- carry only compact Myelin commitments through Fiber payment metadata;
- leave full court bundles, DA payloads, and settlement packages in Myelin's
  artefact store or an external DA provider.

This boundary avoids coupling Fiber's actor/channel internals to Myelin's
session runtime. It also keeps custody and settlement claims auditable: Fiber
remains responsible for channel and payment state, while Myelin remains
responsible for session state, execution commitments, and court-facing evidence.

## Strongest Hook: Fiber External Funding

Fiber's external funding flow is the most concrete integration hook.

The expected flow is:

```text
Myelin bridge controller
  -> Fiber open_channel_with_external_funding
  -> Fiber returns channel_id and final unsigned funding transaction
  -> external wallet/signing policy fills witnesses only
  -> Fiber submit_signed_funding_tx
  -> bridge records channel_id and funding outpoint in Myelin session metadata
```

The critical rule is that the signed Fiber funding transaction must preserve the
raw transaction structure returned by Fiber. The signer may fill witnesses, but
must not rebuild or modify inputs, outputs, outputs data, or cell deps.

Consequences:

- Myelin can reference a Fiber funding transaction or channel outpoint as an
  escrow-like session input.
- Myelin cannot replace Fiber's funding transaction with a Myelin-generated DA
  anchor or settlement transaction after Fiber has negotiated it.
- Any Myelin commitment that must be bound to a Fiber channel should be known
  before funding negotiation, or should be bound later through payment metadata,
  session reports, or a separate CKB carrier transaction.

## Payment-Hash Bridge

A second viable hook is a shared payment hash or preimage.

Fiber supports invoice creation with a supplied payment hash and later explicit
settlement with the matching preimage. That makes the following app-level flow
possible:

```text
participant A opens or joins a Myelin session
participant B creates a Fiber hold invoice with payment_hash H
Myelin session records H as part of the payment-bound close condition
Fiber payment moves through channels while H remains unresolved
Myelin settlement intent verifies the session-side condition
bridge releases the preimage to settle the Fiber invoice
```

This gives useful prototype-level atomicity between Myelin session progress and
Fiber payment settlement. It is not, by itself, custody-grade trustlessness. The
bridge can still become a policy and liveness dependency unless the preimage
release and dispute path are later enforced by deployed CKB scripts.

Expiry handling is mandatory. Fiber TLC expiries and Myelin challenge windows
must be mapped conservatively so that the bridge never releases value on one
side after the other side's challenge or refund route has become unsafe.

## Commitment Metadata Bridge

Fiber payments can carry custom records. These records are suitable for compact
Myelin commitments, for example:

```text
session_id
chunk_index
state_root_before
state_root_after
court_bundle_hash
da_manifest_hash
da_segment_root
settlement_intent_hash
```

They are not suitable for full proof payloads. The bridge should treat Fiber
custom records as a commitment pointer channel, not as DA storage.

The first version should define one deterministic binary payload:

```text
domain: "myelin-fiber-commitment-v1"
version: u16
session_id: [u8; 32]
chunk_index: u64
state_root_after: [u8; 32]
court_bundle_hash: [u8; 32]
da_segment_root: [u8; 32]
settlement_intent_hash: [u8; 32]
```

The bridge should reject payloads that are not domain separated, not canonical,
or too large for Fiber's custom-record limits.

## Data Mapping

The first bridge schema should be explicit and append-only:

```text
MyelinSessionFiberBinding {
  bridge_schema_version,
  myelin_session_id,
  myelin_app_id,
  myelin_vm_profile,
  myelin_consensus_kind,
  fiber_peer_pubkey,
  fiber_channel_id,
  fiber_channel_outpoint,
  fiber_funding_tx_hash,
  fiber_payment_hash,
  fiber_payment_preimage_status,
  latest_myelin_chunk_index,
  latest_myelin_state_root,
  latest_myelin_court_bundle_hash,
  latest_myelin_da_manifest_hash,
  latest_myelin_settlement_intent_hash,
  created_at_unix_ms,
  updated_at_unix_ms
}
```

The schema should not hide uncertainty. Unknown live-chain facts should remain
`null` or `unconfirmed`, not be converted into optimistic booleans.

## CKB Transaction Adapter

Myelin and Fiber both speak CKB-shaped concepts, but they do not share the same
transaction construction surface.

Myelin currently owns:

- `CellTx` and deterministic session transaction commitments;
- CKB projection reports;
- conversion of projected `CellTx` data into CKB JSON transaction shape;
- DA anchor and settlement package generation.

Fiber currently owns:

- `ckb-types` and `ckb-sdk` transaction construction;
- channel funding transactions;
- external funding negotiation;
- funding transaction signing and submission constraints;
- channel and payment actor state.

The bridge should initially use JSON-RPC boundaries rather than importing deep
Fiber internals. If a Rust crate is later useful, it should start as a small
adapter around stable JSON types, not a dependency on Fiber's channel actors.

## Non-Goals For The First Version

The first bridge must not attempt to:

- merge Myelin and Fiber runtimes;
- make Fiber validate Myelin court bundles internally;
- use Fiber custom records as DA storage;
- mutate Fiber funding transactions after negotiation;
- claim CKB-enforced Myelin settlement before deployed court and DA scripts
  exist;
- claim mainnet custody readiness from local projection evidence.

## Phase 0: Specification Lock

Deliverables:

- this document;
- a canonical `MyelinSessionFiberBinding` JSON schema;
- a canonical `myelin-fiber-commitment-v1` binary payload schema;
- a list of exact Fiber RPC methods used by the bridge;
- a list of exact Myelin CLI/session reports consumed by the bridge.

Acceptance:

```text
reviewers can trace every field in the bridge schema to either a Myelin report,
a Fiber RPC response, or an external CKB transaction receipt.
```

## Phase 1: Local RPC Bridge Prototype

Build a standalone bridge prototype outside Fiber's actor internals.

Flow:

```text
1. run myelin session open or open-fixture
2. call Fiber open_channel_with_external_funding
3. sign the returned funding transaction through an external wallet/dev helper
4. call Fiber submit_signed_funding_tx
5. query Fiber list_channels until the channel is visible
6. store a binding between Myelin session_id and Fiber channel_id/outpoint
7. run myelin session commit
8. attach compact Myelin commitment metadata to a Fiber payment where possible
```

Acceptance:

```text
one local run produces:
- Myelin session open report
- Fiber channel id
- Fiber funding transaction hash or pending channel outpoint
- Myelin session commit report
- bridge binding JSON
- verification that all stored hashes match their source artefacts
```

## Phase 2: Payment-Bound Close Prototype

Add a payment hash to the session lifecycle.

Flow:

```text
1. create Fiber invoice with bridge-selected payment_hash
2. record payment_hash in the Myelin session binding
3. run Myelin commit and settlement-intent verification
4. send Fiber payment carrying compact Myelin commitment metadata
5. release preimage only after the Myelin-side close condition verifies
6. record Fiber payment result and Myelin settlement intent together
```

Acceptance:

```text
the bridge can prove that a Fiber payment hash was bound to a specific Myelin
session, chunk, DA root, and settlement intent before the invoice was settled.
```

## Phase 3: Public CKB Testnet Rehearsal

Move from local bridge evidence to public CKB evidence.

Required additions:

- real CKB testnet cells;
- deployed or rehearsal-labelled Myelin carrier/verifier scripts;
- wallet-funded cell selection;
- signature policy;
- CKB RPC inclusion checks;
- confirmation/finality checks;
- explicit operator custody policy.

Acceptance:

```text
the bridge can distinguish:
- local projection success;
- CKB RPC acceptance;
- committed transaction inclusion;
- sufficient confirmation depth;
- still-unimplemented court adjudication.
```

## Phase 4: Script-Enforced Settlement

Only after the public rehearsal should the project attempt script-enforced
settlement between Myelin and Fiber.

Target properties:

- deployed Myelin DA/court scripts on CKB;
- deterministic court payload verification;
- settlement package accepted by CKB with real inputs;
- challenge-window policy aligned with Fiber TLC expiry policy;
- documented failure and refund paths;
- no reliance on a bridge operator for correctness after evidence publication.

This is the first phase where a stronger trustless-settlement claim may become
available. It still requires adversarial testing and production key management
before any custody-production claim.

## Risk Register

### Raw Transaction Mismatch

Fiber external funding rejects structural changes after negotiation. The bridge
must not insert Myelin outputs, deps, or data into the returned Fiber funding
transaction.

Mitigation: bind Myelin data before negotiation where possible, or bind it after
funding through compact payment metadata and separate Myelin/CKB artefacts.

### Premature L1 Claim

Myelin projection evidence is not the same thing as CKB inclusion or court
adjudication.

Mitigation: reports and bridge schemas must keep projection, RPC acceptance,
inclusion, confirmation, and court execution as separate states.

### Expiry Mismatch

Fiber TLC expiries and Myelin challenge windows may use different safety
assumptions.

Mitigation: define one bridge expiry policy. The policy should require the
Fiber-side outgoing expiry to leave enough time for Myelin challenge, DA
retrieval, CKB submission, and refund handling.

### Amount Accounting

Channel balances, CKB capacities, UDT amounts, fees, and settlement outputs must
not be handled with floating point arithmetic.

Mitigation: use integer-only accounting, explicit units, and conservation checks
for every funding, payment, and settlement artefact.

### Metadata Capacity

Fiber custom records are bounded and should not carry large proof payloads.

Mitigation: store only compact commitments in Fiber metadata and keep full DA or
court payloads in Myelin artefact storage or an external DA provider.

### Bridge Operator Trust

A controller that releases preimages or submits transactions can become a trust
and liveness dependency.

Mitigation: keep Phase 1 and Phase 2 claims at the application-bridge level.
Move correctness into CKB scripts before making stronger settlement claims.

## Implementation Notes

The bridge should be conservative:

- prefer JSON files and JSON-RPC first;
- use exact hashes from existing Myelin reports;
- never recompute a source hash differently from the owning system without
  comparing against that system's report;
- persist every Fiber RPC request and response used for a state transition;
- treat every external CKB fact as unconfirmed until proven by RPC inclusion and
  finality checks;
- expose failure states rather than retrying silently.

Recommended first module shape:

```text
tools/myelin-fiber-bridge/
  README.md
  bridge.schema.json
  commitment_payload.md
  src/
    main.rs
    fiber_rpc.rs
    myelin_reports.rs
    binding_store.rs
    expiry_policy.rs
```

This can later graduate into a workspace crate if the prototype proves useful.

## Final Recommendation

Proceed with a bridge-controller prototype. Do not start by modifying Fiber
internals or by forcing Fiber to consume Myelin court bundles.

The first credible milestone is:

```text
one Myelin session, one Fiber externally funded channel, one payment hash, one
compact commitment payload, one binding file, and one end-to-end verification
report that keeps projection, payment state, and CKB inclusion status separate.
```

That gives Myelin a practical route to Fiber integration while preserving the
precision of its current Session L2 claims.
