# Introducing Myelin

Myelin explores the session layer above finite CKB-VM execution: ordering many Cell transitions, tracking authenticated local state, committing data, finalising a controlled session, and packaging disputed-chunk evidence.

The design keeps several boundaries explicit:

- CKB-VM validates one lock/type script group at a time; Myelin's CellDAG schedules across transactions.
- a consumed `OutPoint` prevents physical double spend; a typed conflict hash coordinates different Cells that represent one logical object;
- CellScript compiles through an attested external process adapter; it does not choose trusted conflict keys;
- closed-validator signatures finalise a Myelin session block, not a permissionless chain;
- CKB Molecule encoding proves exact transaction bytes, not contextual CKB validity or node acceptance;
- court and settlement packages are reproducible input shapes, not evidence of a deployed L1 verdict.

The most interesting concurrency case is sharded state. If every update consumes the same session Cell, CKB's physical conflict already serialises it. Myelin improves parallelism when transactions touch different logical domains or share read-only access. When receipt Cells differ but all update one logical session, a field-derived conflict key restores the necessary ordering.

CellScript is connected through `myelin-cellscript-adapter`, pinned to an exact upstream revision and compiler digest. The compiler's runtime-access metadata selects source/index locations. Myelin resolves those locations against authenticated Cell state and schema-aware `ConflictKeySpec`s; source binding names are never scheduler identities.

The pure projector remains capped at `wire-encoded`, while the CKB adapter now carries exact transactions through immutable context, authoritative-node validation, strict local script verification, node acceptance, commitment, and configured-depth finality receipts. The next credibility milestone is a deployed and exercised public CKB testnet court path with durable DA and operational keys.

For the exact current model, see [the architecture](../MYELIN_ARCHITECTURE.md) and [claim ladder](../security/claim-ladder.md).
