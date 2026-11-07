//! Authoritative shared-store handoff lifecycle. All updates belong to a
//! finalised block transaction; elapsed time never creates a refund right.
use crate::*;

/// Bound the work needed to resolve one inherited obligation. Longer histories
/// must reconcile outstanding handoffs before creating another continuation.
pub const MAX_HANDOFF_OWNER_DEPTH: usize = 32;

/// Trusted application classification used only by local storage retention.
/// A consumed scarce claim retains its backing history until retirement; a
/// consumed copyable receipt may move to a verified archive earlier. Returning
/// true must authenticate the source profile and payload as a copyable effect.
/// Neither classification permits deleting the command or handoff nullifier.
pub trait HandoffRetentionPolicy: Send + Sync {
    fn is_copyable(&self, source: &SessionConfig, handoff: &SessionHandoff) -> Result<bool>;
}

/// Unknown application semantics remain pinned until reclamation/retirement.
pub struct RetainUntilTerminal;
impl HandoffRetentionPolicy for RetainUntilTerminal {
    fn is_copyable(&self, _: &SessionConfig, _: &SessionHandoff) -> Result<bool> {
        Ok(false)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffBlockReference {
    pub session_id: Hash32,
    pub height: u64,
    pub block_hash: Hash32,
}
impl HandoffBlockReference {
    pub fn from_block(block: &MyelinBlock) -> Self {
        Self { session_id: block.session_id, height: block.number, block_hash: block.hash() }
    }
    fn validate(&self) -> Result<()> {
        if self.session_id == [0; 32] || self.block_hash == [0; 32] {
            return Err(SessionError::Continuity("zero lifecycle block identity".into()));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum HandoffTerminal {
    Issued,
    Consumed {
        at: HandoffBlockReference,
    },
    Cancelled {
        at: HandoffBlockReference,
    },
    Reclaimed {
        cancelled_at: HandoffBlockReference,
        reclaimed_at: HandoffBlockReference,
    },
    /// Target consumption remains permanent; the source only retires its backing lock.
    Retired {
        consumed_at: HandoffBlockReference,
        retired_at: HandoffBlockReference,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffLifecycle {
    pub issued_at: HandoffBlockReference,
    pub terminal: HandoffTerminal,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum HandoffAction {
    Cancel { handoff_id: Hash32 },
    Reclaim { handoff_id: Hash32 },
    Retire { handoff_id: Hash32, consumed_at: HandoffBlockReference },
    RemoteRetire { handoff_id: Hash32, proof: crate::RemoteConsumptionProof },
}
impl HandoffAction {
    pub fn handoff_id(&self) -> Hash32 {
        match self {
            Self::Cancel { handoff_id }
            | Self::Reclaim { handoff_id }
            | Self::Retire { handoff_id, .. }
            | Self::RemoteRetire { handoff_id, .. } => *handoff_id,
        }
    }
}

/// Exact original handoff and (for refund) a previously committed cancellation.
/// Retirement carries its consuming block reference in the action itself.
/// The application authenticates the action and its source-state disposition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedHandoffAction {
    pub action: HandoffAction,
    pub handoff: SessionHandoff,
    pub cancelled_at: Option<HandoffBlockReference>,
}
impl ResolvedHandoffAction {
    pub fn validate_for(&self, session_id: Hash32, height: u64) -> Result<()> {
        if self.action.handoff_id() != self.handoff.id()
            || (self.handoff.source_session_id == session_id && self.handoff.source_height >= height)
        {
            return Err(SessionError::Continuity("lifecycle action requires an earlier owned handoff".into()));
        }
        match (&self.action, &self.cancelled_at) {
            (HandoffAction::Cancel { .. } | HandoffAction::RemoteRetire { .. }, None) => Ok(()),
            (HandoffAction::Retire { consumed_at, .. }, None)
                if consumed_at.session_id != session_id || consumed_at.height < height =>
            {
                consumed_at.validate()
            }
            (HandoffAction::Reclaim { .. }, Some(at))
                if (at.session_id != session_id || at.height < height)
                    && (at.session_id != self.handoff.source_session_id || at.height > self.handoff.source_height) =>
            {
                at.validate()
            }
            _ => Err(SessionError::Continuity("lifecycle action requires the matching earlier cancellation or consumption".into())),
        }
    }
}

pub(crate) fn actions_commitment(session: Hash32, height: u64, actions: &[ResolvedHandoffAction]) -> Result<Hash32> {
    let value = (session, height, actions);
    codec::size_with_limit(&value, "lifecycle_bytes", codec::MAX_RECORD_BYTES as u64)?;
    let mut h = blake3::Hasher::new();
    h.update(b"myelin:handoff-lifecycle-actions:v2");
    h.update(&postcard::to_allocvec(&value).map_err(|e| SessionError::Codec(e.to_string()))?);
    Ok(*h.finalize().as_bytes())
}

impl StoredHandoff {
    pub fn is_available(&self) -> bool {
        matches!(self.lifecycle.terminal, HandoffTerminal::Issued)
    }

    pub fn validate_lifecycle(&self) -> Result<()> {
        let l = &self.lifecycle;
        l.issued_at.validate()?;
        if l.issued_at.session_id != self.handoff.source_session_id || l.issued_at.height != self.handoff.source_height {
            return Err(SessionError::Continuity("lifecycle issuer differs from handoff".into()));
        }
        match &l.terminal {
            HandoffTerminal::Consumed { at } | HandoffTerminal::Retired { consumed_at: at, .. } => {
                at.validate()?;
                if let HandoffTerminal::Retired { retired_at, .. } = &l.terminal {
                    retired_at.validate()?;
                    if (retired_at.session_id == l.issued_at.session_id && retired_at.height <= l.issued_at.height)
                        || (at.session_id == retired_at.session_id && at.height >= retired_at.height)
                    {
                        return Err(SessionError::Continuity("retirement must follow source issuance and target consumption".into()));
                    }
                }
            }
            state => {
                let cancel = match state {
                    HandoffTerminal::Issued => None,
                    HandoffTerminal::Cancelled { at } => Some(at),
                    HandoffTerminal::Reclaimed { cancelled_at, reclaimed_at } => {
                        reclaimed_at.validate()?;
                        if reclaimed_at.session_id == cancelled_at.session_id && reclaimed_at.height <= cancelled_at.height {
                            return Err(SessionError::Continuity("refund must follow cancellation".into()));
                        }
                        Some(cancelled_at)
                    }
                    HandoffTerminal::Consumed { .. } | HandoffTerminal::Retired { .. } => unreachable!(),
                };
                if let Some(at) = cancel {
                    at.validate()?;
                    if at.session_id == l.issued_at.session_id && at.height <= l.issued_at.height {
                        return Err(SessionError::Continuity("cancellation must follow source issuance".into()));
                    }
                }
            }
        }
        Ok(())
    }

    /// Pure CAS transition. The store must invoke this while holding the same
    /// transaction key used by consumption and commit it with the source block.
    pub fn apply_action(&mut self, resolved: &ResolvedHandoffAction, at: HandoffBlockReference) -> Result<()> {
        self.validate_lifecycle()?;
        resolved.validate_for(at.session_id, at.height)?;
        at.validate()?;
        if self.handoff != resolved.handoff {
            return Err(SessionError::Continuity("lifecycle handoff witness changed".into()));
        }
        let l = &mut self.lifecycle;
        l.terminal = match (&l.terminal, &resolved.action) {
            (HandoffTerminal::Issued, HandoffAction::RemoteRetire { proof, .. }) => {
                let record = crate::FinalisedBlockRecord::decode(&proof.target_record)?;
                HandoffTerminal::Retired { consumed_at: HandoffBlockReference::from_block(&record.block), retired_at: at }
            }
            (HandoffTerminal::Issued, HandoffAction::Cancel { .. }) => HandoffTerminal::Cancelled { at },
            (HandoffTerminal::Consumed { at: consumed }, HandoffAction::Retire { consumed_at, .. }) if consumed == consumed_at => {
                HandoffTerminal::Retired { consumed_at: consumed.clone(), retired_at: at }
            }
            (HandoffTerminal::Cancelled { at: cancellation }, HandoffAction::Reclaim { .. })
                if resolved.cancelled_at.as_ref() == Some(cancellation) =>
            {
                HandoffTerminal::Reclaimed { cancelled_at: cancellation.clone(), reclaimed_at: at }
            }
            _ => return Err(SessionError::Continuity("handoff lifecycle conflict; reconcile its terminal outcome".into())),
        };
        self.validate_lifecycle()
    }
}

fn referenced_record<S: SessionStore>(store: &S, at: &HandoffBlockReference) -> Result<FinalisedBlockRecord> {
    let record = store.load_block(at.session_id, at.height)?;
    let genesis = store.load_genesis(at.session_id)?;
    record.validate_resources(&genesis.config)?;
    if HandoffBlockReference::from_block(&record.block) != *at {
        return Err(SessionError::Recovery("lifecycle block reference mismatch".into()));
    }
    Ok(record)
}

/// Resolve ownership through immutable, byte-preserving successor declarations.
/// Returns sealing blocks from the nearest predecessor back to the issuer.
/// This checks canonical-store bindings, not finality or application execution;
/// archive import must independently audit and replay each referenced history.
pub fn handoff_owner_lineage<S: SessionStore>(
    store: &S,
    owner: &SessionConfig,
    handoff: &SessionHandoff,
) -> Result<Vec<HandoffBlockReference>> {
    let lineage = continuation_lineage(store, owner, handoff.source_session_id)?;
    let source = store.load_genesis(handoff.source_session_id)?;
    if source.config.application_profile.commitment() != handoff.source_application_profile_commitment
        || source.config.consensus_module_commitment != handoff.source_consensus_module_commitment
        || lineage.last().is_some_and(|seal| seal.height < handoff.source_height)
    {
        return Err(SessionError::Continuity("handoff owner lineage does not reach its exact issuer".into()));
    }
    Ok(lineage)
}

/// Canonical byte-preserving succession path, bounded independently of history
/// length. Applications must reject further continuation while an outstanding
/// obligation would exceed this limit.
pub fn continuation_lineage<S: SessionStore>(store: &S, owner: &SessionConfig, origin: Hash32) -> Result<Vec<HandoffBlockReference>> {
    let mut config = owner.clone();
    let mut lineage = Vec::new();
    let mut visited = HashSet::from([owner.session_id]);
    while config.session_id != origin {
        if lineage.len() == MAX_HANDOFF_OWNER_DEPTH {
            return Err(SessionError::Continuity("handoff owner lineage exceeds depth limit".into()));
        }
        let predecessor = config
            .predecessor
            .as_ref()
            .ok_or_else(|| SessionError::Continuity("lifecycle session is not a successor of the handoff source".into()))?;
        if !visited.insert(predecessor.session_id) {
            return Err(SessionError::Continuity("cyclic handoff owner lineage".into()));
        }
        let source = store.load_genesis(predecessor.session_id)?;
        let block = store.load_block(predecessor.session_id, predecessor.final_height)?;
        block.validate_resources(&source.config)?;
        let declaration = block.successor.as_ref().ok_or_else(|| SessionError::Continuity("owner lineage has no successor".into()))?;
        declaration.validate_source(&source.config, block.frame_input.root())?;
        let head = store.load_head(predecessor.session_id)?;
        if !matches!(declaration.transition.state, SuccessorState::Continue)
            || declaration.target_config != config
            || declaration.source_height != block.block.number
            || declaration.source_state_root != block.block.state_root_after
            || block.block.session_id != source.config.session_id
            || block.block.application_profile_commitment != source.config.application_profile.commitment()
            || block.consensus_module_commitment != source.config.consensus_module_commitment
            || !block.block.data_commitments.contains(&declaration.commitment())
            || head.finalised_height != Some(block.block.number)
            || head.block_hash != block.block.hash()
            || head.state_root != block.block.state_root_after
            || head.sealed_by_successor != Some(config.session_id)
        {
            return Err(SessionError::Continuity("handoff ownership requires exact state-preserving canonical succession".into()));
        }
        let target = store.load_genesis(config.session_id)?;
        if target.config != config {
            return Err(SessionError::Continuity("owner genesis differs from successor declaration".into()));
        }
        declaration.transition.check_source_snapshot(&target.state_snapshot)?;
        lineage.push(HandoffBlockReference::from_block(&block.block));
        config = source.config;
    }
    Ok(lineage)
}

/// Validate the actor's lineage and the causal position of a refund's earlier
/// cancellation. Call before execution and again at the atomic store boundary.
pub fn validate_handoff_action<S: SessionStore>(
    store: &S,
    owner: &SessionConfig,
    height: u64,
    action: &ResolvedHandoffAction,
) -> Result<Vec<HandoffBlockReference>> {
    action.validate_for(owner.session_id, height)?;
    let lineage = handoff_owner_lineage(store, owner, &action.handoff)?;
    if let HandoffAction::RemoteRetire { proof, .. } = &action.action {
        if owner.session_id != action.handoff.source_session_id {
            return Err(SessionError::Continuity("remote retirement forbids forwarding".into()));
        }
        let registry = store
            .handoff_import_registry()
            .ok_or_else(|| SessionError::Continuity("remote retirement requires independent peer trust".into()))?;
        proof.verify(&registry, owner, &action.handoff)?;
    }
    if let Some(at) = &action.cancelled_at {
        if at.session_id != owner.session_id
            && !lineage.iter().any(|seal| seal.session_id == at.session_id && at.height <= seal.height)
        {
            return Err(SessionError::Continuity("refund cancellation is not in the owner's earlier lineage".into()));
        }
    }
    Ok(lineage)
}

/// Check the immutable target block which justifies source lock retirement.
pub fn validate_consumption_record(at: &HandoffBlockReference, handoff: Hash32, record: &FinalisedBlockRecord) -> Result<()> {
    at.validate()?;
    if HandoffBlockReference::from_block(&record.block) != *at
        || record.consumed_handoff_ids.iter().filter(|id| **id == handoff).count() != 1
    {
        return Err(SessionError::Continuity("retirement consumption reference does not match the exact target block".into()));
    }
    Ok(())
}

/// Audit indexes against exact committed blocks. This does not substitute for
/// full application authorization replay or claim Byzantine-storage safety.
pub fn audit_handoff_lifecycle<S: SessionStore>(store: &S, stored: &StoredHandoff) -> Result<()> {
    stored.validate_lifecycle()?;
    let source = crate::resolve_handoff_source(store, &stored.handoff)?;
    let issued = source.record;
    let l = &stored.lifecycle;
    if l.issued_at != HandoffBlockReference::from_block(&issued.block) {
        return Err(SessionError::Recovery("lifecycle issuance block mismatch".into()));
    }
    let matches_action = |at: &HandoffBlockReference, action: HandoffAction, cancel: Option<HandoffBlockReference>| -> Result<()> {
        let record = referenced_record(store, at)?;
        let expected = ResolvedHandoffAction { action, handoff: stored.handoff.clone(), cancelled_at: cancel };
        validate_handoff_action(store, &store.load_genesis(at.session_id)?.config, at.height, &expected)?;
        if record.handoff_actions.iter().filter(|a| **a == expected).count() != 1 {
            return Err(SessionError::Recovery("lifecycle action is absent from its exact block".into()));
        }
        Ok(())
    };
    let id = stored.handoff.id();
    match &l.terminal {
        HandoffTerminal::Issued => Ok(()),
        HandoffTerminal::Consumed { at } => {
            let record = referenced_record(store, at)?;
            validate_consumption_record(at, id, &record)
        }
        HandoffTerminal::Retired { consumed_at, retired_at } => {
            let retirement = referenced_record(store, retired_at)?;
            let remote = retirement.handoff_actions.iter().find(|action| action.action.handoff_id() == id).and_then(|action| {
                if let HandoffAction::RemoteRetire { proof, .. } = &action.action {
                    Some(proof)
                } else {
                    None
                }
            });
            if let Some(proof) = remote {
                let registry = store
                    .handoff_import_registry()
                    .ok_or_else(|| SessionError::Recovery("remote retirement trust unavailable".into()))?;
                let source = store.load_genesis(stored.handoff.source_session_id)?.config;
                let target = proof.verify(&registry, &source, &stored.handoff)?;
                validate_consumption_record(consumed_at, id, &target)?;
                matches_action(retired_at, HandoffAction::RemoteRetire { handoff_id: id, proof: proof.clone() }, None)
            } else {
                let target = referenced_record(store, consumed_at)?;
                validate_consumption_record(consumed_at, id, &target)?;
                matches_action(retired_at, HandoffAction::Retire { handoff_id: id, consumed_at: consumed_at.clone() }, None)
            }
        }
        HandoffTerminal::Cancelled { at } => matches_action(at, HandoffAction::Cancel { handoff_id: id }, None),
        HandoffTerminal::Reclaimed { cancelled_at, reclaimed_at } => {
            matches_action(cancelled_at, HandoffAction::Cancel { handoff_id: id }, None)?;
            matches_action(reclaimed_at, HandoffAction::Reclaim { handoff_id: id }, Some(cancelled_at.clone()))
        }
    }
}

pub(crate) fn audit_actions<S: SessionStore>(store: &S, record: &FinalisedBlockRecord) -> Result<()> {
    let at = HandoffBlockReference::from_block(&record.block);
    for action in &record.handoff_actions {
        validate_handoff_action(store, &store.load_genesis(at.session_id)?.config, at.height, action)?;
        let stored =
            store.load_handoff(action.action.handoff_id())?.ok_or_else(|| SessionError::Recovery("action handoff absent".into()))?;
        audit_handoff_lifecycle(store, &stored)?;
        let terminal = &stored.lifecycle.terminal;
        let valid = match (&action.action, terminal) {
            (HandoffAction::Cancel { .. }, HandoffTerminal::Cancelled { at: c }) => *c == at,
            (HandoffAction::Retire { consumed_at, .. }, HandoffTerminal::Retired { consumed_at: c, retired_at: r }) => {
                *r == at && consumed_at == c
            }
            (HandoffAction::RemoteRetire { proof, .. }, HandoffTerminal::Retired { consumed_at: c, retired_at: r }) => {
                *r == at && HandoffBlockReference::from_block(&FinalisedBlockRecord::decode(&proof.target_record)?.block) == *c
            }
            (HandoffAction::Cancel { .. }, HandoffTerminal::Reclaimed { cancelled_at: c, .. }) => *c == at,
            (HandoffAction::Reclaim { .. }, HandoffTerminal::Reclaimed { cancelled_at: c, reclaimed_at: r }) => {
                *r == at && action.cancelled_at.as_ref() == Some(c)
            }
            _ => false,
        };
        if !valid || stored.handoff != action.handoff {
            return Err(SessionError::Recovery("lifecycle index differs from audited action".into()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retirement_commitment_matches_independent_postcard_vector() {
        let v: serde_json::Value = serde_json::from_str(include_str!("../../fixtures/protocol/retirement.json")).unwrap();
        let session: Hash32 = serde_json::from_value(v["session_id"].clone()).unwrap();
        let action: ResolvedHandoffAction = serde_json::from_value(v["resolved_action"].clone()).unwrap();
        action.validate_for(session, 9).unwrap();
        let mut preimage = b"myelin:handoff-lifecycle-actions:v2".to_vec();
        preimage.extend(postcard::to_allocvec(&(session, 9u64, std::slice::from_ref(&action))).unwrap());
        let hex: String = preimage.iter().map(|v| format!("{v:02x}")).collect();
        assert_eq!(hex, v["lifecycle"]["preimage_hex"]);
        let actual: String = actions_commitment(session, 9, &[action]).unwrap().iter().map(|v| format!("{v:02x}")).collect();
        assert_eq!(actual, v["lifecycle"]["commitment"]);
    }
}
