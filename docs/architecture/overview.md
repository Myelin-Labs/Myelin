# Architecture overview

Myelin is one Cargo workspace with six protocol boundaries: external compiler adapter, Cell transaction execution, typed-state resolution, mempool/CellDAG scheduling, closed-validator finality, and evidence packaging.

The canonical architecture and trust-boundary description is in [`../MYELIN_ARCHITECTURE.md`](../MYELIN_ARCHITECTURE.md).

Key rules:

- CellScript is external and attested; it is not vendored.
- producer identity is the raw transaction hash;
- conflict hashes come from concrete typed Cell state;
- script groups share a transaction cycle budget;
- state application is atomic and root-bound;
- pure projection stops at `wire-encoded`; the evidence adapter can advance an exact exercised transaction through context, validation, scripts, acceptance, commitment, and configured-depth finality;
- finality remains closed-validator only.
