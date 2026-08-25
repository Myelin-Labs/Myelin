// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers

//! Ordered, locally verified evidence receipt chains for durable outbox work.

use crate::{decode_record, encode_record, Hash32, OutboxMessage, Result, SessionError};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

const PIPELINE_DOMAIN: &[u8] = b"myelin:evidence-pipeline";
const RECEIPT_DOMAIN: &[u8] = b"myelin:evidence-receipt";
const MAX_PIPELINE_NAME_BYTES: usize = 128;
const MAX_STAGE_NAME_BYTES: usize = 128;
const MAX_EVIDENCE_STAGES: usize = 64;
const MAX_EVIDENCE_BYTES: usize = 16 * 1024 * 1024;

/// One ordered evidence stage and the exact local verifier implementation
/// identity required to accept it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceStage {
    pub name: String,
    pub verifier_commitment: Hash32,
}

/// Immutable receipt ladder selected for an outbox topic.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidencePipelineDescriptor {
    pub pipeline_name: String,
    pub topic: String,
    pub stages: Vec<EvidenceStage>,
}

impl EvidencePipelineDescriptor {
    pub fn new(pipeline_name: impl Into<String>, topic: impl Into<String>, stages: Vec<EvidenceStage>) -> Result<Self> {
        let descriptor = Self { pipeline_name: pipeline_name.into(), topic: topic.into(), stages };
        descriptor.validate()?;
        Ok(descriptor)
    }

    /// Exact CKB claim ladder. Each step remains distinct: node acceptance is
    /// not commitment, and configured confirmation depth is not irreversibility.
    pub fn ckb_submission(topic: impl Into<String>, verifier_commitments: [Hash32; 7]) -> Result<Self> {
        let names = [
            "wire-encoded",
            "context-resolved",
            "consensus-validated",
            "scripts-verified",
            "node-accepted",
            "committed",
            "configured-depth-finality",
        ];
        Self::new(
            "ckb-submission",
            topic,
            names
                .into_iter()
                .zip(verifier_commitments)
                .map(|(name, verifier_commitment)| EvidenceStage { name: name.to_owned(), verifier_commitment })
                .collect(),
        )
    }

    pub fn data_availability(topic: impl Into<String>, verifier_commitments: [Hash32; 2]) -> Result<Self> {
        Self::new(
            "data-availability",
            topic,
            ["payload-committed", "availability-verified"]
                .into_iter()
                .zip(verifier_commitments)
                .map(|(name, verifier_commitment)| EvidenceStage { name: name.to_owned(), verifier_commitment })
                .collect(),
        )
    }

    pub fn court(topic: impl Into<String>, verifier_commitments: [Hash32; 3]) -> Result<Self> {
        Self::new(
            "court",
            topic,
            ["case-opened", "replay-verified", "verdict-finalised"]
                .into_iter()
                .zip(verifier_commitments)
                .map(|(name, verifier_commitment)| EvidenceStage { name: name.to_owned(), verifier_commitment })
                .collect(),
        )
    }

    pub fn settlement(topic: impl Into<String>, verifier_commitments: [Hash32; 4]) -> Result<Self> {
        Self::new(
            "settlement",
            topic,
            ["intent-validated", "submitted", "committed", "configured-depth-finality"]
                .into_iter()
                .zip(verifier_commitments)
                .map(|(name, verifier_commitment)| EvidenceStage { name: name.to_owned(), verifier_commitment })
                .collect(),
        )
    }

    pub fn validate(&self) -> Result<()> {
        if self.pipeline_name.is_empty() || self.pipeline_name.len() > MAX_PIPELINE_NAME_BYTES {
            return Err(SessionError::Evidence(format!("pipeline name length must be 1..={MAX_PIPELINE_NAME_BYTES} bytes")));
        }
        if self.topic.is_empty() || self.topic.len() > super::MAX_OUTBOX_TOPIC_BYTES {
            return Err(SessionError::Evidence("pipeline topic is empty or too long".to_owned()));
        }
        if self.stages.is_empty() || self.stages.len() > MAX_EVIDENCE_STAGES {
            return Err(SessionError::Evidence(format!("evidence stage count must be 1..={MAX_EVIDENCE_STAGES}")));
        }
        let mut names = HashSet::with_capacity(self.stages.len());
        for stage in &self.stages {
            if stage.name.is_empty() || stage.name.len() > MAX_STAGE_NAME_BYTES || !names.insert(stage.name.as_str()) {
                return Err(SessionError::Evidence("evidence stage names must be bounded, non-empty, and unique".to_owned()));
            }
            if stage.verifier_commitment == [0; 32] {
                return Err(SessionError::Evidence("evidence verifier commitment must not be zero".to_owned()));
            }
        }
        Ok(())
    }

    pub fn commitment(&self) -> Hash32 {
        let mut hasher = blake3::Hasher::new();
        hasher.update(PIPELINE_DOMAIN);
        put_string(&mut hasher, &self.pipeline_name);
        put_string(&mut hasher, &self.topic);
        hasher.update(&(self.stages.len() as u32).to_le_bytes());
        for stage in &self.stages {
            put_string(&mut hasher, &stage.name);
            hasher.update(&stage.verifier_commitment);
        }
        *hasher.finalize().as_bytes()
    }
}

/// Evidence bytes accepted by the exact verifier assigned to one stage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceReceipt {
    pub stage_index: u16,
    pub stage_name: String,
    pub verifier_commitment: Hash32,
    pub previous_receipt_commitment: Hash32,
    pub evidence: Vec<u8>,
}

impl EvidenceReceipt {
    pub fn new(
        descriptor: &EvidencePipelineDescriptor,
        stage_index: usize,
        previous_receipt_commitment: Hash32,
        evidence: Vec<u8>,
    ) -> Result<Self> {
        descriptor.validate()?;
        let stage = descriptor
            .stages
            .get(stage_index)
            .ok_or_else(|| SessionError::Evidence("evidence stage index is outside the pipeline".to_owned()))?;
        if evidence.is_empty() || evidence.len() > MAX_EVIDENCE_BYTES {
            return Err(SessionError::Evidence(format!("evidence bytes must be 1..={MAX_EVIDENCE_BYTES}")));
        }
        Ok(Self {
            stage_index: u16::try_from(stage_index).map_err(|_| SessionError::Evidence("stage index overflow".to_owned()))?,
            stage_name: stage.name.clone(),
            verifier_commitment: stage.verifier_commitment,
            previous_receipt_commitment,
            evidence,
        })
    }

    pub fn commitment(&self, pipeline_commitment: Hash32, message_commitment: Hash32) -> Hash32 {
        let mut hasher = blake3::Hasher::new();
        hasher.update(RECEIPT_DOMAIN);
        hasher.update(&pipeline_commitment);
        hasher.update(&message_commitment);
        hasher.update(&self.stage_index.to_le_bytes());
        put_string(&mut hasher, &self.stage_name);
        hasher.update(&self.verifier_commitment);
        hasher.update(&self.previous_receipt_commitment);
        hasher.update(&(self.evidence.len() as u64).to_le_bytes());
        hasher.update(&self.evidence);
        *hasher.finalize().as_bytes()
    }
}

/// Durable monotonic receipt chain for one exact outbox message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRecord {
    pub session_id: Hash32,
    pub message_id: Hash32,
    pub message_commitment: Hash32,
    pub descriptor: EvidencePipelineDescriptor,
    pub receipts: Vec<EvidenceReceipt>,
    pub revision: u64,
}

impl EvidenceRecord {
    pub fn append(
        current: Option<&Self>,
        session_id: Hash32,
        message: &OutboxMessage,
        descriptor: &EvidencePipelineDescriptor,
        receipt: EvidenceReceipt,
    ) -> Result<Self> {
        descriptor.validate()?;
        message.validate()?;
        if message.topic != descriptor.topic {
            return Err(SessionError::Evidence("outbox topic does not match evidence pipeline".to_owned()));
        }
        let message_commitment = message.commitment();
        let (mut receipts, revision) = match current {
            Some(record) => {
                record.validate_for(session_id, message, descriptor)?;
                (
                    record.receipts.clone(),
                    record.revision.checked_add(1).ok_or_else(|| SessionError::Evidence("evidence revision overflow".to_owned()))?,
                )
            }
            None => (Vec::new(), 0),
        };
        let expected_index = receipts.len();
        let expected_previous =
            receipts.last().map_or([0; 32], |previous| previous.commitment(descriptor.commitment(), message_commitment));
        validate_receipt(descriptor, &receipt, expected_index, expected_previous)?;
        receipts.push(receipt);
        let record =
            Self { session_id, message_id: message.id, message_commitment, descriptor: descriptor.clone(), receipts, revision };
        record.validate_for(session_id, message, descriptor)?;
        Ok(record)
    }

    pub fn next_stage(&self) -> Option<&EvidenceStage> {
        self.descriptor.stages.get(self.receipts.len())
    }

    pub fn is_complete(&self) -> bool {
        self.receipts.len() == self.descriptor.stages.len()
    }

    pub fn latest_receipt(&self) -> Option<&EvidenceReceipt> {
        self.receipts.last()
    }

    pub fn validate_for(&self, session_id: Hash32, message: &OutboxMessage, descriptor: &EvidencePipelineDescriptor) -> Result<()> {
        self.validate_intrinsic()?;
        if self.session_id != session_id
            || self.message_id != message.id
            || self.message_commitment != message.commitment()
            || &self.descriptor != descriptor
        {
            return Err(SessionError::Evidence("evidence record identity, descriptor, or revision mismatch".to_owned()));
        }
        Ok(())
    }

    fn validate_intrinsic(&self) -> Result<()> {
        self.descriptor.validate()?;
        if self.session_id == [0; 32]
            || self.message_id == [0; 32]
            || self.message_commitment == [0; 32]
            || self.receipts.is_empty()
            || self.receipts.len() > self.descriptor.stages.len()
            || self.revision != self.receipts.len() as u64 - 1
        {
            return Err(SessionError::Evidence("evidence record identity, receipt count, or revision is invalid".to_owned()));
        }
        let pipeline_commitment = self.descriptor.commitment();
        let mut previous = [0; 32];
        for (index, receipt) in self.receipts.iter().enumerate() {
            validate_receipt(&self.descriptor, receipt, index, previous)?;
            previous = receipt.commitment(pipeline_commitment, self.message_commitment);
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        encode_record(self)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let record: Self = decode_record(bytes)?;
        record.validate_intrinsic()?;
        Ok(record)
    }
}

fn validate_receipt(
    descriptor: &EvidencePipelineDescriptor,
    receipt: &EvidenceReceipt,
    expected_index: usize,
    expected_previous: Hash32,
) -> Result<()> {
    let stage = descriptor
        .stages
        .get(expected_index)
        .ok_or_else(|| SessionError::Evidence("cannot append beyond the terminal evidence stage".to_owned()))?;
    if receipt.stage_index as usize != expected_index
        || receipt.stage_name != stage.name
        || receipt.verifier_commitment != stage.verifier_commitment
        || receipt.previous_receipt_commitment != expected_previous
        || receipt.evidence.is_empty()
        || receipt.evidence.len() > MAX_EVIDENCE_BYTES
    {
        return Err(SessionError::Evidence("evidence receipt skipped a stage or does not match its verifier/linkage".to_owned()));
    }
    Ok(())
}

fn put_string(hasher: &mut blake3::Hasher, value: &str) {
    hasher.update(&(value.len() as u32).to_le_bytes());
    hasher.update(value.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message() -> OutboxMessage {
        OutboxMessage { id: [1; 32], consensus_module_commitment: [2; 32], topic: "ckb/submit".to_owned(), payload: vec![3] }
    }

    #[test]
    fn ckb_receipts_cannot_skip_or_break_links() {
        let descriptor = EvidencePipelineDescriptor::ckb_submission("ckb/submit", [[9; 32]; 7]).unwrap();
        let message = message();
        let first = EvidenceReceipt::new(&descriptor, 0, [0; 32], vec![1]).unwrap();
        let record = EvidenceRecord::append(None, [8; 32], &message, &descriptor, first).unwrap();
        let skipped = EvidenceReceipt::new(&descriptor, 2, [0; 32], vec![2]).unwrap();
        assert!(EvidenceRecord::append(Some(&record), [8; 32], &message, &descriptor, skipped).is_err());
        let wrong_link = EvidenceReceipt::new(&descriptor, 1, [7; 32], vec![2]).unwrap();
        assert!(EvidenceRecord::append(Some(&record), [8; 32], &message, &descriptor, wrong_link).is_err());
    }

    #[test]
    fn evidence_record_binds_the_exact_message_and_pipeline() {
        let descriptor = EvidencePipelineDescriptor::ckb_submission("ckb/submit", [[9; 32]; 7]).unwrap();
        let message = message();
        let first = EvidenceReceipt::new(&descriptor, 0, [0; 32], vec![1]).unwrap();
        let record = EvidenceRecord::append(None, [8; 32], &message, &descriptor, first).unwrap();
        let mut changed = message.clone();
        changed.payload.push(4);
        assert!(record.validate_for([8; 32], &changed, &descriptor).is_err());
        let decoded = EvidenceRecord::decode(&record.encode().unwrap()).unwrap();
        assert_eq!(decoded, record);
    }
}
