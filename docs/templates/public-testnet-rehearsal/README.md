# Public Testnet Rehearsal Evidence Templates

These files are starter artefacts for `docs/public-testnet-rehearsal-runbook.md`.
They are not gates and they are not proof of production readiness.

Use them as follows:

```text
operator-custody-policy.json       usable starter; replace operator details
operator-runbook.json              usable starter; keep fee/finality values aligned with the run
external-da-receipt.template.json  shape only; must be signed by the DA provider
court-economics-deployment.template.json
                                  shape only; must be filled from the generated settlement intent
threshold-lock-deployment.template.json
                                  shape only; must be filled from the generated authority evidence
authority-signature-evidence.template.json
                                  shape only; must be signed by participant authority keys
```

Do not copy a `.template.json` file into a rehearsal artefact directory without
replacing the placeholder hashes, signatures, deployment out-points, and policy
flags. The CLI should reject unreplaced cryptographic templates.

Minimal copy step:

```bash
cp docs/templates/public-testnet-rehearsal/operator-custody-policy.json "$MYELIN_REHEARSAL_DIR/"
cp docs/templates/public-testnet-rehearsal/operator-runbook.json "$MYELIN_REHEARSAL_DIR/"
cp docs/templates/public-testnet-rehearsal/*.template.json "$MYELIN_REHEARSAL_DIR/"
```

For public testnet, deployment evidence should normally use:

```text
network = ckb-testnet
deployment_policy = testnet-beta-...
ckb_enforceable_checked = true only after the code dep is live and checked
testnet_beta_ready = true only after the rehearsal has observed the live code dep
production_ready = false
```

Mainnet production evidence must not be inferred from these templates.
