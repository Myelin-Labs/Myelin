# Claim ladder

Claims must follow evidence, in this order:

| Level | Honest statement | Status |
| --- | --- | --- |
| 0 | CKB-shaped finite-Cell runtime | implemented |
| 1 | exact CKB Molecule transaction bytes and hashes produced | implemented as `wire-encoded` |
| 2 | all referenced CKB context resolved and committed under one stable tip | implemented by `myelin-ckb-adapter` |
| 3 | authoritative CKB node contextually accepted the exact transaction | implemented through linked `test_tx_pool_accept` evidence |
| 4 | all script groups verified locally in that same context and cycle budget | implemented by the adapter; node and local verdicts are both required |
| 5 | exact transaction accepted and observed by a CKB node | implemented |
| 6 | exact transaction committed with a locally verified CKB transaction proof | implemented |
| 7 | committed block stayed canonical through configured confirmation depth | implemented as depth-based `finalized` evidence |
| 8 | deployed public-testnet court adjudicated a disputed chunk | not implemented |

Local court-bundle `valid = true` means the host evidence package is internally consistent. It does not move the bundle to level 4 or 6; `court_verifiable` remains false.

Closed-validator finality says a configured quorum signed one Myelin session block. It does not establish permissionless L2 security.

Prohibited shortcuts:

- do not infer context validity from Molecule encoding;
- do not infer CKB script validity from a VM run against a different or synthetic context;
- do not infer node acceptance from a dry-run RPC request;
- do not infer L1 DA publication from local segment storage;
- do not infer commitment from mempool acceptance or a court verdict from a court input bundle;
- do not infer irreversible finality from a configured confirmation depth;
- do not raise a stage through caller-supplied booleans.
