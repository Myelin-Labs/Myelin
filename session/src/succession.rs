//! Application-authorized continuation and separately replayable state conversion.
//!
//! A receipt commits evidence; it does not prove that an arbitrary application
//! implemented its policy honestly. Replay runs the source executor's hook again.
use crate::*;

/// Output of the source application's deterministic successor policy. The hook
/// must authenticate the complete target through ProtocolEffectsContext first.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum SuccessorState {
    /// Preserve every state byte and the complete application/effect policy.
    Continue,
    /// Preserve state bytes under an explicitly checked compatibility policy.
    Compatible { policy_hash: Hash32, declaration_hash: Hash32, vectors_hash: Hash32 },
    /// Run a separately identified migration program, including seasonal resets.
    /// Inputs are retained so offline execution does not need mutable services.
    Migrate {
        artifact: MigrationArtifact,
        #[serde(deserialize_with = "crate::codec::bounded_bytes")]
        inputs: Vec<u8>,
        state_root: Hash32,
        #[serde(deserialize_with = "crate::codec::bounded_bytes")]
        snapshot: Vec<u8>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MigrationArtifact {
    pub format_version: u32,
    pub program_digest: Hash32,
    pub input_schema_hash: Hash32,
}

/// Transition receipt carried by the current block format.
/// The enclosing declaration binds source/target profiles, roots, target config,
/// height and reverse lineage. This receipt additionally binds execution inputs
/// and exact source bytes, and retains converted target bytes when necessary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuccessorTransition {
    pub source_genesis_commitment: Hash32,
    pub input_root: Hash32,
    pub source_snapshot_hash: Hash32,
    pub state: SuccessorState,
}

impl SuccessorTransition {
    pub(crate) fn execute<E: TransitionExecutor>(
        executor: &E,
        source: &SessionConfig,
        target: &SessionConfig,
        input: &FrameInput,
        snapshot: &[u8],
    ) -> Result<Self> {
        let transition = Self {
            source_genesis_commitment: source.commitment(),
            input_root: input.root(),
            source_snapshot_hash: *blake3::hash(snapshot).as_bytes(),
            state: executor.prepare_successor_state(source, target, input).map_err(SessionError::Execution)?,
        };
        transition.validate(source, target, executor.state_root(), input.root())?;
        transition.check_source_snapshot(snapshot)?;
        target.commit_limits.snapshot(transition.target_snapshot(snapshot))?;
        Ok(transition)
    }

    pub(crate) fn validate_shape(&self, source_root: Hash32, target_root: Hash32) -> Result<()> {
        if [self.source_genesis_commitment, self.input_root, self.source_snapshot_hash].contains(&[0; 32]) {
            return Err(SessionError::Continuity("successor receipt has a zero binding".into()));
        }
        match &self.state {
            SuccessorState::Continue | SuccessorState::Compatible { .. } if source_root != target_root => {
                return Err(SessionError::Continuity("state-preserving successor changes the state root".into()));
            }
            SuccessorState::Compatible { policy_hash, declaration_hash, vectors_hash }
                if [*policy_hash, *declaration_hash, *vectors_hash].contains(&[0; 32]) =>
            {
                return Err(SessionError::Continuity("compatibility requires policy, declaration and vectors".into()));
            }
            SuccessorState::Migrate { artifact, state_root, snapshot, .. } => {
                if artifact.format_version != 1
                    || artifact.program_digest == [0; 32]
                    || artifact.input_schema_hash == [0; 32]
                    || snapshot.is_empty()
                    || *state_root != target_root
                {
                    return Err(SessionError::Continuity("invalid migration artifact, inputs or target state".into()));
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub fn validate(&self, source: &SessionConfig, target: &SessionConfig, source_root: Hash32, input_root: Hash32) -> Result<()> {
        self.validate_shape(source_root, target.initial_state_root)?;
        if self.source_genesis_commitment != source.commitment() || self.input_root != input_root {
            return Err(SessionError::Continuity("successor receipt source genesis or input mismatch".into()));
        }
        match &self.state {
            SuccessorState::Continue => {
                if source.application_profile != target.application_profile || source.protocol_effects != target.protocol_effects {
                    return Err(SessionError::Continuity(
                        "changed application rules require explicit compatibility or migration".into(),
                    ));
                }
            }
            SuccessorState::Compatible { policy_hash, .. } => {
                if source.protocol_effects != (ProtocolEffectsPolicy::Application { policy_hash: *policy_hash })
                    || source.application_profile.state_codec_hash != target.application_profile.state_codec_hash
                {
                    return Err(SessionError::Continuity(
                        "compatibility requires the source application policy and unchanged codec".into(),
                    ));
                }
            }
            SuccessorState::Migrate { inputs, snapshot, .. } => {
                if !matches!(source.protocol_effects, ProtocolEffectsPolicy::Application { .. }) {
                    return Err(SessionError::Continuity("migration requires a source application policy".into()));
                }
                // Bound both the execution inputs and target bytes before encoding.
                resources::check("migration_inputs", inputs.len() as u64, source.application_profile.resources.max_frame_input_bytes)?;
                target.commit_limits.snapshot(snapshot)?;
            }
        }
        Ok(())
    }

    pub fn check_source_snapshot(&self, snapshot: &[u8]) -> Result<()> {
        if snapshot.is_empty() || *blake3::hash(snapshot).as_bytes() != self.source_snapshot_hash {
            return Err(SessionError::Continuity("successor source snapshot mismatch".into()));
        }
        Ok(())
    }

    pub fn target_snapshot<'a>(&'a self, source_snapshot: &'a [u8]) -> &'a [u8] {
        match &self.state {
            SuccessorState::Migrate { snapshot, .. } => snapshot,
            _ => source_snapshot,
        }
    }

    /// BLAKE3 identifiers of exact external artifacts needed to replay this
    /// transition. Archive exporters must retain these and their dependencies.
    pub fn artifact_dependencies(&self) -> Vec<Hash32> {
        match &self.state {
            SuccessorState::Continue => vec![],
            SuccessorState::Compatible { declaration_hash, vectors_hash, .. } => vec![*declaration_hash, *vectors_hash],
            SuccessorState::Migrate { artifact, .. } => vec![artifact.program_digest],
        }
    }

    pub(crate) fn update_commitment(&self, hasher: &mut blake3::Hasher) {
        // Explicit fixed-width/length-prefixed encoding, independent of serde.
        hasher.update(&self.source_genesis_commitment);
        hasher.update(&self.input_root);
        hasher.update(&self.source_snapshot_hash);
        match &self.state {
            SuccessorState::Continue => {
                hasher.update(&[0]);
            }
            SuccessorState::Compatible { policy_hash, declaration_hash, vectors_hash } => {
                hasher.update(&[1]);
                hasher.update(policy_hash);
                hasher.update(declaration_hash);
                hasher.update(vectors_hash);
            }
            SuccessorState::Migrate { artifact, inputs, state_root, snapshot } => {
                hasher.update(&[2]);
                hasher.update(&artifact.format_version.to_le_bytes());
                hasher.update(&artifact.program_digest);
                hasher.update(&artifact.input_schema_hash);
                hasher.update(&(inputs.len() as u64).to_le_bytes());
                hasher.update(inputs);
                hasher.update(state_root);
                hasher.update(&(snapshot.len() as u64).to_le_bytes());
                hasher.update(snapshot);
            }
        }
    }
}

impl SuccessorDeclaration {
    pub fn validate_source(&self, source: &SessionConfig, input_root: Hash32) -> Result<()> {
        self.validate()?;
        if self.source_session_id != source.session_id
            || self.source_application_profile_commitment != source.application_profile.commitment()
        {
            return Err(SessionError::Continuity("successor source profile mismatch".into()));
        }
        self.transition.validate(source, &self.target_config, self.source_state_root, input_root)
    }

    /// Verify source bytes before returning either copied or converted state.
    pub fn target_snapshot<'a>(&'a self, source_snapshot: &'a [u8]) -> Result<&'a [u8]> {
        self.transition.check_source_snapshot(source_snapshot)?;
        Ok(self.transition.target_snapshot(source_snapshot))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_successor_commitments_match_independently_encoded_vectors() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../fixtures/protocol/successor-transition-v1.json")).unwrap();
        let states = [
            SuccessorState::Continue,
            SuccessorState::Compatible { policy_hash: [4; 32], declaration_hash: [5; 32], vectors_hash: [6; 32] },
            SuccessorState::Migrate {
                artifact: MigrationArtifact { format_version: 1, program_digest: [7; 32], input_schema_hash: [8; 32] },
                inputs: b"abc".to_vec(),
                state_root: [9; 32],
                snapshot: b"next".to_vec(),
            },
        ];
        for (state, vector) in states.into_iter().zip(fixture["vectors"].as_array().unwrap()) {
            let transition =
                SuccessorTransition { source_genesis_commitment: [1; 32], input_root: [2; 32], source_snapshot_hash: [3; 32], state };
            let result = crate::continuity::successor_commitment([11; 32], 5, [12; 32], [13; 32], [14; 32], &transition);
            let expected = vector["commitment"].as_str().unwrap();
            let actual: String = result.iter().map(|byte| format!("{byte:02x}")).collect();
            assert_eq!(actual, expected);
            let hex = vector["preimage_hex"].as_str().unwrap();
            let bytes: Vec<_> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
            assert_eq!(blake3::hash(&bytes).to_hex().as_str(), vector["commitment"].as_str().unwrap());
        }
    }
}
