//! Borrowed record serialization: budget checks never clone transaction,
//! handoff, outbox or snapshot buffers just to learn their encoded size.
use crate::*;

#[derive(Serialize)]
struct BlockRef<'a> {
    version: u32,
    session_id: &'a Hash32,
    parent_hash: &'a Hash32,
    number: u64,
    timestamp_ms: u64,
    consensus_kind: &'static str,
    application_profile_commitment: &'a Hash32,
    execution_frame_commitment: &'a Hash32,
    state_root_before: &'a Hash32,
    state_root_after: &'a Hash32,
    ordered_cell_tx_commitments: &'a [Hash32],
    data_commitments: &'a [Hash32],
    scheduler_commitment: &'a Hash32,
}

#[derive(Serialize)]
struct RecordRef<'a> {
    block: BlockRef<'a>,
    consensus_module_commitment: &'a Hash32,
    proof: &'a [u8],
    transactions: &'a [Vec<u8>],
    state_snapshot: &'a [u8],
    outbox: &'a [OutboxMessage],
    frame: &'a ExecutionFrame,
    frame_input: &'a FrameInput,
    successor: Option<SuccessorDeclarationWire>,
    issued_handoffs: &'a [SessionHandoff],
    consumed_handoff_ids: &'a [Hash32],
    request_data_commitments: &'a [Hash32],
    snapshot_bytes: u64,
    handoff_actions: &'a [ResolvedHandoffAction],
    remote_terminals: &'a [RemoteTerminalCertificate],
}

impl FinalisedBlockRecord {
    fn wire_ref<'a>(&'a self, proof: &'a [u8]) -> RecordRef<'a> {
        RecordRef {
            block: BlockRef::from(&self.block),
            consensus_module_commitment: &self.consensus_module_commitment,
            proof,
            transactions: &self.transactions,
            state_snapshot: &self.state_snapshot,
            outbox: &self.outbox,
            frame: &self.frame,
            frame_input: &self.frame_input,
            successor: self.successor.as_ref().map(SuccessorDeclarationWire::from),
            issued_handoffs: &self.issued_handoffs,
            consumed_handoff_ids: &self.consumed_handoff_ids,
            request_data_commitments: &self.request_data_commitments,
            snapshot_bytes: self.resource_snapshot_bytes,
            handoff_actions: &self.handoff_actions,
            remote_terminals: &self.remote_terminals,
        }
    }

    pub(crate) fn check_successor_format(&self) -> Result<()> {
        if self.block.version != SESSION_BLOCK_VERSION {
            return Err(SessionError::Codec("unsupported block format or missing snapshot size".into()));
        }
        if let Some(successor) = &self.successor {
            successor.validate()?;
        }
        Ok(())
    }

    pub(crate) fn encode_borrowed(&self) -> Result<Vec<u8>> {
        self.check_successor_format()?;
        let proof = self.proof.encode()?;
        encode_record(&self.wire_ref(&proof))
    }

    pub(crate) fn retained_size_with_limit(&self, limit: u64) -> Result<u64> {
        self.check_successor_format()?;
        let proof = self.proof.encode()?;
        record_size(&self.wire_ref(&proof), limit)
    }

    /// Size of the complete original record, including a pruned snapshot.
    pub fn encoded_size_with_limit(&self, limit: u64) -> Result<u64> {
        self.check_successor_format()?;
        let proof = self.proof.encode()?;
        let mut size = record_size(&self.wire_ref(&proof), limit)?;
        {
            let original = self.resource_snapshot_bytes;
            if self.state_snapshot.is_empty() && original != 0 {
                let prefix = (64 - original.leading_zeros() as u64).div_ceil(7).max(1);
                size = size
                    .checked_add(original)
                    .and_then(|n| n.checked_add(prefix - 1))
                    .ok_or_else(|| SessionError::BlockLimit("original record size overflow".into()))?;
            } else if original != self.state_snapshot.len() as u64 {
                return Err(SessionError::Codec("snapshot length does not match committed resource usage".into()));
            }
        }
        resources::check("record_bytes", size, limit)?;
        Ok(size)
    }
}

impl<'a> From<&'a MyelinBlock> for BlockRef<'a> {
    fn from(b: &'a MyelinBlock) -> Self {
        Self {
            version: b.version,
            session_id: &b.session_id,
            parent_hash: &b.parent_hash,
            number: b.number,
            timestamp_ms: b.timestamp_ms,
            consensus_kind: b.consensus_kind.as_str(),
            application_profile_commitment: &b.application_profile_commitment,
            execution_frame_commitment: &b.execution_frame_commitment,
            state_root_before: &b.state_root_before,
            state_root_after: &b.state_root_after,
            ordered_cell_tx_commitments: &b.ordered_cell_tx_commitments,
            data_commitments: &b.data_commitments,
            scheduler_commitment: &b.scheduler_commitment,
        }
    }
}

impl<E: TransitionExecutor> PreparedBlock<E> {
    pub(crate) fn check_record_size(&self, module: &Hash32, proof: &[u8], limit: u64) -> Result<()> {
        let consumed_ids = self.consumed_handoffs.iter().map(SessionHandoff::id).collect::<Vec<_>>();
        let record = RecordRef {
            block: BlockRef::from(&self.block),
            consensus_module_commitment: module,
            proof,
            transactions: &self.transactions,
            state_snapshot: &self.snapshot,
            outbox: &self.outbox,
            frame: &self.frame,
            frame_input: &self.frame_input,
            successor: self.successor.as_ref().map(SuccessorDeclarationWire::from),
            issued_handoffs: &self.issued_handoffs,
            consumed_handoff_ids: &consumed_ids,
            request_data_commitments: &self.request_data_commitments,
            snapshot_bytes: self.snapshot.len() as u64,
            handoff_actions: &self.handoff_actions,
            remote_terminals: &self.remote_terminals,
        };
        record_size(&record, limit)?;
        Ok(())
    }
}

pub(crate) fn record_size<T: Serialize>(body: &T, limit: u64) -> Result<u64> {
    let header = RECORD_HEADER_BYTES as u64;
    resources::check("record_bytes", header, limit)?;
    codec::size_with_limit(body, "record_bytes", limit - header).map(|bytes| header + bytes).map_err(|error| match error {
        SessionError::ResourceLimit { used, .. } => {
            SessionError::ResourceLimit { resource: "record_bytes", used: used + header, limit }
        }
        other => other,
    })
}
