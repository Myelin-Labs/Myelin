# Architecture decisions

This page records the **why** behind Myelin's key design choices.
Each decision is a tradeoff; this page explains what was chosen,
what was rejected, and what the constraint was.

## ADR-style entries

Each entry follows a lightweight ADR shape:

```text
Status      (accepted / proposed / superseded)
Context     (what we were solving)
Decision    (what we chose)
Consequence (what this enables and what it costs)
Alternatives (what we considered and why we rejected them)
```

The decisions are ordered by how often they're asked about, not
chronologically.

---

## ADR-001: Use the CKB Cell Model, not an account model

**Status.** Accepted.

**Context.** A session runtime needs a state model. Two natural
candidates: an account model (like Ethereum) or a Cell model
(like CKB, like UTXO chains).

**Decision.** Use the Cell Model. State is a finite set of Cells;
transitions consume and create Cells; there is no global mutable
contract storage.

**Consequence.** Myelin inherits three properties:

- **Parallelism** — independent Cell groups can be verified in
  parallel.
- **Explicit dependencies** — every read and every write is named
  up-front.
- **Native typed assets** — user-defined assets are just Cells
  with type scripts.

It also inherits three constraints:

- **No mutable contract storage.** State changes go through Cell
  replacement.
- **Capacity accounting.** Every Cell must reserve enough
  capacity for its data and scripts.
- **Witness slot discipline.** Off-chain data has to fit into
  fixed witness slots.

**Alternatives considered.**

- **Account model.** Easier dApp portability, but loses parallel
  verification and forces explicit dependency tracking.
- **Hybrid model.** Possible in principle, but adds complexity
  without a clear benefit for the workload Myelin targets.

---

## ADR-002: Use CKB-VM (RISC-V) as the execution substrate

**Status.** Accepted.

**Context.** A session runtime needs a deterministic execution
environment. Options: EVM (familiar), WASM (portable), RISC-V
(CKB's choice), or a custom VM.

**Decision.** Use CKB-VM (RISC-V). Same VM as the CKB chain
itself.

**Consequence.**

- **Court path is trivial.** A disputed chunk can be replayed in
  the same VM that runs CKB scripts.
- **Existing tooling.** Rust, C, and JS toolchains target
  RISC-V. ckb-std, ckb-debugger, ckb-testtool all work.
- **Proven at L1.** Every script running on CKB mainnet runs in
  CKB-VM; Myelin doesn't have to defend a new VM.

The cost: smaller ecosystem than EVM, fewer pre-built libraries,
and the JS-VM is not universally available.

**Alternatives considered.**

- **EVM.** Familiar, but the EVM is a non-Cell-Model execution
  substrate. The court path would need a custom verifier.
- **WASM.** Portable, but every CKB script would need a separate
  WASM implementation to replay on L1.
- **Custom VM.** Total control, but total responsibility — no
  inherited toolchain or verifier.

---

## ADR-003: Use Molecule for serialization, everywhere

**Status.** Accepted.

**Context.** CKB uses Molecule for all on-chain serialization.
Session runtimes need a deterministic, schema-driven encoding
format too.

**Decision.** Use Molecule throughout Myelin. Every CellTx field,
every typed-cell metadata field, every block header.

**Consequence.**

- **`ckb_style_tx_hash` is deterministic.** The same Myelin
  CellTx bytes project to the same CKB transaction hash on every
  machine.
- **Court replay works.** The CKB-VM verifier can read the
  projected Molecule bytes directly.
- **Schema is shared.** The same `.mol` files describe both CKB
  and Myelin types.

The cost: Molecule is verbose and the schemas are non-trivial.
Worth it for the determinism.

**Alternatives considered.**

- **JSON.** Easy to read, but non-deterministic and slow to parse.
- **Protobuf.** Deterministic, but not what CKB uses — would
  break the court path.
- **Custom binary format.** Total control, but loses the CKB
  tooling story.

---

## ADR-004: Pluggable closed-validator finality, permissionless later

**Status.** Accepted.

**Context.** A session runtime needs finality. Relevant options include:
Nakamoto PoW (like CKB mainnet), static committee (like a
permissioned finality), rotating proof of authority, Tendermint (known-set BFT with a strict
greater-than-two-thirds quorum), or permissionless entry (like a public PoS
chain).

**Decision.** Ship three engines today (static closed committee, deterministic
rotating PoA, and finite-session Tendermint), behind one typed selection and
finality-proof dispatch surface.
Permissionless validator entry is future work.

**Consequence.**

- **Sessions can start small.** A two-validator static committee
  is enough to prove the runtime works.
- **Choice of trust model is configurable.** Each session picks
  its finality engine.
- **Trait abstraction keeps the rest clean.** The executor,
  scheduler, state, and projection don't know which engine is
  active.

The cost: Myelin is **not** a permissionless L2 today. Static
committee, PoA, and Tendermint all assume a known validator or authority set.

**Alternatives considered.**

- **Nakamoto PoW.** Same as CKB mainnet, but the runtime would
  need its own chain — not the goal.
- **Permissionless PoS immediately.** Requires staking, slashing,
  identity, and a totally different engine. Out of scope for the
  current seed.
- **Static committee only.** Simpler, but doesn't prove the trait
  abstraction actually works.

---

## ADR-005: Single-chunk court path, bisection as fallback

**Status.** Accepted.

**Context.** A dispute resolution protocol can be:

- **Single-chunk verification** — one chunk is CKB-VM-verifiable.
- **Interactive bisection** — the disputer and the producer walk
  the chunk down to a specific instruction.
- **Optimistic / fraud-proof** — disputes are rare, with a
  challenge period.

**Decision.** Single-chunk verification first. Interactive
bisection is a fallback design for the day when a chunk doesn't
fit a CKB-VM-style verifier.

**Consequence.**

- **Court bundles are self-contained today.** Anyone can replay
  them, even before the L1 court verifier exists.
- **The CKB court verifier, when built, consumes the same
  shape.** No back-compat changes needed for already-produced
  bundles.
- **No multi-round protocol overhead.** Single-chunk disputes
  resolve in one shot.

The cost: chunks must fit a CKB-VM-style verifier. For workloads
that produce very large chunks, this is a constraint.

**Alternatives considered.**

- **Bisection first.** More flexible, but adds multi-round
  protocol overhead and complicates the L1 court verifier.
- **Optimistic.** Familiar, but requires a long challenge period
  and doesn't fit the "disputed settlement" model cleanly.

---

## ADR-006: Projection is the credibility hinge

**Status.** Accepted.

**Context.** Without projection, Myelin could claim CKB-alignment
without proof. With projection, every transition has a
deterministic CKB-equivalent.

**Decision.** Every CellTx ships a `CkbProjectionReport`. The
projection layer is deterministic and pure.

**Consequence.**

- **Honest claim ladder.** Each CellTx knows whether it
  projects, and lists explicit deviation flags if not.
- **Audit is trivial.** Re-running the projection on a given
  CellTx produces the same report.
- **Court path becomes a deterministic input.** The projected
  bytes are what the future court verifier consumes.

The cost: the projection layer must be carefully maintained to
match the CKB Molecule layout and syscall surface. Any drift
between Myelin's Molecule encoding and CKB's breaks the
projection.

**Alternatives considered.**

- **No projection, just claim CKB-alignment.** Easier, but loses
  the credibility hinge.
- **Project only on demand.** Less overhead, but loses the
  per-claim honesty.

---

## ADR-007: No mandatory daemon or RPC server

**Status.** Accepted; networking portion amended by ADR-009.

**Context.** Most L2 designs assume P2P networking, a daemon
process, and an RPC server.

**Decision.** Myelin remains an embeddable kernel + CLI, without an official
mandatory daemon or RPC server. Deployments may compose the optional
authenticated durable session transport and runtime host from ADR-009. The
committee remains a configured closed set.

**Consequence.**

- **The kernel is auditable.** One process, one input, one
  output per subcommand.
- **Testing stays bounded.** Network state and async lifecycle code remain in
  optional edge crates rather than the deterministic execution spine.
- **Production deployment is your choice.** Wrap Myelin in a
  daemon, run it as a serverless function, or just call the CLI
  from a script.

The cost: every deployment still owns process packaging, peer discovery,
operational policy, and any public API surface.

**Alternatives considered.**

- **Daemon from day one.** More familiar, but adds complexity to
  the kernel that doesn't serve the protocol seed goal.
- **RPC server.** Same problem.

---

## ADR-008: Molecule-only public VM object ABI

**Status.** Accepted.

**Context.** The legacy CellScript scheduler-witness decode path
had a non-Molecule VM object ABI version.

**Decision.** Molecule is the only public VM object ABI. Legacy
versions are rejected at admission.

**Consequence.**

- **Deterministic `LOAD_TRANSACTION` bytes.** Both Myelin
  extended and CKB strict semantics use Molecule transaction
  bytes.
- **No legacy serializer in the execution graph.** The native
  myelin-exec has no direct or transitive legacy serializer
  dependency.
- **Public admission is Molecule-only.** No ambiguity about which
  ABI version a chunk used.

The cost: any code that depended on the legacy ABI has to migrate
to Molecule.

**Alternatives considered.**

- **Keep legacy ABI alongside Molecule.** Compatibility, but
  invites silent drift between the two formats.
- **Drop Molecule.** Loses the CKB compatibility story.

---

## ADR-009: Static consensus registration with genesis-locked selection

**Status.** Accepted.

**Context.** Myelin needs multiple closed-validator finality mechanisms without
letting concrete engines leak into session execution, durable transport, or
storage. Rust dynamic libraries would add ABI and supply-chain risk without a
current third-party distribution requirement.

**Decision.** Consensus modules are audited and statically linked into a closed
catalog. A session selects one at runtime, then persists an immutable module
descriptor commitment in genesis. The descriptor commits the canonical module
name, module protocol version, finality proof schema, driver-message schema,
WAL schema, capabilities, and exact validator/quorum configuration.

The session domain owns the deterministic `FinalityVerifier` port. Concrete
selection and adaptation belong to `myelin-session-runtime`. Proof and driver
message codecs belong to the consensus module; transport and RocksDB treat
their payloads as bounded opaque bytes while enforcing session/module identity.
An embeddable supervisor provides explicit dependencies, readiness, criticality,
panic/timeout containment, writer gating, and reverse-order shutdown. It is not
an official full-node daemon.

A session cannot hot-swap its engine, config, proof schema, message schema, or
WAL schema. Recovery under a different descriptor fails before writable state
is exposed. External modules and Rust dynamic-library loading are not supported.

**Change boundary for a new built-in engine.** It may change:

- `myelin-consensus` engine implementation, typed proof/codec, catalog entry,
  and protocol fixtures;
- `myelin-session-runtime` composition adapters and driver service wiring;
- engine-specific tests and documentation.

It must not require engine-specific changes in `myelin-exec`, `myelin-state`,
`myelin-mempool`, `myelin-session`, `myelin-session-network` transport,
`myelin-session-store-rocksdb`, or `myelin-ckb-adapter`.

**Consequence.** Runtime choice remains explicit and testable, while persisted
proofs, messages, WAL, outbox entries, and finalised records can be traced to
one genesis-bound module. The closed enum/catalog still requires a coordinated
Myelin release when adding an engine; this is deliberate before a stable
external module protocol exists.

**Alternatives considered.**

- **Rust dynamic libraries.** Rejected for unstable ABI, allocator/runtime
  coupling, weak crash isolation, and supply-chain complexity.
- **Hot-swappable session engines.** Rejected because one history could acquire
  two interpretations and unsafe recovery paths.
- **Concrete consensus inside `myelin-session`.** Rejected because every new
  engine would expand the session persistence and recovery blast radius.
- **External process modules now.** Deferred until an independently released
  module exists; any future design must pin and attest binaries and locally
  reverify all candidate proofs.

---

## Where to go next

- [FAQ](faq.md) — common questions and answers.
- [What is Myelin?](../concepts/what-is-myelin.md) — the broader
  positioning.
- [Claim ladder](../security/claim-ladder.md) — what the evidence
  actually proves.
