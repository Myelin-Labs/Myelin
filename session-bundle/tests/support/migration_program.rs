//! Current-only bytecode fixture shared by live construction and the archived
//! sandbox runner. This append-only application has no balances or custody claim;
//! the signed reward-ledger suite separately covers ownership and nullifiers.
use myelin_session::{
    ExecutionOutcome, FrameInput, Hash32, MigrationArtifact, ProtocolEffectsContext, ProtocolEffectsPolicy, SessionConfig,
    SessionHandoff, SuccessorState, TransitionExecutor,
};

// Magic, format 1, append-input opcode. No host paths or external services.
pub const PROGRAM: &[u8] = b"MMIG\x01\x01";
pub fn digest(bytes: &[u8]) -> Hash32 {
    *blake3::hash(bytes).as_bytes()
}
pub fn policy() -> ProtocolEffectsPolicy {
    ProtocolEffectsPolicy::Application { policy_hash: digest(PROGRAM) }
}
pub fn target_codec() -> Hash32 {
    digest(b"migration-fixture:appended-state:v1")
}

#[derive(Clone)]
pub struct Converting {
    snapshot: Vec<u8>,
    program: Vec<u8>,
    // A dishonest live executor can produce a signed, structurally valid claim.
    // Only the parent test enables this; the archived interpreter always derives
    // the actual bytecode output, proving that finality alone is insufficient.
    forge_output: bool,
}
impl Converting {
    pub fn restore(snapshot: &[u8], program: Vec<u8>, forge_output: bool) -> Result<Self, String> {
        if program != PROGRAM || snapshot.is_empty() || snapshot.len() > 1024 {
            return Err("unsupported migration program or snapshot".into());
        }
        Ok(Self { snapshot: snapshot.to_vec(), program, forge_output })
    }
    fn convert(&self, input: &[u8]) -> Result<Vec<u8>, String> {
        if input.is_empty() || input.len() > 64 || self.snapshot.len() + input.len() > 1024 {
            return Err("migration input/output exceeds fixture bounds".into());
        }
        let mut snapshot = self.snapshot.clone();
        match self.program[5] {
            1 => snapshot.extend(input),
            _ => return Err("unsupported migration opcode".into()),
        }
        if self.forge_output {
            snapshot.push(b'!');
        }
        Ok(snapshot)
    }
}
impl TransitionExecutor for Converting {
    fn state_root(&self) -> Hash32 {
        digest(&self.snapshot)
    }
    fn snapshot(&self) -> Result<Vec<u8>, String> {
        Ok(self.snapshot.clone())
    }
    fn execute_block(&mut self, _: u64, _: &FrameInput, _: &[SessionHandoff], _: &[Vec<u8>]) -> Result<ExecutionOutcome, String> {
        Err("migration requires its committed source context".into())
    }
    fn execute_block_with_context(
        &mut self,
        context: &ProtocolEffectsContext,
        frame: &FrameInput,
        consumed: &[SessionHandoff],
        transactions: &[Vec<u8>],
    ) -> Result<ExecutionOutcome, String> {
        context.validate().map_err(|e| e.to_string())?;
        if context.policy != policy()
            || context.pre_state_root != self.state_root()
            || context.input_root != frame.root()
            || context.successor_config.is_empty()
            || !context.issued_handoffs.is_empty()
            || !consumed.is_empty()
            || !transactions.is_empty()
        {
            return Err("unsupported migration context".into());
        }
        Ok(ExecutionOutcome { ordered_cell_tx_commitments: vec![], data_commitments: vec![], outbox: vec![], cycles: 0 })
    }
    fn prepare_successor_state(
        &self,
        source: &SessionConfig,
        target: &SessionConfig,
        frame: &FrameInput,
    ) -> Result<SuccessorState, String> {
        let mut profile = source.application_profile.clone();
        profile.state_codec_hash = target_codec();
        if source.protocol_effects != policy()
            || target.protocol_effects != source.protocol_effects
            || target.application_profile != profile
            || target.consensus_module_commitment != source.consensus_module_commitment
            || target.consensus_config_commitment != source.consensus_config_commitment
        {
            return Err("unsupported migration target".into());
        }
        let snapshot = self.convert(&frame.payload)?;
        if digest(&snapshot) != target.initial_state_root {
            return Err("migration program output differs from target root".into());
        }
        Ok(SuccessorState::Migrate {
            artifact: MigrationArtifact {
                format_version: 1,
                program_digest: digest(&self.program),
                input_schema_hash: digest(b"migration-fixture:bounded-append-input:v1"),
            },
            inputs: frame.payload.clone(),
            state_root: digest(&snapshot),
            snapshot,
        })
    }
}
