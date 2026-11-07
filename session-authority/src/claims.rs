//! Signed observations of claims in the canonical shared ledger. Missing and
//! Issued are observations, never cancellation, expiry or refund authorizations.
use crate::{protocol, AuthorityError, FencingPolicy, Result, SessionAuthority};
use myelin_session::{
    audit_handoff_lifecycle, bounded_codec, validate_consumption_record, validate_handoff_source, validate_handoff_trust,
    FinalisedBlockRecord, HandoffAction, HandoffBlockReference, HandoffTerminal, Hash32, ResolvedHandoffAction, SessionConfig,
    SessionHead, SessionStore, StoredHandoff, TransitionExecutor,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

pub const MAX_RECONCILIATION_CLAIMS: usize = 64;
const MAX_EVIDENCE_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum ObservedClaim {
    Missing,
    Present {
        #[serde(deserialize_with = "bounded_codec::bounded_bytes")]
        record: Vec<u8>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimObservation {
    pub id: Hash32,
    pub status: ObservedClaim,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimSession {
    #[serde(deserialize_with = "bounded_codec::bounded_bytes")]
    pub config: Vec<u8>,
    #[serde(deserialize_with = "bounded_codec::bounded_bytes")]
    pub head: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimReconciliation {
    pub policy: Hash32,
    pub observed_epoch: u64,
    #[serde(deserialize_with = "bounded_codec::bounded_vec")]
    pub observations: Vec<ClaimObservation>,
    /// The issuer and any consuming sessions in this same canonical ledger.
    #[serde(deserialize_with = "bounded_codec::bounded_vec")]
    pub sessions: Vec<ClaimSession>,
    /// Unique canonical records, preserving signed sizes but pruning snapshots.
    #[serde(deserialize_with = "bounded_codec::transactions")]
    pub blocks: Vec<Vec<u8>>,
    #[serde(deserialize_with = "bounded_codec::bounded_bytes")]
    pub signature: Vec<u8>,
}
fn invalid(message: &str) -> AuthorityError {
    AuthorityError::Invalid(message.into())
}
fn ids_valid(ids: &[Hash32]) -> Result<()> {
    if ids.is_empty()
        || ids.len() > MAX_RECONCILIATION_CLAIMS
        || ids.contains(&[0; 32])
        || ids.iter().collect::<HashSet<_>>().len() != ids.len()
    {
        return Err(invalid("claim query count/identities invalid"));
    }
    Ok(())
}
fn references(stored: &StoredHandoff) -> Vec<&HandoffBlockReference> {
    let mut refs = vec![&stored.lifecycle.issued_at];
    match &stored.lifecycle.terminal {
        HandoffTerminal::Issued => {}
        HandoffTerminal::Consumed { at } | HandoffTerminal::Cancelled { at } => refs.push(at),
        HandoffTerminal::Reclaimed { cancelled_at, reclaimed_at } => refs.extend([cancelled_at, reclaimed_at]),
        HandoffTerminal::Retired { consumed_at, retired_at } => refs.extend([consumed_at, retired_at]),
    }
    refs
}
impl ClaimReconciliation {
    pub fn digest(&self) -> Result<Hash32> {
        protocol::hash(
            b"myelin:writer-fence-claim-reconciliation:v1",
            &(self.policy, self.observed_epoch, &self.observations, &self.sessions, &self.blocks),
        )
    }
    /// Verify the exact ordered query under independent issuer trust. A caller
    /// migrating at epoch N must pass N; older observations cannot resolve it.
    /// This verifies canonical lifecycle evidence, not application replay or an
    /// independent-ledger transfer protocol. No result mutates a local balance.
    pub fn verify(&self, policy: &FencingPolicy, ids: &[Hash32], minimum_epoch: u64) -> Result<Vec<Option<StoredHandoff>>> {
        ids_valid(ids)?;
        if self.policy != policy.commitment()
            || self.observed_epoch == 0
            || self.observed_epoch < minimum_epoch
            || self.observations.len() != ids.len()
            || self.sessions.len() > ids.len() + 1
            || self.blocks.len() > ids.len() * 3
        {
            return Err(invalid("claim observation scope, epoch or count mismatch"));
        }
        bounded_codec::size_with_limit(self, "claim_evidence_bytes", MAX_EVIDENCE_BYTES).map_err(protocol::session)?;
        protocol::verify(policy.certificate_key, self.digest()?, &self.signature)?;
        let mut sessions = BTreeMap::new();
        for encoded in &self.sessions {
            let genesis = SessionConfig::decode(&encoded.config).map_err(protocol::session)?;
            let head = SessionHead::decode(&encoded.head).map_err(protocol::session)?;
            if genesis.session_id != head.session_id || sessions.insert(head.session_id, (genesis, head)).is_some() {
                return Err(invalid("claim session scope or duplicate genesis"));
            }
        }
        let (issuer, _) = sessions.get(&policy.session_id).ok_or_else(|| invalid("claim issuer genesis absent"))?;
        if issuer.commitment() != policy.genesis_commitment {
            return Err(invalid("claim issuer genesis differs from pinned policy"));
        }
        let mut blocks = BTreeMap::new();
        for bytes in &self.blocks {
            let block = FinalisedBlockRecord::decode(bytes).map_err(protocol::session)?;
            let (genesis, head) = sessions.get(&block.block.session_id).ok_or_else(|| invalid("claim block genesis absent"))?;
            block.validate_resources(genesis).map_err(protocol::session)?;
            if head.finalised_height.is_none_or(|height| block.block.number > height)
                || head.finalised_height == Some(block.block.number)
                    && (block.block.hash() != head.block_hash
                        || block.block.state_root_after != head.state_root
                        || block.block.timestamp_ms != head.timestamp_ms
                        || block.frame_input.end_position != head.input_position
                        || block.frame_input.logical_time_end != head.logical_time)
                || blocks.insert((block.block.session_id, block.block.number), block).is_some()
            {
                return Err(invalid("claim block is duplicate or outside canonical head"));
            }
        }
        let mut used_blocks = HashSet::new();
        let mut used_sessions = HashSet::from([policy.session_id]);
        let mut result = Vec::with_capacity(ids.len());
        for (id, observation) in ids.iter().zip(&self.observations) {
            if observation.id != *id {
                return Err(invalid("claim query identity/order changed"));
            }
            let ObservedClaim::Present { record } = &observation.status else {
                result.push(None);
                continue;
            };
            let stored = StoredHandoff::decode(record).map_err(protocol::session)?;
            if stored.handoff.id() != *id || stored.handoff.source_session_id != policy.session_id {
                return Err(invalid("claim does not belong to queried issuer/identity"));
            }
            for at in references(&stored) {
                let block = blocks.get(&(at.session_id, at.height)).ok_or_else(|| invalid("claim lifecycle block absent"))?;
                if HandoffBlockReference::from_block(&block.block) != *at {
                    return Err(invalid("claim block reference changed"));
                }
                used_blocks.insert((at.session_id, at.height));
                used_sessions.insert(at.session_id);
            }
            let issued = &blocks[&(policy.session_id, stored.handoff.source_height)];
            validate_handoff_source(issuer, issued, &stored.handoff).map_err(protocol::session)?;
            let action = |at: &HandoffBlockReference, action: HandoffAction, cancelled_at| -> Result<()> {
                if at.session_id != policy.session_id {
                    return Err(invalid("fenced claim lifecycle cannot change its source owner"));
                }
                let expected = ResolvedHandoffAction { action, handoff: stored.handoff.clone(), cancelled_at };
                expected.validate_for(at.session_id, at.height).map_err(protocol::session)?;
                if blocks[&(at.session_id, at.height)].handoff_actions.iter().filter(|a| **a == expected).count() != 1 {
                    return Err(invalid("claim terminal action is absent from its canonical block"));
                }
                Ok(())
            };
            match &stored.lifecycle.terminal {
                HandoffTerminal::Issued => {}
                HandoffTerminal::Consumed { at } | HandoffTerminal::Retired { consumed_at: at, .. } => {
                    let consumed = &blocks[&(at.session_id, at.height)];
                    validate_consumption_record(at, *id, consumed).map_err(protocol::session)?;
                    let target = &sessions[&at.session_id].0;
                    validate_handoff_trust(target, issuer).map_err(protocol::session)?;
                    if !stored.handoff.accepts_target(target)
                        || consumed.block.timestamp_ms > stored.handoff.expires_at_ms
                        || at.session_id == policy.session_id && at.height <= stored.handoff.source_height
                    {
                        return Err(invalid("claim consumption target, expiry or causal position differs"));
                    }
                    if let HandoffTerminal::Retired { retired_at, .. } = &stored.lifecycle.terminal {
                        action(retired_at, HandoffAction::Retire { handoff_id: *id, consumed_at: at.clone() }, None)?;
                    }
                }
                HandoffTerminal::Cancelled { at } => action(at, HandoffAction::Cancel { handoff_id: *id }, None)?,
                HandoffTerminal::Reclaimed { cancelled_at, reclaimed_at } => {
                    action(cancelled_at, HandoffAction::Cancel { handoff_id: *id }, None)?;
                    action(reclaimed_at, HandoffAction::Reclaim { handoff_id: *id }, Some(cancelled_at.clone()))?;
                }
            }
            result.push(Some(stored));
        }
        if used_blocks.len() != blocks.len() || used_sessions.len() != sessions.len() {
            return Err(invalid("unreferenced claim history"));
        }
        Ok(result)
    }
}
impl<E: TransitionExecutor> SessionAuthority<E> {
    /// One linearizable query across this ledger's independently fenced sessions.
    /// Claim issuance must belong to this authority's pinned genesis. A different
    /// ledger's Missing answer cannot release a local or remote reservation.
    pub fn reconcile_claims(&self, ids: &[Hash32], minimum_epoch: u64) -> Result<ClaimReconciliation> {
        ids_valid(ids)?;
        let inner = self.locked()?;
        let ledger = &inner.ledger_session.ledger;
        let _commits = ledger.commit_lock()?;
        if inner.chain.config().protocol_effects.remote_terminal().is_some() {
            return Err(invalid("remote scarce claims require terminal-authority lookup and retained consumption evidence; local Missing/Issued is not terminal exclusion"));
        }
        let epoch = inner.ownership.grants.last().ok_or_else(|| invalid("claim issuer has no active writer"))?.epoch;
        if epoch < minimum_epoch {
            return Err(AuthorityError::Fenced("claim observation precedes required migration epoch".into()));
        }
        let mut certificate = ClaimReconciliation {
            policy: self.policy.commitment(),
            observed_epoch: epoch,
            observations: vec![],
            sessions: vec![],
            blocks: vec![],
            signature: vec![],
        };
        let mut sessions = HashSet::from([self.policy.session_id]);
        let mut blocks = BTreeMap::new();
        let mut block_bytes = 0u64;
        for id in ids {
            let status = if let Some(stored) = inner.store.load_handoff(*id).map_err(protocol::session)? {
                if stored.handoff.id() != *id || stored.handoff.source_session_id != self.policy.session_id {
                    return Err(invalid("claim query belongs to another issuer"));
                }
                audit_handoff_lifecycle(inner.store.as_ref(), &stored).map_err(protocol::session)?;
                for at in references(&stored) {
                    sessions.insert(at.session_id);
                    if let std::collections::btree_map::Entry::Vacant(entry) = blocks.entry((at.session_id, at.height)) {
                        let mut block = inner.store.load_block(at.session_id, at.height).map_err(protocol::session)?;
                        block.state_snapshot.clear();
                        let bytes = block.encode().map_err(protocol::session)?;
                        block_bytes = block_bytes
                            .checked_add(bytes.len() as u64)
                            .filter(|n| *n < MAX_EVIDENCE_BYTES)
                            .ok_or_else(|| invalid("claim block evidence exceeds byte bound"))?;
                        entry.insert(bytes);
                    }
                }
                ObservedClaim::Present { record: stored.encode().map_err(protocol::session)? }
            } else {
                ObservedClaim::Missing
            };
            certificate.observations.push(ClaimObservation { id: *id, status });
            bounded_codec::size_with_limit(
                &certificate.observations,
                "claim_evidence_bytes",
                MAX_EVIDENCE_BYTES.saturating_sub(block_bytes),
            )
            .map_err(protocol::session)?;
        }
        let mut sessions = sessions.into_iter().collect::<Vec<_>>();
        sessions.sort();
        for id in sessions {
            certificate.sessions.push(ClaimSession {
                config: inner.store.load_genesis(id).map_err(protocol::session)?.config.encode().map_err(protocol::session)?,
                head: inner.store.load_head(id).map_err(protocol::session)?.encode().map_err(protocol::session)?,
            });
            bounded_codec::size_with_limit(
                &certificate,
                "claim_evidence_bytes",
                (MAX_EVIDENCE_BYTES - 128).saturating_sub(block_bytes),
            )
            .map_err(protocol::session)?;
        }
        certificate.blocks = blocks.into_values().collect();
        bounded_codec::size_with_limit(&certificate, "claim_evidence_bytes", MAX_EVIDENCE_BYTES - 128).map_err(protocol::session)?;
        certificate.signature = inner.keys.certificates.sign(certificate.digest()?);
        Ok(certificate)
    }
}
