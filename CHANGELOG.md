# Changelog

## Unreleased

### Pluggable closed-validator finality

- Added `proof-of-authority` as a third independent `ConsensusKind`, alongside `static-closed-committee` and `tendermint`; `poa` is accepted as a CLI/config alias while reports use the canonical name.
- Added an ordered authority set with deterministic `height mod authority_count` rotation, an independent `myelin:proof-of-authority-seal:v1` signature domain, and seals bound to the exact height, authority id, and canonical `MyelinBlock` hash.
- Added a single typed `FinalityProof` dispatch surface so static quorum certificates, PoA seals, and Tendermint decisions cannot be passed to the wrong engine by shape-compatible accident.
- Added PoA evidence to runtime, Session, and Teeworlds paths. Tests assert identical CellTx ids, witness hashes, scheduler commitments, and pre/post state roots across all three engines; only consensus-bound block/finality material differs.
- Extended the production gate with PoA config parsing, finality, runtime, and Session court-bundle verification.

All three choices use known validator/authority sets. PoA is an operational trust-model option, not a permissionless-security upgrade.

### Public CKB testnet exercise

- Funded account `ckt1qzda0cr08m85hc8jlnfp3zer7xulejywt49kt2rr0vthywaa50xwsq28mv6vp689txs9pyfyhj89qhavalf6ssq3y3h3m` successfully exercised the standard CKB secp256k1-blake160 sighash lock on public `ckb_testnet`.
- Transaction `0xbc662a7d0a452a61e50928a0e50b2b6a1c66c942988b4de8ace72c92a46131de` passed `test_tx_pool_accept` with 1,652,597 cycles and a 100,000-shannon (0.001 CKB) fee, was submitted with the same raw hash, and committed at height 21,908,123 in block `0x2c6a9f92c8aff3b4c10d4c073a28f42f6d5b7dc6a48961e61aa2270bc08e1a18`.
- The self-transfer output was re-read as a live Cell with 9,999.999 CKB capacity, and the block remained canonical through eight observed confirmations. This validates transaction construction, standard-lock signing, public RPC admission, commitment, and Cell recovery; it is not a public-testnet deployment of the full Session/court/DA path.
- Added `ckb generate-rehearsal-key`, `ckb multisig-config`, and `ckb create-cell` for permission-restricted disposable keys, canonical ordered multisig configuration, capacity planning, external signatures, and evidence-backed Cell creation with explicit change.
- Exercised both the legacy genesis multisig and the currently recommended `secp256k1_blake160_multisig_all` v2 script on public testnet. The v2 2-of-3 funding transaction `0xb6141a89ce9e80edb997c6b6635cee4ac98dfa1b9af91fdeca564a5aa00c55a3` and spend `0xd338df676eb97c917566a7006517f8dd14521979e00c90b0cc47cd3392268242` each reached six observed confirmations; the spend carries a locally verified CKB transaction proof and independently re-verifiable receipt chain.
- Corrected strict CKB-VM `SOURCE_CELL_DEP` semantics to expose ordered DepGroup members rather than the declared DepGroup root. This was found when the authoritative node accepted a standard system-script transaction while the local verifier failed to load the secp256k1 data Cell.
- Made context anchors reorganization-safe without requiring the chain tip to stop advancing: the resolution header must remain canonical at its height, while `test_tx_pool_accept` records its own stable validation tip.
- Enabled exact 128-bit CKB PoW nonce representation in nested JSON evidence and allowed `ckb observe`/`verify` to consume a `create-cell` report directly.
- Archived the v2 finalized evidence under `evidence/ckb-testnet/2026-07-29-multisig/`. Full public-testnet CellScript verifier deployment remains blocked by capacity: the 38,628-byte settlement artifact needs 38,822.001 CKB including conservative change and fee, 29,422.005 CKB more than the exercised funding input.

## 0.10.0 — 2026-07-29

Myelin 0.10.0 turns the retained finite-Cell runtime into an evidence-bound CKB execution kernel. It hardens transaction identity, conflict scheduling, state transitions, mempool admission, validator authentication, CKB projection, and the Session L2 path. It also removes the vendored CellScript compiler in favor of an attested adapter to one exact upstream toolchain.

Myelin remains an unpublished, pre-production project. This version deliberately removes obsolete internal formats instead of carrying compatibility shims. It must not be described as a finished trustless or permissionless L2.

### Highlights

- Added a fail-closed CKB evidence adapter with explicit `wire-encoded`, `context-resolved`, `consensus-validated`, `scripts-verified`, `node-accepted`, `committed`, and `finalized` stages.
- Resolved every transaction input, code dependency, dep-group member, and header dependency under a stable CKB tip before local verification.
- Added authoritative node validation through `test_tx_pool_accept`, exact-hash submission and observation, local CKB transaction-proof verification, canonical-header checks, reorganization detection, and configurable confirmation depth.
- Exercised valid DA and settlement transitions against the parent CKB 0.207.0 integration devnet, including committed transactions and rejection of tampered payloads and a competing settlement.
- Added `myelin ckb prove`, `myelin ckb observe`, and `myelin ckb verify` commands for producing and independently checking evidence projections.

### CellScript integration

- Removed the vendored CellScript workspace from this repository.
- Added a process adapter that verifies compiler identity, source revision, target profile, metadata schema, artifact digests, and metadata digests before accepting compiler output.
- Pinned CellScript release `v0.22.0` at peeled commit `830b5971237401a74dd7848b200f48b4d2ed79f4`.
- Pinned the compiler's CKB SDK v5.1.0 dependency at commit `1fbf3d4c9b35ef90bdb9e6621a8d26edde6325ce`.
- Added four Myelin-owned CellScript fixtures for DA carrier, settlement carrier, final DA, and final settlement verification.
- Added a reproducible toolchain gate that clones, builds, attests, and exercises the exact locked upstream compiler.

### Execution and state integrity

- Aligned transaction identity with CKB: witnesses affect `wtxid`, but not the raw transaction hash.
- Standardized the CKB wire version and Molecule dep-group representation and removed obsolete decoders and witness bridges.
- Made conflict identities state-resolved. Compiler metadata now supplies access templates rather than trusted final logical keys.
- Preserved READ/READ parallelism while ordering READ/WRITE and WRITE/WRITE access to the same logical key.
- Added physical OutPoint double-spend detection, logical conflict ordering, global barriers for non-parallelizable actions, and transitive failure handling in the CellDAG.
- Enforced a shared transaction-level VM cycle budget across all lock and type script groups.
- Made state application atomic and bound every transition to exact pre-state and post-state roots.

### Mempool and finality

- Made admission and replacement atomic and bound entries to the state root against which they were validated.
- Added deterministic conflict scoring and rejected unsupported multi-parent overlays unless their combined state is proven.
- Replaced placeholder validator authentication with secp256k1 Schnorr signatures.
- Kept static-committee and weighted-precommit finality in separate signature domains while preserving consensus-independent transaction execution and state roots.

### Session and submission evidence

- Required live submissions to pass node pool validation, return the expected raw transaction hash, and be observable as pending, proposed, or committed.
- Separated submission acceptance from publication and finality claims.
- Bound DA manifests, settlement intents, carrier payloads, verifier dependencies, authority evidence, and readiness reports to their exact lineage.
- Added negative tests for tampering, hash drift, under-capacity outputs, missing dependencies, replay, competing settlement, insufficient confirmation depth, and forged readiness evidence.

### Validation

- The root workspace passes formatting, build, Clippy with warnings denied, and the full test suite.
- The exact upstream CellScript v0.22.0 reproduction and fixture gate passes.
- The parent CKB integration devnet passes live admission, commitment, inclusion, stability, configured-depth finality, tamper rejection, and competing-settlement rejection checks.
- The production gate runs the parent CKB devnet path by default. The external Teeworlds workload remains optional because its repository and prebuilt replayer are not vendored.

### Known production blockers

The following work is required before Myelin can make production or permissionless security claims:

1. Deploy and exercise a canonical CKB threshold lock that directly verifies participant signatures on chain. The current devnet funding lock is an integration fixture.
2. Implement the complete disputed-chunk court, including challenge windows, adjudication, timeout behavior, bonds, slashing, and payout economics.
3. Deploy the full path to a public CKB testnet and preserve independently verifiable finalized transaction and header proof artifacts.
4. Integrate durable external data availability with retrieval proofs, redundancy, service guarantees, signed receipts, and failure drills.
5. Bind Session readiness directly to the generic finalized CKB evidence object so that the two evidence surfaces cannot diverge.
6. Add broader differential testing against the parent CKB implementation, fuzz and property testing, and long-running state, mempool, contention, and reorganization soak tests.
7. Prove combined state overlays for safe multi-parent mempool packages instead of conservatively rejecting them.
8. Establish production validator and operator key custody, rotation, recovery, monitoring, and incident-response procedures.

The current `finalized` stage means canonical-chain revalidation plus a configured confirmation depth. It is operational finality evidence, not a claim of absolute CKB irreversibility.
