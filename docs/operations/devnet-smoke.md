# Local CKB devnet

The root CKB validation profile runs the [strict Rust microgame court](../architecture/ckb-microgame.md).
It deploys an actual RISC-V type script and exercises valid and invalid moves,
challenge, settlement, timeout/abort and reorg recovery. A retained package binds
program bytes, resolved Cells, transactions and block receipts for offline replay.

```bash
CKB_BIN=/absolute/path/to/ckb scripts/myelin_production_gate.sh --profile ckb --out target/ckb-validation
```

The optional compiler-specific carrier drill is maintained independently in
`integrations/cellscript/carrier-acceptance.md`. It is not part of a root profile.
