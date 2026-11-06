use super::*;
use myelin_session::BlockRequest;
use myelin_session_authority::{CommandReconciliation, CommitAuthorization, FencingPolicy, VerifiedObservation, WriterGrant};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FencedSubmission {
    pub policy: FencingPolicy,
    pub grant: WriterGrant,
    pub authorization: CommitAuthorization,
    pub commands: Vec<CommandIdentity>,
}
impl FencedSubmission {
    pub(super) fn validate(&self, state: &State, ids: &[Hash32]) -> Result<()> {
        self.validate_structure(state, ids)?;
        self.authorization.verify_binding(&self.policy, &self.grant).map_err(reconciliation)
    }
    fn validate_structure(&self, state: &State, ids: &[Hash32]) -> Result<()> {
        if self.policy.session_id != state.scope.session_id
            || state.fencing_policy.as_ref() != Some(&self.policy)
            || self.policy.genesis_commitment != state.scope.genesis_commitment
            || self.commands.len() != ids.len()
            || self.commands.is_empty()
            || self.commands.len() > 256
        {
            return Err(JournalError::Invalid("fenced submission scope/command count differs".into()));
        }
        for (command, id) in self.commands.iter().zip(ids) {
            if command.id != *id || !state.entries.iter().any(|entry| entry.original.identity == *command) {
                return Err(JournalError::Invalid("fenced request does not cover original journal identities".into()));
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FencedRecovery {
    pub reservation_token: u64,
    pub submission: FencedSubmission,
    pub certificate: CommandReconciliation,
}

impl CommandJournal {
    /// Pin the trust policy at journal creation. Never accept a substitute key
    /// merely because a response declares the expected session/genesis ID.
    pub fn open_fenced(path: impl AsRef<Path>, config: &SessionConfig, limits: JournalLimits, policy: FencingPolicy) -> Result<Self> {
        if policy.session_id != config.session_id || policy.genesis_commitment != config.commitment() {
            return Err(JournalError::Invalid("fencing policy does not name this genesis".into()));
        }
        let journal = Self::open(path, config, limits)?;
        journal.update(|state| match &state.fencing_policy {
            Some(existing) if existing == &policy => Ok(()),
            Some(_) => Err(JournalError::Invalid("journal fencing policy cannot be replaced".into())),
            None if state.entries.is_empty() && state.reservation.is_none() && state.reconciliations.is_empty() => {
                state.fencing_policy = Some(policy);
                Ok(())
            }
            None => Err(JournalError::Conflict("existing unfenced work requires explicit adoption; no implicit epoch upgrade".into())),
        })?;
        Ok(journal)
    }
    /// Persist the exact authenticated request before allowing network/local
    /// submission. It must cover every original command in reservation order.
    pub fn begin_fenced_commit(
        &self,
        token: u64,
        policy: FencingPolicy,
        grant: WriterGrant,
        authorization: CommitAuthorization,
        request: &BlockRequest,
    ) -> Result<FencedSubmission> {
        self.update(|state| {
            let reservation = find_reservation(state, token)?;
            if reservation.phase != ReservationPhase::Held || reservation.construction_head != authorization.base_head {
                return Err(JournalError::Conflict("fenced commit requires an unsubmitted reservation at the signed head".into()));
            }
            let binding = FencedSubmission { policy, grant, authorization, commands: request.frame_input.commands.clone() };
            binding.validate(state, &reservation.command_ids)?;
            binding.authorization.verify_candidate(&binding.policy, &binding.grant, request).map_err(reconciliation)?;
            set_phase(state, ReservationPhase::Submitting);
            state.reservation.as_mut().unwrap().fenced_submission = Some(binding.clone());
            Ok(binding)
        })
    }

    /// Apply signed canonical observations, never a caller's Missing boolean.
    /// A higher epoch proves that the submitted epoch cannot later commit. A
    /// later same-ID commit remains protected by the canonical receipt index.
    /// Replayed evidence cannot reset a newer reservation/submission.
    pub fn reconcile_fenced(
        &self,
        token: u64,
        certificate: &CommandReconciliation,
        mut interpret: impl FnMut(&FinalisedBlockRecord, &CommandReceipt) -> std::result::Result<Option<BusinessOutcome>, String>,
    ) -> Result<Vec<JournalEntry>> {
        self.update(|state| {
            let binding = match &state.reservation {
                Some(r) if r.token != token => {
                    return Err(JournalError::Conflict("reservation advanced; old evidence cannot release newer work".into()))
                }
                Some(r) => r
                    .fenced_submission
                    .clone()
                    .ok_or_else(|| JournalError::Conflict("submission has no durable epoch binding".into()))?,
                None => state
                    .reconciliations
                    .iter()
                    .rev()
                    .find(|r| r.reservation_token == token)
                    .map(|r| r.submission.clone())
                    .ok_or_else(|| JournalError::Conflict("unknown fenced reservation".into()))?,
            };
            let verified = certificate.verify(&binding.policy, &binding.authorization, &binding.commands).map_err(reconciliation)?;
            let can_requeue = certificate.observed_epoch > binding.authorization.epoch;
            let profile = state.scope.application_profile;
            let result_limit = state.limits.max_result_bytes;
            for observed in verified {
                match observed {
                    VerifiedObservation::Missing(command) => {
                        let entry = find_entry(state, command.id)?;
                        if can_requeue && matches!(entry.status, EntryStatus::Reserved { token: t, .. } if t == token) {
                            entry.status = EntryStatus::Queued;
                        }
                    }
                    VerifiedObservation::Committed { receipt, record } => {
                        if record.block.application_profile_commitment != profile {
                            return Err(JournalError::Invalid(
                                "canonical business record uses a different application profile".into(),
                            ));
                        }
                        let entry = find_entry(state, receipt.command.id)?;
                        match &entry.status {
                            EntryStatus::IdentityConflict(_) => {
                                return Err(JournalError::Invalid("canonical evidence contradicts retained identity conflict".into()))
                            }
                            EntryStatus::Committed { receipt: previous, outcome, .. } => {
                                if *previous != receipt {
                                    return Err(JournalError::Invalid("canonical receipt changed".into()));
                                }
                                if outcome.is_some() {
                                    continue;
                                }
                            }
                            _ => {}
                        }
                        let outcome = interpret(&record, &receipt).map_err(JournalError::Reconciliation)?;
                        if outcome.as_ref().is_some_and(|o| o.len() > result_limit) {
                            return Err(JournalError::Invalid("business result exceeds journal limit".into()));
                        }
                        entry.status = EntryStatus::Committed { receipt, outcome, acknowledged: false };
                    }
                    VerifiedObservation::IdentityConflict { receipt } => {
                        let entry = find_entry(state, receipt.command.id)?;
                        if matches!(entry.status, EntryStatus::Committed { .. }) {
                            return Err(JournalError::Invalid("canonical conflict contradicts retained receipt".into()));
                        }
                        entry.status = EntryStatus::IdentityConflict(receipt);
                    }
                }
            }
            if !state.entries.iter().any(|e| matches!(e.status, EntryStatus::Reserved { token: t, .. } if t == token)) {
                state.reservation = None;
            }
            let digest = certificate.digest().map_err(reconciliation)?;
            if !state.reconciliations.iter().any(|r| r.reservation_token == token && r.certificate.digest().ok() == Some(digest)) {
                if state.reconciliations.len() >= state.limits.max_entries {
                    return Err(JournalError::Invalid("fenced recovery evidence retention capacity reached".into()));
                }
                state.reconciliations.push(FencedRecovery {
                    reservation_token: token,
                    submission: binding.clone(),
                    certificate: certificate.clone(),
                });
            }
            Ok(binding
                .commands
                .iter()
                .map(|c| state.entries.iter().find(|e| e.original.identity.id == c.id).unwrap().clone())
                .collect())
        })
    }

    /// Read-only audit of retained signed recovery decisions.
    pub fn fenced_recoveries(&self, start: usize, limit: usize) -> Result<Vec<FencedRecovery>> {
        if limit > 256 {
            return Err(JournalError::Invalid("recovery evidence page exceeds 256".into()));
        }
        let state = self.state.lock().map_err(|_| JournalError::Storage("journal mutex poisoned".into()))?;
        self.ensure_healthy()?;
        Ok(state.reconciliations.iter().skip(start).take(limit).cloned().collect())
    }
}

pub(super) fn validate_recoveries(state: &State, previous: Option<&State>) -> Result<()> {
    if let Some(policy) = &state.fencing_policy {
        if policy.session_id != state.scope.session_id || policy.genesis_commitment != state.scope.genesis_commitment {
            return Err(JournalError::Invalid("stored fencing policy scope differs".into()));
        }
    }
    if state.reconciliations.len() > state.limits.max_entries {
        return Err(JournalError::Invalid("recovery evidence count exceeds bound".into()));
    }
    let mut bindings = std::collections::BTreeMap::new();
    for (index, recovery) in state.reconciliations.iter().enumerate() {
        if recovery.reservation_token == 0 || recovery.reservation_token >= state.next_token {
            return Err(JournalError::Invalid("invalid recovery reservation token".into()));
        }
        let binding = &recovery.submission;
        let ids = binding.commands.iter().map(|c| c.id).collect::<Vec<_>>();
        // `previous` is the verified in-memory state held under the journal
        // mutex, never a persisted verification flag. Decode always passes None.
        // Recheck structural bindings against current entries even when the
        // complete signed evidence is byte-for-byte unchanged at this index.
        let verified_unchanged = previous.is_some_and(|old| {
            old.scope == state.scope && old.fencing_policy == state.fencing_policy && old.reconciliations.get(index) == Some(recovery)
        });
        binding.validate_structure(state, &ids)?;
        if !verified_unchanged {
            binding.authorization.verify_binding(&binding.policy, &binding.grant).map_err(reconciliation)?;
            recovery.certificate.verify(&binding.policy, &binding.authorization, &binding.commands).map_err(reconciliation)?;
        }
        let digest = binding.authorization.digest().map_err(reconciliation)?;
        if bindings.insert(recovery.reservation_token, digest).is_some_and(|previous| previous != digest) {
            return Err(JournalError::Invalid("reservation has contradictory signed submissions".into()));
        }
    }
    Ok(())
}
fn reconciliation(error: impl std::fmt::Display) -> JournalError {
    JournalError::Reconciliation(error.to_string())
}
