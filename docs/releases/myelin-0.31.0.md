# Myelin 0.31.0

Myelin 0.31.0 strengthens the boundary between application command construction,
continuous session commits, and external recovery evidence. A command can be
reconciled after a commit/ACK crash without executing it again; a candidate keeps
the exact head it was built against; and a runtime can stop further state
advancement when independent recovery evidence is missing, invalid or too far
behind.

This remains a pre-production release for controlled sessions. It does not ship
a production DEX, independent DA providers, complete court economics, or a
permissionless CKB withdrawal protocol.

## Changes

- Genesis-selectable required command identities, frame-root binding, atomic
  RocksDB command receipts, duplicate rejection, recovery-index auditing, and
  `SessionChain::command_status` reconciliation.
- Explicit construction heads for producer selections and candidates, including
  manual submissions and cumulative open windows; checked preparation through
  `prepare_block_at` and the existing final head CAS.
- Distinct stale/rejected/unknown commit outcomes. Unknown outcomes and failed
  acknowledgements after commit retain reservations for durable reconciliation.
- Required runtime admission policy selection, guarded preparation and commit,
  and removal of the mutable-chain escape from `RuntimeHost`.
- A canonical-anchor recoverability policy that re-verifies deployment-supplied
  evidence and bounds all further block advancement, not just new deposits.
- Required escrow application binding on every transaction, including ones
  without ledger inputs/outputs; a nonzero evidence root is never treated as
  an automatic application consistency proof.
- The English DEX architecture and integration contracts, including one canonical
  ownership model, stable logical cancellation, compatible funding/withdrawal
  histories, and DA failure boundaries.
- The previously unreleased exact upstream CellScript 0.31.0 compiler pin and
  witness/metadata contract update. Myelin and CellScript are independently
  versioned projects.

## Compatibility

Session records and RocksDB use schema 7. Application profile/frame commitments
change. Old development stores and reports must be regenerated. This release
provides no automatic custody migration. External adapters must update command
metadata, producer snapshot/error contracts, runtime admission policy selection,
and escrow application binding. See [integration contracts](../architecture/session-safety.md).

## Validation

Passed on the release worktree:

```bash
cargo fmt --all --check
cargo check --locked --workspace --all-targets
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
RUN_TEEWORLDS=0 scripts/myelin_production_gate.sh
```

The final workspace run passed 756 tests with no failures or ignored tests.
The gate reproduced the exact pinned CellScript compiler, ran its 24-test entry
witness ABI suite, compiled all four Myelin fixtures, and passed the parent CKB
devnet smoke on CKB 0.209.0 (`all_live_checks_passed = true`). It also passed all three session
consensus paths, evidence/settlement report checks, and structural audits.

Additional focused regression tests passed after the final test refinements:
required command identities, receipt recovery and missing-index rejection,
reconstructed-command replay, stale/mixed construction heads, uncertain commits,
commit-before-ACK failure, canonical recovery anchors, publication lag, evidence
revocation before commit and after restart, and escrow application binding.

Teeworlds is an external workload and was explicitly skipped with
`RUN_TEEWORLDS=0`; this is not a passing Teeworlds integration result. These checks
prove the exercised local/devnet paths, not public-testnet court deployment or
production custody safety.

## Remaining boundaries

Command receipts establish committed frame inclusion; application scripts still
authenticate commands and enforce economic semantics. The recoverability gate
checks operational admission; protocol-level publication validity must be bound
and enforced by the application's validators. Its evidence verifier is supplied
by the deployment. CKB acceptance of custody/withdrawal claims requires the
separate asset-specific contracts and exit protocol.
