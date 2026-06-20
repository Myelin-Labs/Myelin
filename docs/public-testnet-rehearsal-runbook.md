# Public CKB Testnet Rehearsal Runbook

This runbook describes the first public CKB testnet rehearsal for Myelin
Session L2 readiness. It is not a gate script. Run it manually, preserve the
artefacts, and update `MYELIN_PRODUCTION_REHEARSAL_REPORT.md` with the observed
provenance.

## Goal

Move from:

```text
production-evidence-complete prototype
```

to:

```text
public-testnet production rehearsal complete
```

without claiming mainnet custody readiness.

## Non-Goals

```text
- no new release gate;
- no mainnet deployment;
- no custody-production claim;
- no hidden replacement of fixture evidence with ambiguous "ready" booleans.
```

## Inputs

Set these before the run:

```bash
export MYELIN_REHEARSAL_DIR=reports/public-testnet-rehearsal-$(date +%Y%m%d)
export CKB_TESTNET_RPC=https://example-testnet-rpc.invalid
export CKB_TESTNET_FUNDING_TX_HASH=0x...
export CKB_TESTNET_FUNDING_INDEX=0x0
export CKB_TESTNET_FUNDING_CAPACITY_SHANNONS=...
export CKB_TESTNET_CARRIER_CAPACITY_SHANNONS=...
export CKB_TESTNET_FEE_SHANNONS=...
export CKB_TESTNET_LOCK_CODE_HASH=0x...
export CKB_TESTNET_LOCK_HASH_TYPE=type
export CKB_TESTNET_LOCK_ARGS=0x...
export CKB_TESTNET_LOCK_DEP_TX_HASH=0x...
export CKB_TESTNET_LOCK_DEP_INDEX=0x...
export CKB_TESTNET_DA_VERIFIER_CODE_HASH=0x...
export CKB_TESTNET_DA_VERIFIER_DEP_TX_HASH=0x...
export CKB_TESTNET_DA_VERIFIER_DEP_INDEX=0x...
export CKB_TESTNET_DA_WITNESS=0x...
mkdir -p "$MYELIN_REHEARSAL_DIR"
```

The rehearsal operator must also have:

```text
1. a funded public CKB testnet cell;
2. compiled CellScript DA-anchor and settlement verifier artefacts;
3. operator custody policy JSON labelled for rehearsal;
4. operator runbook JSON labelled for rehearsal;
5. external DA receipt JSON, either real provider evidence or explicitly
   labelled rehearsal-provider evidence.
```

Copy the tracked Myelin CellScript verifier sources into the rehearsal
artefact directory before submission:

```bash
cp cellscript/examples/myelin/da-anchor-carrier.cell "$MYELIN_REHEARSAL_DIR/"
cp cellscript/examples/myelin/settlement-carrier.cell "$MYELIN_REHEARSAL_DIR/"
cp cellscript/examples/myelin/da-anchor-final.cell "$MYELIN_REHEARSAL_DIR/"
cp cellscript/examples/myelin/settlement-final.cell "$MYELIN_REHEARSAL_DIR/"
```

## Phase 1: Build Session Artefacts

Use deterministic fixture session data for the first rehearsal unless real game
session inputs are already available.

```bash
cargo run -p myelin-cli -- session open-fixture \
  --consensus static-closed-committee \
  --out "$MYELIN_REHEARSAL_DIR/session-open.json"

cargo run -p myelin-cli -- session commit-fixture \
  --session "$MYELIN_REHEARSAL_DIR/session-open.json" \
  --out "$MYELIN_REHEARSAL_DIR/session-commit.json"

cargo run -p myelin-cli -- session court-bundle \
  --commit "$MYELIN_REHEARSAL_DIR/session-commit.json" \
  --chunk-index 0 \
  --out "$MYELIN_REHEARSAL_DIR/session-court.json"

cargo run -p myelin-cli -- session verify-court-bundle \
  --bundle "$MYELIN_REHEARSAL_DIR/session-court.json" \
  --out "$MYELIN_REHEARSAL_DIR/session-court-verify.json"
```

Acceptance:

```text
session-court-verify.json has valid = true
```

## Phase 2: Build DA Evidence

For a rehearsal-provider receipt:

```bash
cargo run -p myelin-cli -- session da-manifest \
  --bundle "$MYELIN_REHEARSAL_DIR/session-court.json" \
  --storage-dir "$MYELIN_REHEARSAL_DIR/session-da-store" \
  --external-da-receipt "$MYELIN_REHEARSAL_DIR/external-da-receipt.json" \
  --out "$MYELIN_REHEARSAL_DIR/session-da.json"

cargo run -p myelin-cli -- session verify-da-manifest \
  --manifest "$MYELIN_REHEARSAL_DIR/session-da.json" \
  --bundle "$MYELIN_REHEARSAL_DIR/session-court.json" \
  --storage-dir "$MYELIN_REHEARSAL_DIR/session-da-store" \
  --out "$MYELIN_REHEARSAL_DIR/session-da-verify.json"
```

Acceptance:

```text
session-da-verify.json has valid = true
session-da.json has availability.production_ready = true only if the receipt
really carries signed production SLA fields
```

If the receipt is not a real provider receipt, label it as rehearsal-provider
evidence in `MYELIN_PRODUCTION_REHEARSAL_REPORT.md`.

## Phase 3: Build Packages

```bash
cargo run -p myelin-cli -- session da-anchor-package \
  --manifest "$MYELIN_REHEARSAL_DIR/session-da.json" \
  --bundle "$MYELIN_REHEARSAL_DIR/session-court.json" \
  --out "$MYELIN_REHEARSAL_DIR/session-da-anchor-package.json"

cargo run -p myelin-cli -- session verify-da-anchor-package \
  --package "$MYELIN_REHEARSAL_DIR/session-da-anchor-package.json" \
  --manifest "$MYELIN_REHEARSAL_DIR/session-da.json" \
  --bundle "$MYELIN_REHEARSAL_DIR/session-court.json" \
  --out "$MYELIN_REHEARSAL_DIR/session-da-anchor-package-verify.json"

cargo run -p myelin-cli -- session settlement-intent \
  --bundle "$MYELIN_REHEARSAL_DIR/session-court.json" \
  --da-manifest "$MYELIN_REHEARSAL_DIR/session-da.json" \
  --kind disputed-close \
  --current-time-ms 60000 \
  --challenge-window-ms 60000 \
  --court-economics-deployment-evidence "$MYELIN_REHEARSAL_DIR/court-economics-deployment.json" \
  --out "$MYELIN_REHEARSAL_DIR/session-settlement-intent.json"

cargo run -p myelin-cli -- session verify-settlement-intent \
  --intent "$MYELIN_REHEARSAL_DIR/session-settlement-intent.json" \
  --bundle "$MYELIN_REHEARSAL_DIR/session-court.json" \
  --da-manifest "$MYELIN_REHEARSAL_DIR/session-da.json" \
  --out "$MYELIN_REHEARSAL_DIR/session-settlement-intent-verify.json"

cargo run -p myelin-cli -- session settlement-package \
  --intent "$MYELIN_REHEARSAL_DIR/session-settlement-intent.json" \
  --bundle "$MYELIN_REHEARSAL_DIR/session-court.json" \
  --da-manifest "$MYELIN_REHEARSAL_DIR/session-da.json" \
  --authority-signature-evidence "$MYELIN_REHEARSAL_DIR/authority-signature-evidence.json" \
  --threshold-lock-deployment-evidence "$MYELIN_REHEARSAL_DIR/threshold-lock-deployment.json" \
  --out "$MYELIN_REHEARSAL_DIR/session-settlement-package.json"

cargo run -p myelin-cli -- session verify-settlement-package \
  --package "$MYELIN_REHEARSAL_DIR/session-settlement-package.json" \
  --intent "$MYELIN_REHEARSAL_DIR/session-settlement-intent.json" \
  --bundle "$MYELIN_REHEARSAL_DIR/session-court.json" \
  --da-manifest "$MYELIN_REHEARSAL_DIR/session-da.json" \
  --out "$MYELIN_REHEARSAL_DIR/session-settlement-package-verify.json"
```

Acceptance:

```text
all verify reports have valid = true
deployment evidence files are labelled fixture, rehearsal, testnet, or real
```

## Phase 4: Submit Public Testnet Transactions

Use the deployed verifier code deps and public testnet funding cell. The exact
submission command depends on whether the rehearsal uses the carrier path or
the final-script path.

For the DA-anchor carrier path:

```bash
cargo run -p myelin-cli -- session carrier-submission \
  --package "$MYELIN_REHEARSAL_DIR/session-da-anchor-package.json" \
  --input-tx-hash "$CKB_TESTNET_FUNDING_TX_HASH" \
  --input-index "$CKB_TESTNET_FUNDING_INDEX" \
  --input-capacity-shannons "$CKB_TESTNET_FUNDING_CAPACITY_SHANNONS" \
  --carrier-capacity-shannons "$CKB_TESTNET_CARRIER_CAPACITY_SHANNONS" \
  --fee-shannons "$CKB_TESTNET_FEE_SHANNONS" \
  --lock-code-hash "$CKB_TESTNET_LOCK_CODE_HASH" \
  --lock-hash-type "$CKB_TESTNET_LOCK_HASH_TYPE" \
  --lock-args "$CKB_TESTNET_LOCK_ARGS" \
  --lock-code-dep-tx-hash "$CKB_TESTNET_LOCK_DEP_TX_HASH" \
  --lock-code-dep-index "$CKB_TESTNET_LOCK_DEP_INDEX" \
  --verifier-code-hash "$CKB_TESTNET_DA_VERIFIER_CODE_HASH" \
  --verifier-code-dep-tx-hash "$CKB_TESTNET_DA_VERIFIER_DEP_TX_HASH" \
  --verifier-code-dep-index "$CKB_TESTNET_DA_VERIFIER_DEP_INDEX" \
  --verifier-source "$MYELIN_REHEARSAL_DIR/da-anchor-carrier.cell" \
  --witness "$CKB_TESTNET_DA_WITNESS" \
  --rpc-url "$CKB_TESTNET_RPC" \
  --submit \
  --require-accepted \
  --out "$MYELIN_REHEARSAL_DIR/da-anchor-carrier-submission.json"
```

For the settlement carrier or final-script path, use the same command with the
settlement package and settlement verifier. When rehearsing final-script
settlement evidence, also provide the evidence cell dep and authority input
arguments, and set:

```bash
--verifier-role final-l1-script
```

Every submitted transaction report must be written under:

```text
$MYELIN_REHEARSAL_DIR
```

Acceptance:

```text
1. submitted_to_rpc = true
2. accepted_by_rpc = true
3. dry_run = false
4. rpc_url is the public testnet endpoint
5. rpc_result matches the projected ckb_raw_tx_hash
```

## Phase 5: Observe Public Testnet Inclusion, Stability, and Finality

For every submitted transaction report:

```bash
cargo run -p myelin-cli -- session verify-submission-context \
  --submission "$MYELIN_REHEARSAL_DIR/<submission>.json" \
  --rpc-url "$CKB_TESTNET_RPC" \
  --out "$MYELIN_REHEARSAL_DIR/<role>-context.json"

cargo run -p myelin-cli -- session verify-submission-economics \
  --submission "$MYELIN_REHEARSAL_DIR/<submission>.json" \
  --rpc-url "$CKB_TESTNET_RPC" \
  --min-fee-shannons 1 \
  --min-fee-rate-shannons-per-kb 1000 \
  --max-fee-shannons 100000 \
  --out "$MYELIN_REHEARSAL_DIR/<role>-economics.json"

cargo run -p myelin-cli -- session verify-submission-inclusion \
  --submission "$MYELIN_REHEARSAL_DIR/<submission>.json" \
  --rpc-url "$CKB_TESTNET_RPC" \
  --min-status committed \
  --out "$MYELIN_REHEARSAL_DIR/<role>-inclusion.json"

cargo run -p myelin-cli -- session verify-submission-stability \
  --inclusion "$MYELIN_REHEARSAL_DIR/<role>-inclusion.json" \
  --rpc-url "$CKB_TESTNET_RPC" \
  --out "$MYELIN_REHEARSAL_DIR/<role>-stability.json"

cargo run -p myelin-cli -- session verify-submission-finality \
  --inclusion "$MYELIN_REHEARSAL_DIR/<role>-inclusion.json" \
  --rpc-url "$CKB_TESTNET_RPC" \
  --min-confirmations 6 \
  --out "$MYELIN_REHEARSAL_DIR/<role>-finality.json"
```

Then aggregate:

```bash
cargo run -p myelin-cli -- session verify-submission-readiness \
  --context "$MYELIN_REHEARSAL_DIR/<role>-context.json" \
  --economics "$MYELIN_REHEARSAL_DIR/<role>-economics.json" \
  --inclusion "$MYELIN_REHEARSAL_DIR/<role>-inclusion.json" \
  --stability "$MYELIN_REHEARSAL_DIR/<role>-stability.json" \
  --finality "$MYELIN_REHEARSAL_DIR/<role>-finality.json" \
  --require-live-submission \
  --operator-custody-policy "$MYELIN_REHEARSAL_DIR/operator-custody-policy.json" \
  --operator-runbook "$MYELIN_REHEARSAL_DIR/operator-runbook.json" \
  --out "$MYELIN_REHEARSAL_DIR/<role>-readiness.json"
```

Acceptance:

```text
production_submission_ready = true
readiness_evidence_mode is live-ckb-carrier or final-l1-script
live_carrier_submission_ready or final_l1_script_submission_ready matches the
chosen path
end_to_end_production_ready is interpreted only according to the artefact
provenance in MYELIN_PRODUCTION_REHEARSAL_REPORT.md
```

## Phase 6: Record Rehearsal Outcome

Update `MYELIN_PRODUCTION_REHEARSAL_REPORT.md` with:

```text
1. public testnet RPC endpoint used;
2. submitted transaction hashes;
3. committed block hashes and numbers;
4. confirmation depth observed;
5. verifier code-dep out-points and code hashes;
6. DA receipt provenance;
7. custody/runbook provenance;
8. remaining blockers.
```

The result should be one of:

```text
public-testnet production rehearsal complete
public-testnet production rehearsal partially complete
public-testnet production rehearsal failed
```

Do not label the result as mainnet production-ready.

## Minimal Failure Triage

Use the existing verifiers to locate the failure. Do not add a new gate.

```text
context failure    -> funding input, code dep, or lock/data hash mismatch
economics failure  -> capacity, fee floor, fee rate, max-fee, or change issue
inclusion failure  -> public testnet did not commit the expected tx hash
stability failure  -> reorg or changed committed block identity
finality failure   -> insufficient confirmation depth
readiness failure  -> lineage mismatch, submission evidence mismatch, missing
                     operator artefact, or missing production evidence object
```
