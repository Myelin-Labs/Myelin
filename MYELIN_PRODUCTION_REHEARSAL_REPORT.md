# Myelin Production Rehearsal Report

This report classifies the current production-readiness evidence by provenance.
It is deliberately not a new gate. Its job is to make clear which artefacts are
real, fixture-backed, mock-backed, devnet-backed, public-testnet-backed, or still
missing.

## Status

Current release posture:

```text
production-evidence-complete prototype / public-testnet rehearsal candidate
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
| Consensus evidence | Static closed committee and Tendermint fixture certificates | Fixture | Both consensus engines finalise the same state transition with separated signature domains. | Production validator set, key management, and sustained validator operation. |
| Teeworlds workload | `reports/myelin-teeworlds-repro.json` plus Teeworlds acceptance output | Fixture / local external checkout | Teeworlds replay can produce a CKB-compatible court bundle under deterministic replay evidence. | Public reproducibility package and long-running workload coverage. |
| DA manifest | `session da-manifest --storage-dir` and `verify-da-manifest` | Fixture + local sealed storage | Court replay payload is bound to a sealed local Merkle segment and recomputable DA availability evidence. | Real external DA publication and retrieval over production infrastructure. |
| External DA receipt | `myelin-external-da-receipt-v2` test fixture in unit tests | Fixture | Provider-signed receipt format, signature binding, SLA fields, and production-ready recomputation are enforced. | A real provider receipt, real HTTPS retrieval endpoint, audit log commitment, and retention verification. |
| DA production-readiness blocker | Final readiness now requires a recomputed production DA manifest for final-L1 DA/settlement evidence | Fixture-backed proof path | A naked `production_ready` boolean cannot clear the real DA blocker. | Same proof path must be fed by real public-testnet DA artefacts. |
| DA anchor package | `session da-anchor-package` and `verify-da-anchor-package` | Fixture | DA anchor CellTx package binds manifest, court bundle hash, segment root, and projection. | Real final L1 DA publication script and public-chain transaction. |
| Settlement intent | `session settlement-intent` and `verify-settlement-intent` | Fixture | Disputed-close settlement binds verified court bundle, DA manifest, challenge window, and court economics. | Real dispute instance, real bond/slash economics, and public-chain court verifier deployment. |
| Court economics deployment evidence | `--court-economics-deployment-evidence` path and regressions | Fixture | Court economics deployment evidence is recomputable and stale commitments are rejected. | Real deployed court/dispute economics script, audited source hash, audit report hash, and public-chain code dep. |
| Settlement package | `session settlement-package` and `verify-settlement-package` | Fixture | Package binds exact intent JSON, court bundle, DA manifest, final state root, and authority requirement. | Real settlement authority cell and public-chain final settlement transaction. |
| Authority signature evidence | `--authority-signature-evidence` path and regressions | Fixture | Participant authority signatures are required before production threshold-lock readiness can be claimed. | Real participant keys, signing ceremony, threshold policy, and custody process. |
| Threshold-lock deployment evidence | `--threshold-lock-deployment-evidence` path and regressions | Fixture | Deployment evidence is bound into settlement authority attestation and checked before final readiness. | Real canonical threshold-lock script deployment and audited public-chain code dep. |
| Carrier submission path | Optional `scripts/myelin_ckb_devnet_smoke.sh` | Local devnet | Compact carrier path can be deployed and submitted to a live local CKB node with negative tamper checks. | Public CKB testnet rehearsal with archived tx hashes and block evidence. |
| Final-script submission path | Unit fixtures and final-script readiness checks | Fixture / mock RPC | Final-script readiness requires live pre-submit markers, authority input checks, evidence cell deps, and production evidence preflights. | Public CKB testnet final DA and final settlement submission artefacts. |
| CKB inclusion / stability / finality | Production gate mock JSON-RPC reports | Mock | Aggregator logic rejects mismatched lineage, shallow finality, missing inclusion, and reorged block identity. | Real public testnet RPC observations over time. |
| Context / economics preflight | Production gate mock JSON-RPC reports; devnet smoke for carrier path | Mock + optional local devnet | Inputs, code deps, capacities, fee floor, fee rate, max fee, and change accounting are checked. | Real funding, wallet/change handling, fee bump policy, and retry evidence on public testnet. |
| Operator custody policy | `--operator-custody-policy` typed JSON path and regression | Fixture document | Readiness can hash and validate custody controls, but the default gate does not provide real operator custody. | Approved custody procedure, HSM or multisig setup, rotation drill, and emergency drill. |
| Operator runbook | `--operator-runbook` typed JSON path and regression | Fixture document | Readiness can hash and validate runbook controls, but the default gate does not provide a real production runbook. | Exercised runbook with monitoring, retry, reorg response, escalation, and incident logs. |
| External audit | None | Missing | No claim. | Independent audit, issue triage, and accepted risk register. |

## Current Closure Claim

The strongest claim currently supported by the repository is:

```text
Myelin can construct and verify a mutually bound production-readiness evidence
graph for DA, settlement authority, court economics, final-script submission,
public-chain observation, and operator policy when those artefacts are supplied.
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
production-evidence-complete prototype / public-testnet rehearsal candidate
```

After they are met, the correct release label can become:

```text
public-testnet production rehearsal complete
```

It should not become `mainnet custody production-ready` until every fixture,
mock, and local-devnet-only production claim in the table above has been
replaced by real, archived, reviewable production artefacts.
