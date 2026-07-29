# Production gate

`scripts/myelin_production_gate.sh` is the merge-readiness gate. It runs formatting, diff checks, workspace check/clippy/tests, focused protocol tests, exact upstream CellScript v0.22.0 reproduction and fixture compilation, all three closed-validator CLI paths, runtime/session/court/DA/settlement workflows, RPC admission/observation checks, the parent CKB devnet deployment smoke, dependency/stale-surface audits, and optionally the external Teeworlds workload.

```bash
scripts/myelin_production_gate.sh
```

Without the external workload:

```bash
RUN_TEEWORLDS=0 scripts/myelin_production_gate.sh
```

The gate clones the locked compiler and SDK revisions into a temporary toolchain root. Override the cache location when needed:

```bash
CELLSCRIPT_TOOLCHAIN_ROOT=/absolute/cache/path \
RUN_TEEWORLDS=0 scripts/myelin_production_gate.sh
```

The parent CKB smoke is enabled by default and expects `/home/arthur/a19q3/ckb` (override `CKB_ROOT`). Use `RUN_CKB_DEVNET=0` only when that checkout/binary is intentionally unavailable.

The gate expects exact consensus names, strict court VM profiles, honest separation of wire projection from adapter receipts, and false values for unfinished public-testnet/production claims. Passing the devnet smoke proves live behavior on an ephemeral local CKB chain; it is not public-chain deployment evidence.
