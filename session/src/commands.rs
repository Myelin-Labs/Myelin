// SPDX-License-Identifier: MIT
//! Durable application command identities. Authorization and payload interpretation
//! remain application responsibilities; transaction reconstruction cannot change
//! the identity of an already committed command.

use crate::{
    decode_record, encode_record, FinalisedBlockRecord, Hash32, Result, SessionChain, SessionError, SessionStore, TransitionExecutor,
};
use serde::{Deserialize, Serialize};

/// Application-scoped identity and digest of the original signed command.
/// The application must authenticate both fields against the frame payload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandIdentity {
    pub id: Hash32,
    pub payload_hash: Hash32,
}

/// Inclusion in a durably committed execution frame, not proof of an external effect.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandReceipt {
    pub session_id: Hash32,
    pub command: CommandIdentity,
    pub input_position: u64,
    pub block_height: u64,
    pub block_hash: Hash32,
    pub frame_commitment: Hash32,
}

impl CommandReceipt {
    pub fn for_record(record: &FinalisedBlockRecord, index: usize) -> Result<Self> {
        let command = record
            .frame_input
            .commands
            .get(index)
            .ok_or_else(|| SessionError::InvalidFrame("command index outside frame".to_owned()))?;
        Ok(Self {
            session_id: record.block.session_id,
            command: command.clone(),
            input_position: record
                .frame_input
                .start_position
                .checked_add(index as u64)
                .ok_or_else(|| SessionError::InvalidFrame("command position overflow".to_owned()))?,
            block_height: record.block.number,
            block_hash: record.block.hash(),
            frame_commitment: record.frame.commitment(),
        })
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        encode_record(self)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        decode_record(bytes)
    }
}

/// Reconciliation does not resubmit anything. Missing means not committed at
/// the queried head, not safe to release funds or race another writer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandStatus {
    Missing,
    Committed(CommandReceipt),
    IdentityConflict(CommandReceipt),
}

impl<E: TransitionExecutor, S: SessionStore> SessionChain<E, S> {
    /// Resolve an application journal entry against this audited session head.
    pub fn command_status(&self, command: &CommandIdentity) -> Result<CommandStatus> {
        let Some(receipt) = self.store.load_command_receipt(self.config.session_id, command.id)? else {
            return Ok(CommandStatus::Missing);
        };
        if receipt.session_id != self.config.session_id
            || receipt.command.id != command.id
            || self.head.finalised_height.is_none_or(|height| receipt.block_height > height)
        {
            return Err(SessionError::Recovery("command receipt is outside the current session head".to_owned()));
        }
        let record = self.store.load_block(self.config.session_id, receipt.block_height)?;
        let index = receipt
            .input_position
            .checked_sub(record.frame_input.start_position)
            .and_then(|index| usize::try_from(index).ok())
            .ok_or_else(|| SessionError::Recovery("command receipt position mismatch".to_owned()))?;
        if CommandReceipt::for_record(&record, index)? != receipt {
            return Err(SessionError::Recovery("command receipt does not match committed frame".to_owned()));
        }
        Ok(if receipt.command.payload_hash == command.payload_hash {
            CommandStatus::Committed(receipt)
        } else {
            CommandStatus::IdentityConflict(receipt)
        })
    }
}
