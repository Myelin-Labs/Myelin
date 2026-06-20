# Adversarial Evidence Matrix

This matrix records the current positive and negative evidence coverage for the
Session L2 production-readiness graph. It is not a new gate. It is a compact
review aid for deciding whether a readiness claim is backed by a recomputable
artefact and at least one rejection path.

## Scope

Covered claim:

```text
The prototype should not mark production readiness true merely because a JSON
field says it is true. Production readiness must be derived from bound,
recomputable, mutually consistent evidence.
```

Out of scope for this matrix:

```text
- mainnet custody approval
- external audit sign-off
- public-testnet rehearsal completion
- sustained load testing
```

## Matrix

| Evidence area | Positive path | Negative path | Current status |
|---|---|---|---|
| Court bundle binding | `session_court_bundle_is_single_chunk_projectable`, `teeworlds_court_bundle_tendermint_precommit_path_verifies` | `session_court_bundle_rejects_tampered_state_root` | Covered for fixture court bundles. |
| DA manifest payload binding | `session_da_manifest_binds_to_verified_court_bundle_payload`, `session_da_manifest_can_be_backed_by_sealed_segment_storage` | `session_da_manifest_rejects_tampered_segment_root`, `session_da_manifest_rejects_tampered_availability_evidence` | Covered for local sealed DA storage. |
| External DA receipt binding | `session_da_manifest_binds_external_da_receipt_evidence`, `session_da_manifest_accepts_signed_production_da_receipt` | Receipt mismatch and signature failure are exercised inside `session_da_manifest_binds_external_da_receipt_evidence`; forged production readiness is rejected by `session_submission_readiness_rejects_forged_production_da_flag` | Covered for fixture provider receipt format; real provider rehearsal still missing. |
| DA production readiness | `session_submission_readiness_clears_da_blocker_for_recomputed_production_da_manifest` | `session_submission_readiness_rejects_forged_production_da_flag` | Covered: a naked boolean can no longer clear the real DA blocker. |
| DA anchor package binding | `session_da_anchor_package_binds_verified_manifest_into_ckb_projectable_celltx` | `session_da_anchor_package_rejects_tampered_manifest_hash` | Covered for package construction and projection. |
| DA anchor submission RPC binding | `session_da_anchor_submission_records_rpc_acceptance` | `session_da_anchor_submission_rejects_missing_live_input_before_broadcast`, `session_da_anchor_submission_rejects_rpc_hash_mismatch` | Covered for request construction and RPC-result binding; public-testnet inclusion still missing. |
| Settlement intent binding | `session_settlement_intent_binds_to_verified_court_bundle_and_challenge_window` | `session_settlement_intent_rejects_premature_settlement_permission` | Covered for fixture challenge-window semantics. |
| Court economics deployment evidence | `session_settlement_intent_accepts_bound_court_economics_deployment_evidence` | `session_settlement_intent_rejects_stale_court_economics_deployment_commitment`, `session_settlement_intent_rejects_tampered_court_economics` | Covered for recomputation, stale-commitment rejection, and tamper rejection. |
| Settlement package binding | `session_settlement_package_binds_verified_intent_into_ckb_projectable_celltx` | `session_settlement_package_rejects_tampered_intent_hash`, `session_settlement_package_rejects_tampered_authority_lineage`, `session_settlement_package_rejects_tampered_authority_authentication` | Covered for package, authority lineage, and authentication binding. |
| Authority signature evidence | `session_settlement_package_accepts_bound_threshold_lock_deployment_evidence` | `session_settlement_package_rejects_production_threshold_lock_without_participant_signatures` | Covered for fixture participant-authority signatures; real participant ceremony still missing. |
| Final settlement authority preflight | `session_submission_readiness_accepts_end_to_end_production_final_settlement_evidence` | `session_submission_readiness_requires_final_settlement_authority_preflight`, `session_submission_readiness_requires_final_settlement_threshold_lock_deployment_preflight`, `session_submission_readiness_requires_final_settlement_uniqueness_evidence` | Covered for readiness aggregation. |
| Settlement submission RPC binding | `session_settlement_submission_records_rpc_acceptance` | `session_settlement_submission_rejects_rpc_hash_mismatch` | Covered for RPC-result binding. |
| CKB carrier inclusion | `session_submission_inclusion_verifies_carrier_package_commitment`, `session_submission_inclusion_verifies_compact_carrier_payload`, `session_submission_inclusion_observes_committed_da_anchor` | `session_submission_inclusion_rejects_carrier_commitment_mismatch`, `session_submission_inclusion_rejects_carrier_type_args_mismatch`, `session_submission_inclusion_rejects_carrier_type_code_hash_mismatch` | Covered under mock RPC and optional local devnet smoke. |
| Context preflight | `session_submission_context_accepts_live_ckb_carrier_submission_schema`, `session_submission_context_marks_live_inputs_and_deps_ready` | `session_submission_context_rejects_carrier_verifier_code_dep_hash_mismatch`, `session_submission_context_rejects_missing_live_input` | Covered for live-cell and verifier-code-dep checks. |
| Economics preflight | `session_submission_economics_accepts_balanced_transaction_with_fee`, `session_submission_economics_reports_explicit_change_output` | `session_submission_economics_rejects_underfunded_transaction`, `session_submission_economics_rejects_insufficient_fee_rate`, `session_submission_economics_rejects_excessive_fee_without_change` | Covered for capacity, fee floor, fee rate, max fee, and change distinction. |
| Stability and finality | `session_submission_stability_accepts_same_committed_block`, `session_submission_finality_confirms_after_required_depth` | `session_submission_stability_detects_reorged_block_identity`, `session_submission_finality_rejects_shallow_confirmation_depth` | Covered under mock RPC; public-testnet observation still missing. |
| Readiness lineage and live submission | `session_submission_readiness_accepts_coherent_ready_reports`, `session_submission_readiness_accepts_live_submission_when_rpc_result_matches`, `session_submission_readiness_labels_final_l1_script_evidence_separately` | `session_submission_readiness_rejects_mismatched_hash_and_unconfirmed_finality`, `session_submission_readiness_rejects_mixed_report_lineage`, `session_submission_readiness_rejects_invalid_submission_report_reference`, `session_submission_readiness_requires_live_submission_when_requested`, `session_submission_readiness_rejects_recorded_carrier_hash_when_live_is_required`, `session_submission_readiness_requires_carrier_submission_when_live_is_required`, `session_submission_readiness_rejects_final_l1_script_without_preflight_evidence` | Covered for aggregator coherence and evidence-mode separation. |
| Operator policy evidence | `session_submission_readiness_binds_operator_custody_and_runbook_evidence` | `session_submission_readiness_rejects_weak_operator_policy_documents` | Covered for typed fixture documents; real approved operator artefacts still missing. |
| Carrier transaction construction | `session_carrier_submission_builds_payload_bound_ckb_request_with_change`, `session_carrier_submission_builds_final_settlement_with_authority_input`, `session_carrier_submission_can_submit_to_rpc_and_record_result` | `session_carrier_submission_rejects_package_payload_binding_drift`, `session_carrier_submission_requires_declared_authority_for_final_settlement`, `session_carrier_submission_requires_final_da_dep_for_final_settlement`, `session_carrier_submission_rejects_submit_when_verifier_dep_hash_mismatches`, `session_carrier_submission_rejects_live_submit_without_readable_verifier_source`, `session_carrier_submission_rejects_under_capacity_outputs` | Covered for carrier request construction and basic live-submit preflight. |
| Runtime smoke | `runtime_smoke_static_closed_committee_finalises_a_block`, `runtime_smoke_tendermint_finalises_a_block`, `runtime_smoke_state_is_consensus_agnostic_but_certificates_differ` | `runtime_smoke_rejects_unknown_consensus_kind` | Covered for CLI runtime wiring. |

## Remaining Gaps

These are not requests for more gates. They are the production-rehearsal gaps
that must be closed with real artefacts:

```text
1. Public CKB testnet final-script submission evidence.
2. Real external DA provider receipt and retrieval proof.
3. Real canonical threshold-lock deployment evidence.
4. Real deployed court/dispute economics script evidence.
5. Real operator custody and runbook artefacts, approved and exercised.
6. Reorg/retry/monitoring evidence from an actual rehearsal run.
7. Sustained adversarial and load evidence beyond unit/regression tests.
8. External audit result and issue disposition.
```

## Review Rule

For any future production-readiness claim:

```text
If the artefact is still fixture, mock, or local-devnet only, label it as such.
If a positive path exists without a matching rejection path, do not treat the
area as production-evidence complete.
```
