# Remaining Myelin issues

Baseline: `2e2a540c777b19b4ea312c378bd7e0aa2eda8ec5`. The original issue bodies
are retained in [the scope snapshot](remaining-issue-snapshot.json). The user
selected Myelin work first; UTXOPlay platform issues remain separate work.

## Required results

| Issue | Implementation and evidence required for completion |
| --- | --- |
| #16 | Explicit source-proof Export/Verify/Import ports; independently pinned genesis, consensus and application policies; bounded durable inbox; atomic target consumption; independent-store/process fault drill; complete offline source dependencies; unsupported remote policy targets rejected |
| #17 | Documented transition table and irreversible points; one authoritative consume/cancel decision; exact-target test assets; source refund only after an exclusive cancellation certificate; lost ACK, clone, race, partition, restart, reordering and delayed-delivery evidence |
| #22 | A strict microgame verifier actually executed by CKB; actual open/update/challenge/settle/timeout transactions; one custody/single-spend domain; independent DA retrieval; local receipt revalidation and reorg handling; negative script execution evidence; instructional test-asset report |
| #6 | Reconcile shipped/reference/proposed/deployed documentation and the Myelin task checklist. Keep unfinished UTXOPlay tasks visible in the cross-project roadmap. |

## Integration decisions

Remote proofs are external evidence, not locally committed source history.
Import must never make `load_block` invent a source block or treat import as
consumption. Live execution, durable commit, recovery, replay and archive export
must share the same explicit source-resolution rule.

Source finality uses independently selected consensus configuration. Application
authorization and evidence-stage verification run separately. A supplied public
key, transport signature, receipt count or nonempty evidence cannot supply trust.

The scarce-asset reference will use an explicitly trusted durable terminal
authority, one exact target and test assets. Its unique irreversible decision
orders consumption against cancellation. Unknown results retain the lock;
expiry/local absence cannot authorize a refund. A consume decision binds the
exact target history/candidate so a second clone cannot obtain another credit.
This authority's availability and signing/storage assumptions must be explicit.

The CKB fixture must enforce game transitions and custody in scripts. Existing
carrier checks remain evidence of carrier checks. The existing selected CKB lane
will retain actual script, transaction, genesis and receipt identities for the
reference flow. Public-testnet execution is a distinct configured deployment;
the issue explicitly requires a no-real-funds devnet drill first.

Existing validation profiles remain the entry point. Tests should cover distinct
protocol failures and real persistence/process boundaries; avoid duplicate gates
and compatibility readers. Completion requires current evidence for every issue
clause, not just the five final checkboxes.

## Implementation checkpoint

Final CI, source identities and publication receipts are recorded in the linked
[issue #16](https://github.com/Myelin-Labs/Myelin/issues/16),
[issue #17](https://github.com/Myelin-Labs/Myelin/issues/17) and
[issue #22](https://github.com/Myelin-Labs/Myelin/issues/22) delivery records.
The local results below describe this implementation checkpoint.

- #16: explicit proof ports, independent source/target pins, durable bounded inbox,
  observed-fork quarantine, atomic consumption and target-only archive replay are
  implemented. The remote receipt drill passed 14 separate process calls.
- #17: typed remote coordination, one durable exclusive terminal authority,
  persistent exact target reservations, genesis/kernel-enforced consumption and certificate-gated cancel and
  retained foreign consumption proofs are implemented. The test-asset drill
  passed 63 separate process calls; integration races and source-only offline
  retirement archives passed. The copied-history regression also verifies that
  an unregistered target store cannot downgrade the genesis terminal rule.
- #22: the same strict CKB binary passed offline VM and actual devnet execution:
  22 transactions, 34 distinct checks, wrong game results rejected by the actual
  type script, actual challenge/settlement/timeout/abort and reorg handling.
  Local archive preflight reexecuted 31 proofs, rehashed 231 blocks and reverified
  22 receipts plus five input roots. External-provider retrieval is recorded by
  the separate CI recovery job; local files are not counted as independent DA.
- #6: earlier P1 post-merge Myelin/Chess CI runs both succeeded. UTXOPlay remains
  outside this user's selected scope and must stay visibly unfinished.
