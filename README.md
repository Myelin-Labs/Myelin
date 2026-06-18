# Myelin

Myelin is a CKB-style isomorphic L2 protocol seed derived from Spora's
typed Cell execution work.

This repository intentionally removes Spora's old L1/node surface: no PoW
consensus, no mining, no P2P block propagation, no full-node daemon, and no
L1 mempool/block-template stack. What remains is the kernel needed to build an
off-chain finite Cell ledger:

- `cellscript/` - the local CellScript fork with the `typed-cell` target profile.
- `exec/` - Cell transactions, lock/type script verification, VM/syscall glue,
  typed-cell scheduler witnesses, and CellDAG scheduling.
- `state/` - live Cell state roots and data-availability proof primitives.
- `mempool/` - Cell transaction pool and deterministic conflict scoring.
- `crypto/`, `math/`, `utils/` - local support crates retained by the execution
  and state kernel.

## Protocol Shape

Myelin is intended to evolve toward:

```text
CellScript source
  -> typed-cell metadata + VM artifact
  -> CellTx delta
  -> CellDAG conflict scheduler
  -> deterministic VM verification
  -> committed L2 Cell state root
```

The target protocol is not an independent L1. It is a fast finite Cell ledger
whose state transitions are designed to stay close to CKB Cell semantics, so a
future L1 court can verify disputed transitions and exits.

## Current Bootstrap Scope

This initial repository cut keeps the Spora typed-cell execution substrate and
removes the chain infrastructure that is not part of the Myelin L2 protocol.
The internal Rust crate names still use the `spora-*` prefix where they were
inherited from the source project; crate renaming is deliberately deferred to a
separate migration.

No test run is implied by this bootstrap commit.

## Requirements

- Rust 1.85 or newer.
- `pkg-config`, OpenSSL, Clang, and libclang for the retained native crates.

## Licence

Myelin keeps the inherited MIT licence.
