# Public Testnet Rehearsal Evidence Templates

These files are starter artefacts for `docs/public-testnet-rehearsal-runbook.md`.
They are not gates and they are not proof of production readiness.

For signed or commitment-bound evidence, prefer the `myelin session
external-da-receipt`, `authority-signature-evidence`,
`threshold-lock-deployment-evidence`, and
`court-economics-deployment-evidence` helpers. These templates are primarily
shape references and fallback review aids.

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
deployment_policy = ckb-system-multisig-v2-testnet
code_hash = 0x36c971b8d41fbd94aabca77dc75e826729ac98447b46f91e00796155dddb0d29
hash_type = data1
code_dep = 0x2eefdeb21f3a3edf697c28a52601b4419806ed60bb427420455cc29a090b26d5:0
ckb_enforceable_checked = true only after the complete DepGroup is live and every ordered member is checked by data hash
testnet_beta_ready = true only after the rehearsal has observed the live multisig-v2 code and secp256k1 data Cells
production_ready = false
```

The current public-network DepGroups order the multisig-v2 code Cell before
the shared secp256k1 data Cell. Evidence preserves the exact ordered member
list but resolves each role from its data hash, so it does not confuse member
position with script identity.

Mainnet production evidence must not be inferred from these templates.
