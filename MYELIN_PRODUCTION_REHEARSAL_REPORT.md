# Myelin Production Rehearsal Report

This report classifies the current production-readiness evidence by provenance.
It is deliberately not a new gate. Its job is to make clear which artefacts are
real, fixture-backed, mock-backed, devnet-backed, public-testnet-backed, or still
missing.

## Status

Current release posture:

```text
production-evidence-complete prototype / public-testnet rehearsal partially complete
```

Not current release posture:

```text
mainnet custody production-ready
```

The current positive `end_to_end_production_ready = true` regression proves that
the production evidence graph can close under controlled fixture-backed
artefacts. It does not prove that the same graph has already been closed with
real public-chain, external DA, production custody, or audited mainnet
deployment artefacts.

## Evidence Provenance

| Area | Current artefact | Provenance | Current production meaning | Mainnet gap |
|---|---|---|---|---|
| Session open / commit / court bundle | `session open-fixture`, `commit-fixture`, `court-bundle`, `verify-court-bundle` | Fixture | The Session L2 spine is deterministic, court-verifiable, and consensus-mode separated. | Real participant descriptors, real session funding, and real dispute inputs. |
| Teeworlds court-bundle checks | `scripts/myelin_teeworlds_acceptance.sh` and `verify-court-bundle` | Fixture / local external checkout | The reference Teeworlds court bundle verifies with `court_checks: 22`. | Public reproducibility package and broader workload coverage. |
| Consensus evidence | Static committee, rotating PoA, and Tendermint fixture proofs | Fixture | All three engines finalise the same state transition with separated proof domains. | Production validator set, key management, and sustained validator operation. |
| Teeworlds workload | Generated `reports/myelin-teeworlds-repro.json` plus Teeworlds acceptance output | Fixture / local external checkout | Teeworlds replay can produce a CKB-compatible court bundle under deterministic replay evidence. | Public reproducibility package and long-running workload coverage. |
| DA manifest | `session da-manifest --storage-dir` and `verify-da-manifest` | Fixture + local sealed storage | Court replay payload is bound to a sealed local Merkle segment and recomputable DA availability evidence. | Real external DA publication and retrieval over production infrastructure. |
| External DA receipt | `myelin-external-da-receipt` test fixture in unit tests | Fixture | Provider-signed receipt format, signature binding, SLA fields, and production-ready recomputation are enforced. | A real provider receipt, real HTTPS retrieval endpoint, audit log commitment, and retention verification. |
| DA production-readiness blocker | Final readiness now requires a recomputed production DA manifest for final-L1 DA/settlement evidence | Fixture-backed proof path | A naked `production_ready` boolean cannot clear the real DA blocker. | Same proof path must be fed by real public-testnet DA artefacts. |
| DA anchor package | `session da-anchor-package` and `verify-da-anchor-package` | Fixture | DA anchor CellTx package binds manifest, court bundle hash, segment root, and projection. | Real final L1 DA publication script and public-chain transaction. |
| Settlement intent | `session settlement-intent` and `verify-settlement-intent` | Fixture | Disputed-close settlement binds verified court bundle, DA manifest, challenge window, and court economics. | Real dispute instance, real bond/slash economics, and public-chain court verifier deployment. |
| Court economics deployment evidence | `--court-economics-deployment-evidence` path and regressions | Fixture | Court economics deployment evidence is recomputable and stale commitments are rejected. | Real deployed court/dispute economics script, audited source hash, audit report hash, and public-chain code dep. |
| Settlement package | `session settlement-package` and `verify-settlement-package` | Fixture | Package binds exact intent JSON, court bundle, DA manifest, final state root, and authority requirement. | Real settlement authority cell and public-chain final settlement transaction. |
| Authority signature evidence | `--authority-signature-evidence` path and regressions | Fixture | Participant authority signatures are required before production threshold-lock readiness can be claimed. | Real participant keys, signing ceremony, threshold policy, and custody process. |
| Threshold-lock deployment evidence | `--threshold-lock-deployment-evidence` path and regressions | Fixture | Deployment evidence is bound into settlement authority attestation and checked before final readiness. | Real canonical threshold-lock script deployment and audited public-chain code dep. |
| Public-testnet standard and multisig locks | `evidence/ckb-testnet/2026-07-29-multisig/` | Public testnet | Standard sighash, legacy multisig, and recommended multisig-v2 2-of-3 create/spend transactions were accepted and committed; the multisig-v2 spend has a verified transaction proof and configured-depth finality receipt. | Repeat the now parent-devnet-accepted Session multisig-v2 final-settlement path on public testnet and replace disposable rehearsal keys with an approved custody ceremony. |
| Carrier submission path | Optional `scripts/myelin_ckb_devnet_smoke.sh` | Local devnet | Compact carrier path can be deployed and submitted to a live local CKB node with negative tamper checks. | Public CKB testnet rehearsal with archived tx hashes and block evidence. |
| Final-script submission path | Unit fixtures and final-script readiness checks | Fixture / mock RPC | Final-script readiness requires live pre-submit markers, authority input checks, evidence cell deps, and production evidence preflights. Parameterized CellScript entry bytes and a canonical multisig `WitnessArgs.lock` have not yet been jointly accepted by parent CKB. | Resolve the entry-witness/multisig-lock composition boundary, require the parent-CKB devnet gate, then archive public-testnet final DA and settlement artefacts. |
| CKB inclusion / stability / finality | Production gate mock reports plus archived multisig-v2 receipts | Mock + public testnet | The real multisig-v2 spend binds resolved context, node acceptance, canonical inclusion proof, and six-confirmation observation. | Apply the same generic evidence object to final DA/court/settlement transactions. |
| Context / economics preflight | Production gate mocks, devnet smoke, and public-testnet `create-cell` reports | Mock + devnet + public testnet | Real funding, standard/multisig signing, explicit change, exact fee, live dependencies, and capacity shortfall reporting were exercised. | Fund verifier deployment and establish fee-bump/retry policy for Session transactions. |
| Operator custody policy | `--operator-custody-policy` typed JSON path and regression | Fixture document | Readiness can hash and validate custody controls, but the default gate does not provide real operator custody. | Approved custody procedure, HSM or multisig setup, rotation drill, and emergency drill. |
| Operator runbook | `--operator-runbook` typed JSON path and regression | Fixture document | Readiness can hash and validate runbook controls, but the default gate does not provide a real production runbook. | Exercised runbook with monitoring, retry, reorg response, escalation, and incident logs. |
| External audit | None | Missing | No claim. | Independent audit, issue triage, and accepted risk register. |

## Current Closure Claim

The strongest claim currently supported by the repository is:

```text
Myelin can construct and verify a mutually bound production-readiness evidence
graph for DA, settlement authority, court economics, final-script submission,
public-chain observation, and operator policy when those artefacts are supplied.
Its generic evidence engine has also finalized a real public-testnet
multisig-v2 spend, but not a Session settlement.
```

The claim intentionally excludes:

```text
- real external DA provider availability
- real public CKB testnet final-script settlement
- real threshold-lock and court-economics script deployments
- real operator custody
- real monitoring / retry / reorg operations
- mainnet custody approval
```

## 2026-07-29 Public-Testnet Checkpoint

The checkpoint is **partially complete**. It achieved:

```text
- standard sighash admission and commitment;
- legacy genesis 2-of-3 multisig compatibility;
- recommended multisig-v2 2-of-3 funding and spend;
- strict local CKB-VM plus authoritative node validation;
- canonical transaction proof verification and six-confirmation observation;
- an exact, non-broadcasting capacity plan for settlement-final.elf.
```

It did not achieve:

```text
- CellScript verifier deployment;
- DA publication or independent retrieval;
- court execution or economics;
- final Session settlement;
- production custody or an external audit.
```

The archived 38,628-byte settlement verifier needed 38,822.001 CKB including
conservative change and fee, leaving the exercised 9,399.996 CKB input short by
29,422.005 CKB. The current locked compiler produces a 38,940-byte verifier;
under the same assumptions it needs 39,134.001 CKB and is short by 29,734.005
CKB. The exact older plan and public transaction evidence remain archived in
`evidence/ckb-testnet/2026-07-29-multisig/` and are not evidence for the current
artifact.

## Public-Testnet Rehearsal Entry Criteria

A public-testnet rehearsal is ready to start when the runner has:

```text
1. a funded public CKB testnet account;
2. built CellScript final DA and settlement verifier artefacts;
3. a public CKB testnet RPC endpoint;
4. a real or explicitly labelled rehearsal external DA receipt;
5. operator custody and runbook JSON files labelled as rehearsal artefacts;
6. an output directory for immutable run artefacts.
```

Starter evidence document shapes live under
`docs/templates/public-testnet-rehearsal/`. They are rehearsal inputs, not
readiness proof by themselves.

## Public-Testnet Rehearsal Exit Criteria

The first public-testnet rehearsal is complete when the artefact directory
contains:

```text
1. deployed verifier code-dep out-points and code hashes;
2. DA anchor or carrier transaction hash accepted by public testnet RPC;
3. final settlement or settlement carrier transaction hash accepted by public
   testnet RPC;
4. inclusion, stability, and finality reports from public testnet RPC;
5. context and economics reports from public testnet RPC;
6. readiness reports whose provenance is public-testnet, not mock;
7. a copy of this report updated with the public-testnet artefact paths and
   remaining non-mainnet blockers.
```

## Release Boundary

Until those exit criteria are met, the correct release label is:

```text
production-evidence-complete prototype / public-testnet rehearsal partially complete
```

After they are met, the correct release label can become:

```text
public-testnet production rehearsal complete
```

It should not become `mainnet custody production-ready` until every fixture,
mock, and local-devnet-only production claim in the table above has been
replaced by real, archived, reviewable production artefacts.
