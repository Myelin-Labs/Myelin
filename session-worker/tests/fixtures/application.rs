use myelin_session::{
    ApplicationProfile, ApplicationResourceEnvelope, ApplicationVmProfile, ExecutionOutcome, FrameInput, Hash32, PendingOutboxMessage,
    SessionHandoff, TransitionExecutor,
};

pub fn profile() -> ApplicationProfile {
    ApplicationProfile {
        command_identity_required: true,
        application_id: "bounded-worker-fixture".into(),
        program_digest: [11; 32],
        input_schema_hash: [12; 32],
        state_codec_hash: [13; 32],
        logical_time_policy_hash: [14; 32],
        entropy_policy_hash: [15; 32],
        vm: ApplicationVmProfile { spawn_ipc_required: false },
        resources: ApplicationResourceEnvelope {
            max_frame_inputs: 1000,
            max_frame_input_bytes: 1_000_000,
            max_frame_cycles: 1_000_000,
            max_logical_time_span: 1000,
            max_inspect_query_bytes: 1024,
            max_inspect_result_bytes: 1024,
        },
        court_profile_hash: [16; 32],
        handoff_policy_hash: [17; 32],
    }
}

#[derive(Clone)]
pub struct Application {
    pub root: Hash32,
}
impl Application {
    pub fn restore(snapshot: &[u8]) -> Result<Self, String> {
        Ok(Self { root: snapshot.try_into().map_err(|_| "snapshot must be a root")? })
    }
}
impl TransitionExecutor for Application {
    fn state_root(&self) -> Hash32 {
        self.root
    }
    fn snapshot(&self) -> Result<Vec<u8>, String> {
        Ok(self.root.to_vec())
    }
    fn execute_block_with_context(
        &mut self,
        context: &myelin_session::ProtocolEffectsContext,
        input: &FrameInput,
        handoffs: &[myelin_session::SessionHandoff],
        transactions: &[Vec<u8>],
    ) -> Result<ExecutionOutcome, String> {
        context.require_authority_metadata([99; 32])?;
        self.execute_block(context.height, input, handoffs, transactions)
    }
    fn execute_block(
        &mut self,
        height: u64,
        input: &FrameInput,
        handoffs: &[SessionHandoff],
        transactions: &[Vec<u8>],
    ) -> Result<ExecutionOutcome, String> {
        if !transactions.is_empty() {
            return Err("fixture only accepts host commands".into());
        }
        let mut hash = blake3::Hasher::new();
        hash.update(&self.root);
        hash.update(&height.to_le_bytes());
        hash.update(&input.root());
        for handoff in handoffs {
            hash.update(&handoff.id());
        }
        self.root = *hash.finalize().as_bytes();
        Ok(ExecutionOutcome {
            ordered_cell_tx_commitments: vec![],
            data_commitments: vec![],
            outbox: vec![PendingOutboxMessage { topic: "fixture/result".into(), payload: self.root.to_vec() }],
            cycles: 0,
        })
    }
}
