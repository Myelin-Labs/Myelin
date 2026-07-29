# First run

Generate the local session evidence spine:

```bash
mkdir -p reports

cargo run -p myelin-cli -- session open-fixture \
  --consensus static-closed-committee \
  --out reports/session-open.json

cargo run -p myelin-cli -- session commit-fixture \
  --session reports/session-open.json \
  --out reports/session-commit.json

cargo run -p myelin-cli -- session court-bundle \
  --commit reports/session-commit.json \
  --chunk-index 0 \
  --out reports/session-court-bundle.json

cargo run -p myelin-cli -- session verify-court-bundle \
  --bundle reports/session-court-bundle.json \
  --out reports/session-court-verify.json

cargo run -p myelin-cli -- session da-manifest \
  --bundle reports/session-court-bundle.json \
  --out reports/session-da-manifest.json
```

Use `--consensus weighted-precommit` on `open-fixture` to exercise the other closed-validator engine; the commit reads the selected kind from the v2 open report.

Expected boundaries:

- commit uses `ckb-strict-basic`, verifies scripts, mutates the Cell state atomically, and finalises a canonical Myelin block;
- the court bundle and DA manifest are internally reproducible;
- projection remains `projection_stage = "wire-encoded"` with `wire_encoded = true`;
- `court_verifiable` and `l1_court_implemented` remain false;
- local DA publication does not imply L1 publication.

The wire stage proves deterministic CKB Molecule bytes and hashes only. See [Claim ladder](../security/claim-ladder.md).
