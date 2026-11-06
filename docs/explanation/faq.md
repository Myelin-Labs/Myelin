# FAQ

Answers to the questions that come up most often. Organised by
topic; click a question to jump to the answer.

## General

### What is Myelin in one sentence?

Myelin is a production CKB-aligned runtime for finite off-chain Cell sessions:
it verifies ordered transitions in CKB-VM, advances a durable
session head under closed validators, and emits staged evidence for
the exact CKB transaction.

See [What is Myelin?](../concepts/what-is-myelin.md) for the
long version.

### Is Myelin production-ready?

Yes. Myelin is **in production** for CKB-aligned application sessions, with
deterministic execution, closed-validator finality, durable commits, and
recovery. Applications configure their authorization, admission, resource,
and operator policies for their deployment.

CKB transaction verification, public-network court deployment, independent
DA, and custody each have their own evidence requirements. Use the
[claim ladder](../security/claim-ladder.md) to identify the guarantees of
an exact deployment.

### Is Myelin an L2?

Myelin is a **session runtime** with off-chain execution, closed-validator
finality, CKB projection, court bundles, and settlement packages. Its session
security model binds a known validator or authority set at genesis.
Permissionless L2 security and on-chain adjudication require separately
specified protocols and deployed contracts.

### What's the difference between Myelin and a CKB full node?

Myelin is **not** a CKB full node. It doesn't import the CKB
client, doesn't sync the CKB chain, doesn't run the CKB consensus.
It re-implements the parts of CKB it needs (Cell, CellTx, VM,
syscalls) in its own workspace and runs them off-chain.

A CKB full node is the L1; Myelin is a tool that runs on top of
it.

### What's the difference between Myelin and a sidechain?

Myelin supplies an application-chain runtime: execution, state roots, blocks,
configured static committee/PoA/Tendermint finality, durable storage and recovery.
Continuous production advances the session head, and explicit succession can
change its configuration. DA is one evidence component of that runtime.

An application can use these modules to build a CKB-oriented app sidechain.
It must still supply its node/service integration, networking and operating
policies, plus any CKB custody/bridge and dispute contracts. Myelin finality is
the configured validators' finality; an L1 DA anchor alone does not make CKB
verify the sidechain's execution. The repository does not ship a deployed court
or a turnkey permissionless sidechain node.

### Can CellScript contracts run on Myelin?

Yes, through its CellTx/CKB-strict execution path. CellScript compiles `.cell`
source into a RISC-V ELF and metadata. The pinned compiler adapter checks the
artifact, target profile and witness ABI; the application supplies the code
CellDeps, input/output Cells and witnesses. Myelin resolves scheduling accesses
from actual state and verifies the lock/type script groups before committing.

This requires the contract's dependencies and syscalls to fit the admitted
execution profile. Uploading source or a blob to DA does not deploy an executable
contract, and running the ELF in Myelin does not deploy it on CKB L1. The current
adapter uses the `ckb` target and `WitnessArgs.input_type` entry payload; it does
not accept the old raw entry-witness format. See the
[CellScript adapter](../architecture/cellscript.md) and
[execution/state/finality flow](../concepts/what-is-myelin.md).

## CKB concepts

### What is a Cell?

A Cell is the atomic unit of CKB state. It has four fields:
capacity (in shannons), data (arbitrary bytes), a lock script
(controls who can spend it), and an optional type script (enforces
state rules across Cells sharing the type).

See [What is CKB?](../concepts/what-is-ckb.md) for the full
explanation.

### What's the difference between a lock script and a type script?

A **lock script** runs when a Cell is consumed; it answers "is
this transaction authorised to spend this Cell?" — typically
signature verification.

A **type script** runs at commit time for every output Cell whose
`type` matches; it answers "does this Cell's data obey the schema
defined by this script?"

Myelin treats both as first-class in CellTx.

### What is CKB-VM?

CKB-VM is the RISC-V-based virtual machine that runs every CKB
script. It's deterministic, sandboxed, and has a small syscall
surface. Myelin runs a compatible execution substrate.

See [What is CKB-VM?](../concepts/what-is-ckb-vm.md).

### What is Molecule?

Molecule is CKB's deterministic, schema-driven, zero-copy binary
serialization format. Myelin uses it everywhere so that projected
CellTx bytes produce a deterministic CKB transaction hash.

### What's the difference between Cell capacity and balance?

Capacity is CKB's storage-and-value budget. Every Cell must
reserve enough capacity to cover the on-chain storage cost of its
data plus its scripts. Balance is the user-visible CKB amount.

You can't have more balance than capacity in a Cell. If you want
more balance, you need more capacity.

## Myelin architecture

### Why does Myelin have its own VM if it uses CKB-VM?

Myelin doesn't have a separate VM. It uses a CKB-VM-compatible
verifier — the same RISC-V instruction set, the same syscall
surface (with a small Myelin-only extension), the same
determinism contract.

The Myelin-only extension produces `projection_stage =
"myelin-native"` CellTxs, which can't project to CKB-style
transactions. CKB-compatible CellTxs run with no Myelin-only
syscalls.

### What is "projection"?

Projection is the function from a Myelin CellTx to a CKB-style
transaction. The output is a `CkbProjectionReport` that says
either `wire_encoded: true` (the CellTx is CKB-projectable)
or `wire_encoded: false` with explicit deviation flags.

See [CKB-style projection](../architecture/projection.md).

### What is the CellDAG scheduler?

The scheduler is the admission and audit component that builds a
CellDAG from CellTx read/write sets and emits ordered parallel
batches. It's deterministic (no fee markets, no L1 consensus
weighting) and auditable (every dependency is an explicit edge).

See [CellDAG scheduler](../architecture/scheduler.md).

### Why does Myelin have its own consensus if CKB has one?

Myelin's committee-based finality is for the **fast path** —
high-throughput block finality inside a session. CKB's Nakamoto
PoW is for the **slow path** — custody, court, and DA anchors.

They serve different jobs. The Myelin committee finalises blocks
in milliseconds; CKB finalises blocks in tens of seconds. The
projection layer connects the two.

### What's a committee certificate?

A finality proof is the engine-specific signature evidence that finalises a
`MyelinBlock`. For the static closed committee, it is a list of
signatures from quorum-weight validators. For PoA, it is one height-bound seal
from the scheduled authority. For Tendermint, it is a decision
certificate containing more-than-two-thirds signed precommits after a full
proposal/prevote/precommit round.

The typed proof is what makes a `MyelinBlock` a finalised consensus block.

## Operations

### How do I run Myelin today?

```bash
cargo run -p myelin-cli -- celltx simple-report
```

See [First run](../getting-started/first-run.md) for the full
end-to-end walk.

### How do I run the production gate?

```bash
scripts/myelin_production_gate.sh --profile core --out target/validation
```

See [Production gate](../operations/production-gate.md) for what
each step does.

### How do I run the devnet smoke?

You need a local CKB devnet first (via OffCKB or a parent `ckb`
checkout). Then:

```bash
scripts/myelin_ckb_devnet_smoke.sh
```

See [Local CKB devnet smoke](../operations/devnet-smoke.md).

### Where do the reports go?

By convention, into `reports/`. The CLI takes `--out <path>` for
every report-producing subcommand.

## Security

### What does "CKB-aligned" actually mean?

It means: a specific transition has a deterministic CKB
projection — i.e. the same bytes produce the same CKB transaction
hash on every machine, and a CKB-VM-style verifier can replay the
transition to verify the state root.

It does **not** mean: permissionless, censorship-resistant, or
mainnet-ready. Those require Tier 3 of the claim ladder.

### Is Myelin's closed-validator finality secure?

It's a **trust assumption**, not a cryptographic guarantee. You
trust the configured validators. If you don't trust them, the
projection + court path is your recourse.

### What does "tier 3" mean?

Tier 3 of the [claim ladder](../security/claim-ladder.md) is
"CKB-aligned adjudication path" — a CKB court verifier deployed
on a live chain has actually replayed a disputed chunk and emitted
a verdict. Myelin is not at Tier 3 today.

### What does the court bundle prove?

It proves that a specific disputed chunk is a **deterministic
input** that a CKB-VM-style verifier could consume. It does not
prove the verifier has actually consumed it — that's Tier 3.

### What's the difference between the execution report and the projection report?

The **execution report** answers "did Myelin's VM accept this
CellTx?" The **projection report** answers "can this CellTx be
encoded as a CKB-style transaction without changing semantics?"

Both are needed for Tier 1. The execution report alone is just
"Myelin runtime evidence" — not CKB-alignment evidence.

### What happens if a chunk's projection fails?

The CellTx still executes in Myelin (if the executor accepts it),
but its profile becomes `myelin-native` or `ckb-inspired-only`
instead of `wire-encoded`. The execution report and projection
report both carry the profile label and the explicit deviation
list.

Public demos should default to `wire-encoded`. If a demo can't
reach it, the labels should reflect that.

## Development

### How do I add a new script type?

Write the script in Rust (or C, or JS for the JS-VM), compile to a
RISC-V ELF, deploy it as a Cell dep, and reference it from a
CellTx's `cell_deps[]`. The verifier will load it like any other
CellScript.

See [CellScript & typed-cell metadata](../architecture/cellscript.md)
for the compiler-side contract.

### How do I write typed-cell metadata?

You don't, usually. The compiler emits typed-cell metadata from
your CellScript source. If you need to write it manually (for an
experiment), see the `TypedCellDecl` API in `myelin-exec`.

### How do I debug a failing VM execution?

Use `ckb-debugger` to reproduce the VM execution step by step.
For Myelin-specific issues, the `vm-probe` command shows the
witness wiring and the syscall trace.

### How do I contribute?

See the project's `AGENTS.md` and `README.md`. The kernel follows
the layering rules in
[System overview](../architecture/overview.md#layering-rules) —
those rules keep the runtime auditable.

## Where to go next

- [Architecture decisions](architecture-decisions.md) — the why
  behind the design.
- [What is Myelin?](../concepts/what-is-myelin.md) — the broader
  positioning.
- [Claim ladder](../security/claim-ladder.md) — what Myelin
  actually proves today.
