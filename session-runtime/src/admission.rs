// SPDX-License-Identifier: MIT
//! Local operational admission. Deployments that make publication a consensus
//! validity rule must also bind and enforce that rule in their application.

use myelin_session::{Hash32, SessionHead, SessionStore};
use std::sync::Arc;

/// Checked before execution and again immediately before durable commit.
pub struct AdmissionContext<'a> {
    pub base_head: &'a SessionHead,
    pub transaction_count: usize,
}

pub trait BatchAdmission: Send + Sync {
    fn check(&self, context: &AdmissionContext<'_>) -> Result<(), String>;
}

/// Explicit policy for deployments without external publication requirements.
pub struct UnrestrictedAdmission;
impl BatchAdmission for UnrestrictedAdmission {
    fn check(&self, _: &AdmissionContext<'_>) -> Result<(), String> {
        Ok(())
    }
}

/// A claim about an independently recoverable canonical state, not an L1
/// settlement authorization. None height denotes the session genesis.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryAnchor {
    pub session_id: Hash32,
    pub height: Option<u64>,
    pub block_hash: Hash32,
    pub state_root: Hash32,
}

#[derive(Clone, Debug)]
pub struct RecoveryEvidence {
    pub anchor: RecoveryAnchor,
    pub evidence: Vec<u8>,
}

/// Return locally available evidence. Absence, expiration and retrieval failure
/// must fail closed; a process-health flag is not recovery evidence.
pub trait RecoveryEvidenceSource: Send + Sync {
    fn latest(&self, session_id: Hash32) -> Result<RecoveryEvidence, String>;
}

/// Deployment-owned verifier for complete independent recovery at the exact
/// anchor, including genesis. Must check evidence identity, authenticity,
/// retention/freshness and sufficient data/witness coverage; no boolean bypass.
pub trait RecoveryEvidenceVerifier: Send + Sync {
    fn verify(&self, evidence: &RecoveryEvidence) -> Result<(), String>;
}

/// Bounds *all* state advancement, including transfers of existing assets.
/// The store must be the host's audited canonical store. Proof verification is
/// repeated on each check, so a revoked/expired certificate cannot be cached
/// as permanent permission. Publication and settlement remain separate.
pub struct RecoverabilityAdmission<S: SessionStore> {
    store: Arc<S>,
    source: Arc<dyn RecoveryEvidenceSource>,
    verifier: Arc<dyn RecoveryEvidenceVerifier>,
    max_unpublished_blocks: u64,
}

impl<S: SessionStore> RecoverabilityAdmission<S> {
    pub fn new(
        store: Arc<S>,
        source: Arc<dyn RecoveryEvidenceSource>,
        verifier: Arc<dyn RecoveryEvidenceVerifier>,
        max_unpublished_blocks: u64,
    ) -> Result<Self, String> {
        // This adapter publishes after commit. Zero lag requires a different
        // pre-publication protocol rather than an impossible bootstrap.
        if max_unpublished_blocks == 0 {
            return Err("post-commit publication requires a nonzero lag allowance".to_owned());
        }
        Ok(Self { store, source, verifier, max_unpublished_blocks })
    }
}

impl<S: SessionStore> BatchAdmission for RecoverabilityAdmission<S> {
    fn check(&self, context: &AdmissionContext<'_>) -> Result<(), String> {
        let head = context.base_head;
        if self.store.load_head(head.session_id).map_err(|error| error.to_string())? != *head {
            return Err("admission base is not the durable head".to_owned());
        }
        let evidence = self.source.latest(head.session_id)?;
        let anchor = &evidence.anchor;
        if anchor.session_id != head.session_id || evidence.evidence.is_empty() || evidence.evidence.len() > 16 * 1024 * 1024 {
            return Err("recovery evidence identity or size is invalid".to_owned());
        }
        let recovered_count = match anchor.height {
            Some(height) => {
                if head.finalised_height.is_none_or(|latest| height > latest) {
                    return Err("recovery anchor is ahead of the finalized head".to_owned());
                }
                let record = self.store.load_block(head.session_id, height).map_err(|error| error.to_string())?;
                if record.block.session_id != head.session_id
                    || record.block.number != height
                    || record.block.hash() != anchor.block_hash
                    || record.block.state_root_after != anchor.state_root
                {
                    return Err("recovery anchor does not match canonical history".to_owned());
                }
                height.checked_add(1).ok_or("recovery height overflow")?
            }
            None => {
                let genesis = self.store.load_genesis(head.session_id).map_err(|error| error.to_string())?;
                if anchor.block_hash != [0; 32] || anchor.state_root != genesis.config.initial_state_root {
                    return Err("recovery anchor does not match genesis".to_owned());
                }
                0
            }
        };
        self.verifier.verify(&evidence)?;
        let candidate_count = head.next_height().map_err(|error| error.to_string())?.checked_add(1).ok_or("height overflow")?;
        if candidate_count.checked_sub(recovered_count).ok_or("recovery anchor ahead of candidate")? > self.max_unpublished_blocks {
            return Err("candidate exceeds independently recoverable state lag".to_owned());
        }
        Ok(())
    }
}
