# Myelin CKB projection audit

## Result

Myelin has two deliberately separate projection surfaces:

- `project_cell_tx_to_ckb` is a pure wire projection. It emits only `Rejected` or `WireEncoded`.
- `myelin-ckb-adapter` is the evidence engine. It can advance the same exact raw transaction through `ContextResolved`, `ConsensusValidated`, `ScriptsVerified`, `NodeAccepted`, `Committed`, and `Finalized` only by constructing and verifying linked receipts.

There is no caller-controlled boolean override. Deserializing a higher-stage report is not enough: `verify_projection` recomputes transaction identity, context and receipt commitments, header hashes, observation bindings, the CKB transaction Merkle root, confirmation depth, and canonical-block equality.

For continuous operation, `EvidenceRuntime` stores that progression as one
revisioned receipt ladder per exact outbox message. Its CKB descriptor names
the seven stages from `wire-encoded` through `configured-depth-finality` and
commits the local verifier identity assigned to every stage. External
collection cannot advance the ladder until the adapter re-verifies the returned
evidence locally. The terminal receipt is persisted before the outbox item is
acknowledged, so restart resumes without collapsing or skipping claims.

## Stage model

| Stage | Required evidence | Producer |
| --- | --- | --- |
| `Rejected` | one or more wire/invariant blockers | `myelin-exec` |
| `WireEncoded` | exact version-0 transaction, CKB Molecule bytes, raw hash, witness-inclusive hash | `myelin-exec` |
| `ContextResolved` | all inputs, code deps, DepGroup members, and header deps resolved under one stable tip; node/chain/genesis/rule context committed | `myelin-ckb-adapter` |
| `ConsensusValidated` | exact transaction accepted by CKB `test_tx_pool_accept` around a separately sampled stable validation tip; fee, cycles, and node result committed | `myelin-ckb-adapter` |
| `ScriptsVerified` | strict local CKB-VM verification over that resolved context and one transaction cycle budget; local and node cycles recorded | `myelin-ckb-adapter` |
| `NodeAccepted` | `send_transaction` returns the exact raw hash and `get_transaction(..., false)` observes that hash as pending, proposed, or committed | `myelin-ckb-adapter` |
| `Committed` | the exact transaction is committed in the declared canonical block and its CKB transaction proof recomputes the block header's transactions root | `myelin-ckb-adapter` |
| `Finalized` | the committed block remains canonical, the transaction remains committed there, and the configured confirmation depth is reached | `myelin-ckb-adapter` |

An on-chain court verdict is outside this enum. It needs deployed-script, transaction, economic-policy, and chain evidence beyond transaction projection.

## Immutable context boundary

`ContextResolvedReceipt` commits to:

```text
chain and node version
genesis hash and consensus-rules hash
stable tip header
raw transaction hash and canonical transaction-JSON hash
every referenced OutPoint, role and source index
complete CellOutput and data
creation block identity where available
every resolved header dependency
```

DepGroups are parsed only as CKB Molecule `OutPointVec` data and every member is resolved independently. The resolver samples the tip before and after resolution and retries or fails if it moved. Strict VM `SOURCE_CELL_DEP` access uses the ordered expanded member view, while `LOAD_TRANSACTION` remains bound to the original declared DepGroup transaction bytes. Receipt verification rejects omitted, duplicated, reordered, mutated, stale, or transaction-unreferenced context.

Normal descendant blocks do not make a resolved context stale. Before node
validation and submission, Myelin re-queries the context tip by height and
requires its exact header hash to remain canonical. A same-height replacement
is rejected as a reorganization. The node validation call records its own
stable tip, which may be a descendant of the context anchor.

## Consensus and VM boundary

`ConsensusValidatedReceipt` records the authoritative node's contextual fee/cycle result and binds it to the context commitment and validation tip. Myelin does not pretend that a partial local rewrite of CKB consensus rules is authoritative.

`ScriptsVerifiedReceipt` independently runs all lock/type groups with `VmSemantics::CkbStrict` over the same resolved Cells and headers. Both the node cycle result and local cycle result are recorded. Their numeric values need not be identical because the two verifiers need not account for host overhead identically; acceptance requires both verdicts to succeed and both to remain within the configured transaction budget.

## Acceptance, inclusion, and finality

Node acceptance requires all three observations:

1. `test_tx_pool_accept` succeeds for the exact transaction;
2. `send_transaction` returns its exact raw transaction hash;
3. `get_transaction` with `only_committed = false` observes that exact hash as pending, proposed, or committed.

Commitment is stronger. Myelin re-queries the transaction, parses and verifies the committed header hash, checks the canonical header at that height, obtains the CKB transaction proof, and locally recomputes:

```text
proof(indices, lemmas, raw_tx_hash) -> raw_transactions_root
merkle(raw_transactions_root, witnesses_root) -> transactions_root
transactions_root == committed_header.transactions_root
```

The node's `verify_transaction_proof` response is retained as corroborating evidence, but offline verification does not trust it in place of recomputation.

Finality re-observes the transaction, re-queries the canonical block by height, rejects any block replacement, and requires the configured depth. This is depth-based operational finality, not a proof that CKB can never reorganize.

## Identity and failure audit

- producer `OutPoint`s and ordered transaction commitments use raw transaction identity;
- changing only witnesses leaves the raw hash unchanged and changes the witness-inclusive hash;
- scheduler plans bind to the raw txid but never enter CKB witnesses;
- context, consensus, scripts, node, commitment, and finality receipts form a one-way commitment chain;
- all receipt structs deny unknown fields;
- tests mutate context, node cycles, proof-recovered hashes, confirmation counts, and canonical block identity and require rejection.

The pure wire projection fails closed for a nonzero version, mismatched output/data lengths, Molecule encoding failure, raw-hash failure, or witness-hash failure. The evidence engine additionally fails closed for unstable tips, missing/non-live Cells, malformed DepGroups, missing headers, context drift, node rejection, local VM failure, cycle-limit breach, hash mismatch, observation timeout, proof mismatch, reorg, and insufficient confirmation depth.

## Exercised evidence

The adapter has been exercised against the parent CKB 0.207.0 integration devnet and against public `ckb_testnet` through `myelin ckb prove`, `observe`, and `verify`. The archived public transaction reached `Finalized` with a locally checked transaction proof and 13 observed confirmations. The full devnet gate also compiles four exact upstream CellScript v0.22.0 programs, deploys their ELF artifacts, commits valid DA and settlement transitions, and requires CKB to reject payload tampering and a competing settlement.

This does not establish a public-testnet court or permissionless L2 security. Session/court package-local `ckb_projection` fields may still honestly remain `wire-encoded`; a package advances only when it carries or references a separately verified `CkbEvidenceProjection` for its exact transaction.

## Remaining boundary

The next credibility milestone is not another projection enum value. Canonical `secp256k1_blake160_multisig_all` identity, exact witnesses, and finalized public-testnet transaction evidence are now exercised. The remaining milestone is to bind deployed verifier/court code OutPoints, provider-neutral DA certificates and live retrieval, production key custody, court economics, and operator policy into the same reproducible evidence bundle.
