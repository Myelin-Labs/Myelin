//! Genesis-bound aggregate limits. Operational CPU/RSS/time limits are separate.
use crate::{BlockRequest, ExecutionOutcome, FinalisedBlockRecord, FrameInput, Hash32, Result, SessionError, SessionHandoff};
use serde::{Deserialize, Serialize};

const DOMAIN: &[u8] = b"myelin:commit-resource-limits:v1";

/// Version-one limits on one complete state transition. Counts and byte totals
/// are independent; a zero count disables that effect. Byte boundaries are inclusive.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitResourceLimits {
    pub version: u16,
    /// Postcard bytes of the entire FrameInput, including command identities.
    pub max_frame_bytes: u64,
    pub max_issued_handoffs: u64,
    /// Combined consumption and cancellation/refund action count.
    pub max_consumed_handoffs: u64,
    /// Combined issued, consumed and lifecycle-witness payload bytes.
    pub max_handoff_payload_bytes: u64,
    /// Combined issued, consumed and lifecycle-witness authorization bytes.
    pub max_authorization_bytes: u64,
    /// All request, executor, outbox, handoff and successor commitments in the block.
    pub max_data_commitments: u64,
    pub max_outbox_messages: u64,
    /// Combined UTF-8 topic and payload bytes, excluding fixed message fields.
    pub max_outbox_bytes: u64,
    pub max_snapshot_bytes: u64,
    /// Complete encoded FinalisedBlockRecord, including its unpruned snapshot,
    /// finality proof and record envelope. Not the sum of database index writes.
    pub max_record_bytes: u64,
}

impl Default for CommitResourceLimits {
    fn default() -> Self {
        Self {
            version: 1,
            max_frame_bytes: 16 * 1024 * 1024,
            max_issued_handoffs: 64,
            max_consumed_handoffs: 64,
            max_handoff_payload_bytes: 16 * 1024 * 1024,
            max_authorization_bytes: 256 * 1024,
            max_data_commitments: 4096,
            max_outbox_messages: 1024,
            max_outbox_bytes: 16 * 1024 * 1024,
            max_snapshot_bytes: 64 * 1024 * 1024,
            max_record_bytes: 128 * 1024 * 1024,
        }
    }
}

impl CommitResourceLimits {
    pub fn validate(&self) -> Result<()> {
        if self.version != 1 || self.max_frame_bytes == 0 || self.max_snapshot_bytes == 0 || self.max_record_bytes == 0 {
            return Err(SessionError::InvalidConfig(
                "resource limits require version 1 and nonzero frame/snapshot/record bounds".into(),
            ));
        }
        for value in [self.max_issued_handoffs, self.max_consumed_handoffs, self.max_data_commitments, self.max_outbox_messages] {
            if value > crate::codec::MAX_SEQUENCE_ITEMS as u64 {
                return Err(SessionError::InvalidConfig("resource count limit exceeds the supported decoder ceiling".into()));
            }
        }
        for value in [
            self.max_frame_bytes,
            self.max_handoff_payload_bytes,
            self.max_authorization_bytes,
            self.max_outbox_bytes,
            self.max_snapshot_bytes,
            self.max_record_bytes,
        ] {
            if value > crate::codec::MAX_RECORD_BYTES as u64 {
                return Err(SessionError::InvalidConfig("resource byte limit exceeds the supported decoder ceiling".into()));
            }
        }
        Ok(())
    }

    pub fn commitment(&self) -> Hash32 {
        let mut h = blake3::Hasher::new();
        h.update(DOMAIN);
        h.update(&self.version.to_le_bytes());
        for value in [
            self.max_frame_bytes,
            self.max_issued_handoffs,
            self.max_consumed_handoffs,
            self.max_handoff_payload_bytes,
            self.max_authorization_bytes,
            self.max_data_commitments,
            self.max_outbox_messages,
            self.max_outbox_bytes,
            self.max_snapshot_bytes,
            self.max_record_bytes,
        ] {
            h.update(&value.to_le_bytes());
        }
        *h.finalize().as_bytes()
    }

    pub(crate) fn frame(&self, frame: &FrameInput) -> Result<()> {
        check("frame_commands", frame.commands.len() as u64, crate::codec::MAX_SEQUENCE_ITEMS as u64)?;
        crate::codec::size_with_limit(frame, "frame_bytes", self.max_frame_bytes).map(|_| ())
    }

    pub(crate) fn request(&self, request: &BlockRequest) -> Result<HandoffUsage> {
        check("issued_handoffs", request.handoffs.len() as u64, self.max_issued_handoffs)?;
        check(
            "consumed_handoffs",
            add("consumed_handoffs", request.consume_handoffs.len() as u64, request.handoff_actions.len() as u64)?,
            self.max_consumed_handoffs,
        )?;
        self.frame(&request.frame_input)?;
        let commitments = [
            request.data_commitments.len() + usize::from(!request.handoff_actions.is_empty()),
            request.handoffs.len(),
            request.consume_handoffs.len(),
            usize::from(request.successor.is_some()),
        ]
        .into_iter()
        .try_fold(2, |sum, n| add("data_commitments", sum, n as u64))?;
        check("data_commitments", commitments, self.max_data_commitments)?;
        let mut usage = HandoffUsage::default();
        for intent in &request.handoffs {
            usage.add(self, intent.payload.len(), intent.authorization.len())?;
            intent.validate()?;
        }
        let predictable = add("record_bytes", usage.payload, usage.authorization)?;
        let predictable = request.transactions.iter().try_fold(predictable, |sum, tx| add("record_bytes", sum, tx.len() as u64))?;
        check("record_bytes", add("record_bytes", predictable, request.frame_input.payload.len() as u64)?, self.max_record_bytes)?;
        Ok(usage)
    }

    pub(crate) fn outcome(&self, request: &BlockRequest, outcome: &ExecutionOutcome) -> Result<()> {
        self.outcome_parts(
            request.data_commitments.len() + usize::from(!request.handoff_actions.is_empty()),
            request.handoffs.len(),
            request.consume_handoffs.len(),
            request.successor.is_some(),
            true,
            outcome,
        )
    }

    pub(crate) fn outcome_parts(
        &self,
        request_commitments: usize,
        issued: usize,
        consumed: usize,
        successor: bool,
        effects_context: bool,
        outcome: &ExecutionOutcome,
    ) -> Result<()> {
        let count =
            [request_commitments, outcome.data_commitments.len(), outcome.outbox.len(), issued, consumed, usize::from(successor)]
                .into_iter()
                .try_fold(1 + u64::from(effects_context), |sum, count| add("data_commitments", sum, count as u64))?;
        check("data_commitments", count, self.max_data_commitments)?;
        check("outbox_messages", outcome.outbox.len() as u64, self.max_outbox_messages)?;
        let bytes = outcome
            .outbox
            .iter()
            .try_fold(0, |sum, m| add("outbox_bytes", add("outbox_bytes", sum, m.topic.len() as u64)?, m.payload.len() as u64))?;
        check("outbox_bytes", bytes, self.max_outbox_bytes)
    }

    pub fn snapshot(&self, bytes: &[u8]) -> Result<()> {
        check("snapshot_bytes", bytes.len() as u64, self.max_snapshot_bytes)
    }

    /// Validate retained record fields before encoding/copying them. Consumed
    /// payloads are checked separately after resolving their source records.
    pub fn record(&self, record: &FinalisedBlockRecord) -> Result<()> {
        let original = record.resource_snapshot_bytes;
        check("snapshot_bytes", original, self.max_snapshot_bytes)?;
        if record.block.data_commitments.last() != Some(&snapshot_commitment(self, original)) {
            return Err(SessionError::Codec("resource budget or snapshot commitment is absent or changed".into()));
        }
        self.frame(&record.frame_input)?;
        check("issued_handoffs", record.issued_handoffs.len() as u64, self.max_issued_handoffs)?;
        check(
            "consumed_handoffs",
            add("consumed_handoffs", record.consumed_handoff_ids.len() as u64, record.handoff_actions.len() as u64)?,
            self.max_consumed_handoffs,
        )?;
        check("data_commitments", record.block.data_commitments.len() as u64, self.max_data_commitments)?;
        check("outbox_messages", record.outbox.len() as u64, self.max_outbox_messages)?;
        let bytes = record
            .outbox
            .iter()
            .try_fold(0, |sum, m| add("outbox_bytes", add("outbox_bytes", sum, m.topic.len() as u64)?, m.payload.len() as u64))?;
        check("outbox_bytes", bytes, self.max_outbox_bytes)?;
        self.snapshot(&record.state_snapshot)?;
        self.handoffs(record.issued_handoffs.iter().chain(record.handoff_actions.iter().map(|a| &a.handoff)))?;
        record.encoded_size_with_limit(self.max_record_bytes).map(|_| ())
    }

    pub fn handoffs<'a>(&self, handoffs: impl IntoIterator<Item = &'a SessionHandoff>) -> Result<HandoffUsage> {
        let mut usage = HandoffUsage::default();
        for handoff in handoffs {
            usage.add(self, handoff.payload.len(), handoff.authorization.len())?;
        }
        Ok(usage)
    }
}

pub(crate) fn snapshot_commitment(limits: &CommitResourceLimits, bytes: u64) -> Hash32 {
    let mut h = blake3::Hasher::new();
    h.update(b"myelin:commit-resource-usage:v1");
    h.update(&limits.commitment());
    h.update(&bytes.to_le_bytes());
    *h.finalize().as_bytes()
}

/// Checked accumulator shared by execution, replay and transactional stores.
#[derive(Default)]
pub struct HandoffUsage {
    payload: u64,
    authorization: u64,
}

impl HandoffUsage {
    pub fn add(&mut self, limits: &CommitResourceLimits, payload: usize, authorization: usize) -> Result<()> {
        self.payload = add("handoff_payload_bytes", self.payload, payload as u64)?;
        check("handoff_payload_bytes", self.payload, limits.max_handoff_payload_bytes)?;
        self.authorization = add("authorization_bytes", self.authorization, authorization as u64)?;
        check("authorization_bytes", self.authorization, limits.max_authorization_bytes)
    }
}

pub(crate) fn check(resource: &'static str, used: u64, limit: u64) -> Result<()> {
    if used > limit {
        Err(SessionError::ResourceLimit { resource, used, limit })
    } else {
        Ok(())
    }
}

fn add(resource: &'static str, left: u64, right: u64) -> Result<u64> {
    left.checked_add(right).ok_or_else(|| SessionError::BlockLimit(format!("{resource} total overflow")))
}
