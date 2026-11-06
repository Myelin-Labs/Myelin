//! Canonical command observations after a submission, including durable epoch
//! revocation. A Missing observation is releasable only after a strictly newer
//! epoch and only for the exact previously persisted signed submission.
use crate::{protocol, AuthorityError, CommitAuthorization, FencingPolicy, Result, SessionAuthority};
use myelin_session::{
    bounded_codec, CommandIdentity, CommandReceipt, CommandStatus, FinalisedBlockRecord, Hash32, SessionHead, SessionStore,
    TransitionExecutor,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

pub const MAX_RECONCILIATION_COMMANDS: usize = 256;
const MAX_RECORD_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandObservation {
    pub command: CommandIdentity,
    pub status: ObservedStatus,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObservedStatus {
    Missing,
    Committed { receipt: CommandReceipt, block_index: u32 },
    IdentityConflict { receipt: CommandReceipt, block_index: u32 },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandReconciliation {
    pub policy: Hash32,
    pub submission_digest: Hash32,
    pub submitted_epoch: u64,
    pub observed_epoch: u64,
    #[serde(deserialize_with = "bounded_codec::bounded_bytes")]
    pub observed_head: Vec<u8>,
    #[serde(deserialize_with = "bounded_codec::bounded_vec")]
    pub observations: Vec<CommandObservation>,
    /// Unique canonical block records with snapshots pruned. These retain
    /// original signed snapshot sizes and exact command receipts/outbox data.
    #[serde(deserialize_with = "bounded_codec::transactions")]
    pub blocks: Vec<Vec<u8>>,
    #[serde(deserialize_with = "bounded_codec::bounded_bytes")]
    pub signature: Vec<u8>,
}
pub enum VerifiedObservation {
    Missing(CommandIdentity),
    Committed { receipt: CommandReceipt, record: Arc<FinalisedBlockRecord> },
    IdentityConflict { receipt: CommandReceipt },
}
impl CommandReconciliation {
    pub fn digest(&self) -> Result<Hash32> {
        protocol::hash(
            b"myelin:writer-fence-reconciliation:v1",
            &(
                self.policy,
                self.submission_digest,
                self.submitted_epoch,
                self.observed_epoch,
                &self.observed_head,
                &self.observations,
                &self.blocks,
            ),
        )
    }
    pub fn verify(
        &self,
        policy: &FencingPolicy,
        submission: &CommitAuthorization,
        commands: &[CommandIdentity],
    ) -> Result<Vec<VerifiedObservation>> {
        validate_commands(commands)?;
        if self.policy != policy.commitment()
            || submission.policy != self.policy
            || self.submission_digest != submission.digest()?
            || self.submitted_epoch != submission.epoch
            || self.submitted_epoch == 0
            || self.observed_epoch < self.submitted_epoch
            || self.observations.len() != commands.len()
            || self.blocks.len() > commands.len()
            || self.blocks.iter().try_fold(0usize, |sum, bytes| sum.checked_add(bytes.len())).is_none_or(|n| n > MAX_RECORD_BYTES)
        {
            return Err(AuthorityError::Invalid("reconciliation scope, submission, epoch or size mismatch".into()));
        }
        protocol::verify(policy.certificate_key, self.digest()?, &self.signature)?;
        let head = SessionHead::decode(&self.observed_head).map_err(protocol::session)?;
        if head.session_id != policy.session_id {
            return Err(AuthorityError::Invalid("observation is for another session".into()));
        }
        let blocks = self
            .blocks
            .iter()
            .map(|bytes| FinalisedBlockRecord::decode(bytes).map(Arc::new).map_err(protocol::session))
            .collect::<Result<Vec<_>>>()?;
        let mut seen_blocks = HashSet::new();
        for block in &blocks {
            if block.block.session_id != policy.session_id
                || !seen_blocks.insert(block.block.number)
                || head.finalised_height.is_none_or(|height| block.block.number > height)
                || head.finalised_height == Some(block.block.number) && block.block.hash() != head.block_hash
            {
                return Err(AuthorityError::Invalid("observation block is outside canonical head".into()));
            }
        }
        let mut used = HashSet::new();
        let mut result = vec![];
        for (observed, command) in self.observations.iter().zip(commands) {
            if observed.command != *command {
                return Err(AuthorityError::Invalid("reconciliation command order/payload differs".into()));
            }
            let (receipt, index, committed) = match &observed.status {
                ObservedStatus::Missing => {
                    result.push(VerifiedObservation::Missing(command.clone()));
                    continue;
                }
                ObservedStatus::Committed { receipt, block_index } => (receipt, *block_index as usize, true),
                ObservedStatus::IdentityConflict { receipt, block_index } => (receipt, *block_index as usize, false),
            };
            let block = blocks.get(index).ok_or_else(|| AuthorityError::Invalid("missing observation block".into()))?;
            used.insert(index);
            let position = receipt
                .input_position
                .checked_sub(block.frame_input.start_position)
                .and_then(|i| usize::try_from(i).ok())
                .ok_or_else(|| AuthorityError::Invalid("receipt input position invalid".into()))?;
            if receipt.session_id != policy.session_id
                || receipt.command.id != command.id
                || (receipt.command.payload_hash == command.payload_hash) != committed
                || CommandReceipt::for_record(block, position).map_err(protocol::session)? != *receipt
            {
                return Err(AuthorityError::Invalid("observation receipt/payload does not match exact committed frame".into()));
            }
            result.push(if committed {
                VerifiedObservation::Committed { receipt: receipt.clone(), record: Arc::clone(block) }
            } else {
                VerifiedObservation::IdentityConflict { receipt: receipt.clone() }
            });
        }
        if used.len() != blocks.len() {
            return Err(AuthorityError::Invalid("unreferenced reconciliation block".into()));
        }
        Ok(result)
    }
}
impl<E: TransitionExecutor> SessionAuthority<E> {
    /// One linearizable observation under the same lock used for grants and
    /// commits. Signed evidence remains valid after later epochs: an earlier
    /// revoked epoch never becomes writable again. Later same-ID inclusion is
    /// handled by canonical command uniqueness, not by minting another ID.
    pub fn reconcile_submission(
        &self,
        submission: &CommitAuthorization,
        commands: &[CommandIdentity],
    ) -> Result<CommandReconciliation> {
        validate_commands(commands)?;
        let inner = self.locked()?;
        let submitted = inner
            .ownership
            .grants
            .iter()
            .find(|g| g.epoch == submission.epoch)
            .ok_or_else(|| AuthorityError::Invalid("submission epoch is absent from ownership history".into()))?;
        submission.verify_binding(&self.policy, submitted)?;
        let current = inner.ownership.grants.last().ok_or_else(|| AuthorityError::Fenced("no active writer".into()))?;
        let mut certificate = CommandReconciliation {
            policy: self.policy.commitment(),
            submission_digest: submission.digest()?,
            submitted_epoch: submission.epoch,
            observed_epoch: current.epoch,
            observed_head: inner.chain.head().encode().map_err(protocol::session)?,
            observations: vec![],
            blocks: vec![],
            signature: vec![],
        };
        let mut indices = BTreeMap::new();
        let mut total_bytes = 0usize;
        for command in commands {
            let status = match inner.chain.command_status(command).map_err(protocol::session)? {
                CommandStatus::Missing => ObservedStatus::Missing,
                status => {
                    let (receipt, committed) = match status {
                        CommandStatus::Committed(receipt) => (receipt, true),
                        CommandStatus::IdentityConflict(receipt) => (receipt, false),
                        CommandStatus::Missing => unreachable!(),
                    };
                    let index = match indices.get(&receipt.block_height) {
                        Some(index) => *index,
                        None => {
                            let mut block =
                                inner.store.load_block(self.policy.session_id, receipt.block_height).map_err(protocol::session)?;
                            block.state_snapshot.clear();
                            let bytes = block.encode().map_err(protocol::session)?;
                            total_bytes = total_bytes
                                .checked_add(bytes.len())
                                .filter(|n| *n <= MAX_RECORD_BYTES)
                                .ok_or_else(|| AuthorityError::Invalid("reconciliation records exceed bound".into()))?;
                            let index = certificate.blocks.len() as u32;
                            certificate.blocks.push(bytes);
                            indices.insert(receipt.block_height, index);
                            index
                        }
                    };
                    if committed {
                        ObservedStatus::Committed { receipt, block_index: index }
                    } else {
                        ObservedStatus::IdentityConflict { receipt, block_index: index }
                    }
                }
            };
            certificate.observations.push(CommandObservation { command: command.clone(), status });
        }
        certificate.signature = inner.keys.certificates.sign(certificate.digest()?);
        Ok(certificate)
    }
}
fn validate_commands(commands: &[CommandIdentity]) -> Result<()> {
    let ids: HashSet<_> = commands.iter().map(|c| c.id).collect();
    if commands.is_empty()
        || commands.len() > MAX_RECONCILIATION_COMMANDS
        || ids.len() != commands.len()
        || commands.iter().any(|c| c.id == [0; 32] || c.payload_hash == [0; 32])
    {
        return Err(AuthorityError::Invalid("reconciliation command count/identity invalid".into()));
    }
    Ok(())
}
