// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers

//! Genesis-bound application identity, execution frames, and read-only
//! inspection for a Myelin session.

use crate::{Hash32, Result, SessionError};
use serde::{Deserialize, Serialize};

const APPLICATION_PROFILE_DOMAIN: &[u8] = b"myelin:application-profile";
const FRAME_INPUT_DOMAIN: &[u8] = b"myelin:execution-frame-input";
const EXECUTION_FRAME_DOMAIN: &[u8] = b"myelin:execution-frame";
const INSPECT_QUERY_DOMAIN: &[u8] = b"myelin:inspect-query";
const INSPECT_RESULT_DOMAIN: &[u8] = b"myelin:inspect-result";
const MAX_APPLICATION_ID_BYTES: usize = 256;

/// The only VM semantics accepted by Myelin session and court paths.
///
/// Spawn/IPC is a committed capability choice within strict CKB semantics; it
/// does not select a different VM or relax syscall/source validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationVmProfile {
    /// Whether strict CKB spawn/IPC syscalls are required by the application.
    pub spawn_ipc_required: bool,
}

impl ApplicationVmProfile {
    const fn commitment_byte(self) -> u8 {
        if self.spawn_ipc_required {
            1
        } else {
            0
        }
    }
}

/// Hard application-level envelope checked independently of block limits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationResourceEnvelope {
    /// Maximum number of application inputs covered by one execution frame.
    pub max_frame_inputs: u64,
    /// Maximum canonical application-input payload bytes in one frame.
    pub max_frame_input_bytes: u64,
    /// Maximum aggregate CKB-VM cycles reported for one frame.
    pub max_frame_cycles: u64,
    /// Maximum logical-time distance covered by one frame.
    pub max_logical_time_span: u64,
    /// Maximum bytes accepted by one read-only inspection query.
    pub max_inspect_query_bytes: u64,
    /// Maximum bytes returned by one read-only inspection.
    pub max_inspect_result_bytes: u64,
}

impl ApplicationResourceEnvelope {
    /// Reject an envelope that would make any operation unbounded or unusable.
    pub fn validate(&self) -> Result<()> {
        if self.max_frame_inputs == 0
            || self.max_frame_input_bytes == 0
            || self.max_frame_cycles == 0
            || self.max_logical_time_span == 0
            || self.max_inspect_query_bytes == 0
            || self.max_inspect_result_bytes == 0
        {
            return Err(SessionError::InvalidConfig("application resource-envelope limits must all be non-zero".to_owned()));
        }
        Ok(())
    }
}

/// Complete immutable interpretation of application state for one session.
///
/// The hashes identify canonical artifacts or policy documents. The full
/// artifacts remain application-owned; genesis binds their exact digests.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationProfile {
    /// Stable human-readable application identity, committed verbatim.
    pub application_id: String,
    /// Exact replayer/program artifact digest.
    pub program_digest: Hash32,
    /// Canonical command/tape schema digest.
    pub input_schema_hash: Hash32,
    /// Canonical state codec digest.
    pub state_codec_hash: Hash32,
    /// Logical-time policy document digest.
    pub logical_time_policy_hash: Hash32,
    /// RNG/entropy policy document digest.
    pub entropy_policy_hash: Hash32,
    /// Strict CKB VM capability profile.
    pub vm: ApplicationVmProfile,
    /// Hard per-frame and inspection bounds.
    pub resources: ApplicationResourceEnvelope,
    /// Court/replay procedure digest.
    pub court_profile_hash: Hash32,
    /// Deterministic policy interpreted by the bound application when it
    /// accepts cross-session handoffs.
    pub handoff_policy_hash: Hash32,
}

impl ApplicationProfile {
    /// Validate a complete, fail-closed application identity.
    pub fn validate(&self) -> Result<()> {
        if self.application_id.is_empty() || self.application_id.len() > MAX_APPLICATION_ID_BYTES {
            return Err(SessionError::InvalidConfig(format!("application_id length must be 1..={MAX_APPLICATION_ID_BYTES} bytes")));
        }
        if [
            self.program_digest,
            self.input_schema_hash,
            self.state_codec_hash,
            self.logical_time_policy_hash,
            self.entropy_policy_hash,
            self.court_profile_hash,
            self.handoff_policy_hash,
        ]
        .contains(&[0; 32])
        {
            return Err(SessionError::InvalidConfig("application profile artifact and policy hashes must not be zero".to_owned()));
        }
        self.resources.validate()
    }

    /// Canonical commitment written into genesis and every session block.
    pub fn commitment(&self) -> Hash32 {
        let mut hasher = blake3::Hasher::new();
        hasher.update(APPLICATION_PROFILE_DOMAIN);
        hasher.update(&(self.application_id.len() as u32).to_le_bytes());
        hasher.update(self.application_id.as_bytes());
        hasher.update(&self.program_digest);
        hasher.update(&self.input_schema_hash);
        hasher.update(&self.state_codec_hash);
        hasher.update(&self.logical_time_policy_hash);
        hasher.update(&self.entropy_policy_hash);
        hasher.update(&[self.vm.commitment_byte()]);
        hasher.update(&self.resources.max_frame_inputs.to_le_bytes());
        hasher.update(&self.resources.max_frame_input_bytes.to_le_bytes());
        hasher.update(&self.resources.max_frame_cycles.to_le_bytes());
        hasher.update(&self.resources.max_logical_time_span.to_le_bytes());
        hasher.update(&self.resources.max_inspect_query_bytes.to_le_bytes());
        hasher.update(&self.resources.max_inspect_result_bytes.to_le_bytes());
        hasher.update(&self.court_profile_hash);
        hasher.update(&self.handoff_policy_hash);
        *hasher.finalize().as_bytes()
    }
}

/// Canonical application input attached to one candidate block.
///
/// The payload is retained in the finalised record so a range replay can
/// reproduce application/court execution. Its root is derived by Myelin.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameInput {
    /// First application input position covered by the frame.
    pub start_position: u64,
    /// Exclusive end application input position.
    pub end_position: u64,
    /// First logical-time unit visible to the application.
    pub logical_time_start: u64,
    /// Exclusive logical-time end.
    pub logical_time_end: u64,
    /// Canonical application-owned command/tape bytes.
    pub payload: Vec<u8>,
}

impl FrameInput {
    /// Validate ordering and the genesis-bound resource envelope.
    pub fn validate(&self, profile: &ApplicationProfile) -> Result<()> {
        let input_count = self
            .end_position
            .checked_sub(self.start_position)
            .ok_or_else(|| SessionError::InvalidFrame("input range is reversed".to_owned()))?;
        if input_count > profile.resources.max_frame_inputs {
            return Err(SessionError::InvalidFrame(format!(
                "frame input count {input_count} exceeds {}",
                profile.resources.max_frame_inputs
            )));
        }
        if self.payload.len() as u64 > profile.resources.max_frame_input_bytes {
            return Err(SessionError::InvalidFrame(format!(
                "frame input bytes {} exceed {}",
                self.payload.len(),
                profile.resources.max_frame_input_bytes
            )));
        }
        let logical_span = self
            .logical_time_end
            .checked_sub(self.logical_time_start)
            .ok_or_else(|| SessionError::InvalidFrame("logical-time range is reversed".to_owned()))?;
        if logical_span > profile.resources.max_logical_time_span {
            return Err(SessionError::InvalidFrame(format!(
                "logical-time span {logical_span} exceeds {}",
                profile.resources.max_logical_time_span
            )));
        }
        if input_count == 0 && !self.payload.is_empty() {
            return Err(SessionError::InvalidFrame("an empty input range must have an empty canonical payload".to_owned()));
        }
        if input_count != 0 && self.payload.is_empty() {
            return Err(SessionError::InvalidFrame("a non-empty input range must have a canonical payload".to_owned()));
        }
        Ok(())
    }

    /// Root committed by the execution frame.
    pub fn root(&self) -> Hash32 {
        let mut hasher = blake3::Hasher::new();
        hasher.update(FRAME_INPUT_DOMAIN);
        hasher.update(&self.start_position.to_le_bytes());
        hasher.update(&self.end_position.to_le_bytes());
        hasher.update(&self.logical_time_start.to_le_bytes());
        hasher.update(&self.logical_time_end.to_le_bytes());
        hasher.update(&(self.payload.len() as u64).to_le_bytes());
        hasher.update(&self.payload);
        *hasher.finalize().as_bytes()
    }
}

/// Measured resources committed by an execution frame.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionResources {
    /// Aggregate CKB-VM cycles consumed by the ordered Cell transactions.
    pub cycles: u64,
    /// Aggregate canonical encoded CellTx bytes.
    pub transaction_bytes: u64,
    /// Canonical application input bytes.
    pub input_bytes: u64,
}

/// Auditable application slice bound by one finalised block.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionFrame {
    pub session_id: Hash32,
    pub height: u64,
    pub application_profile_commitment: Hash32,
    pub input_start: u64,
    pub input_end: u64,
    pub input_root: Hash32,
    pub logical_time_start: u64,
    pub logical_time_end: u64,
    pub state_root_before: Hash32,
    pub state_root_after: Hash32,
    pub ordered_cell_tx_commitments: Vec<Hash32>,
    pub resources: ExecutionResources,
}

impl ExecutionFrame {
    /// Validate all derived fields against the retained frame input and
    /// immutable application profile.
    pub fn validate(&self, profile: &ApplicationProfile, input: &FrameInput) -> Result<()> {
        input.validate(profile)?;
        if self.session_id == [0; 32]
            || self.application_profile_commitment != profile.commitment()
            || self.input_root != input.root()
            || self.input_start != input.start_position
            || self.input_end != input.end_position
            || self.logical_time_start != input.logical_time_start
            || self.logical_time_end != input.logical_time_end
            || self.state_root_before == [0; 32]
            || self.state_root_after == [0; 32]
        {
            return Err(SessionError::InvalidFrame(
                "execution frame does not match its session profile, input, or state roots".to_owned(),
            ));
        }
        if self.resources.input_bytes != input.payload.len() as u64 {
            return Err(SessionError::InvalidFrame("execution frame input byte count does not match retained input".to_owned()));
        }
        if self.resources.cycles > profile.resources.max_frame_cycles {
            return Err(SessionError::InvalidFrame(format!(
                "execution cycles {} exceed {}",
                self.resources.cycles, profile.resources.max_frame_cycles
            )));
        }
        Ok(())
    }

    /// Canonical frame commitment included directly in `MyelinBlock`.
    pub fn commitment(&self) -> Hash32 {
        let mut hasher = blake3::Hasher::new();
        hasher.update(EXECUTION_FRAME_DOMAIN);
        hasher.update(&self.session_id);
        hasher.update(&self.height.to_le_bytes());
        hasher.update(&self.application_profile_commitment);
        hasher.update(&self.input_start.to_le_bytes());
        hasher.update(&self.input_end.to_le_bytes());
        hasher.update(&self.input_root);
        hasher.update(&self.logical_time_start.to_le_bytes());
        hasher.update(&self.logical_time_end.to_le_bytes());
        hasher.update(&self.state_root_before);
        hasher.update(&self.state_root_after);
        hasher.update(&(self.ordered_cell_tx_commitments.len() as u32).to_le_bytes());
        for txid in &self.ordered_cell_tx_commitments {
            hasher.update(txid);
        }
        hasher.update(&self.resources.cycles.to_le_bytes());
        hasher.update(&self.resources.transaction_bytes.to_le_bytes());
        hasher.update(&self.resources.input_bytes.to_le_bytes());
        *hasher.finalize().as_bytes()
    }
}

/// Immutable context supplied to an application inspection implementation.
#[derive(Clone, Debug)]
pub struct InspectContext<'a> {
    pub session_id: Hash32,
    pub finalised_height: Option<u64>,
    pub state_root: Hash32,
    pub application_profile: &'a ApplicationProfile,
}

/// Application-owned, read-only state query boundary.
///
/// The port receives immutable snapshot/query bytes and cannot mutate the
/// session chain or emit outbox work through this interface.
pub trait InspectPort: Send + Sync + 'static {
    fn inspect(&self, context: &InspectContext<'_>, snapshot: &[u8], query: &[u8]) -> std::result::Result<Vec<u8>, String>;
}

impl<F> InspectPort for F
where
    F: Fn(&InspectContext<'_>, &[u8], &[u8]) -> std::result::Result<Vec<u8>, String> + Send + Sync + 'static,
{
    fn inspect(&self, context: &InspectContext<'_>, snapshot: &[u8], query: &[u8]) -> std::result::Result<Vec<u8>, String> {
        self(context, snapshot, query)
    }
}

/// Self-authenticating result of a read-only inspection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InspectionReceipt {
    pub session_id: Hash32,
    pub finalised_height: Option<u64>,
    pub state_root: Hash32,
    pub application_profile_commitment: Hash32,
    pub query_hash: Hash32,
    pub result_hash: Hash32,
    pub result: Vec<u8>,
}

pub(crate) fn inspect_query_hash(query: &[u8]) -> Hash32 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(INSPECT_QUERY_DOMAIN);
    hasher.update(&(query.len() as u64).to_le_bytes());
    hasher.update(query);
    *hasher.finalize().as_bytes()
}

pub(crate) fn inspect_result_hash(
    session_id: Hash32,
    finalised_height: Option<u64>,
    state_root: Hash32,
    profile: Hash32,
    query_hash: Hash32,
    result: &[u8],
) -> Hash32 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(INSPECT_RESULT_DOMAIN);
    hasher.update(&session_id);
    hasher.update(&finalised_height.unwrap_or(u64::MAX).to_le_bytes());
    hasher.update(&state_root);
    hasher.update(&profile);
    hasher.update(&query_hash);
    hasher.update(&(result.len() as u64).to_le_bytes());
    hasher.update(result);
    *hasher.finalize().as_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> ApplicationProfile {
        ApplicationProfile {
            application_id: "test-application".to_owned(),
            program_digest: [1; 32],
            input_schema_hash: [2; 32],
            state_codec_hash: [3; 32],
            logical_time_policy_hash: [4; 32],
            entropy_policy_hash: [5; 32],
            vm: ApplicationVmProfile { spawn_ipc_required: false },
            resources: ApplicationResourceEnvelope {
                max_frame_inputs: 100,
                max_frame_input_bytes: 1_000,
                max_frame_cycles: 10_000,
                max_logical_time_span: 1_000,
                max_inspect_query_bytes: 100,
                max_inspect_result_bytes: 200,
            },
            court_profile_hash: [6; 32],
            handoff_policy_hash: [7; 32],
        }
    }

    fn frame(profile: &ApplicationProfile, input: &FrameInput) -> ExecutionFrame {
        ExecutionFrame {
            session_id: [7; 32],
            height: 8,
            application_profile_commitment: profile.commitment(),
            input_start: input.start_position,
            input_end: input.end_position,
            input_root: input.root(),
            logical_time_start: input.logical_time_start,
            logical_time_end: input.logical_time_end,
            state_root_before: [9; 32],
            state_root_after: [10; 32],
            ordered_cell_tx_commitments: vec![[11; 32]],
            resources: ExecutionResources { cycles: 12, transaction_bytes: 13, input_bytes: input.payload.len() as u64 },
        }
    }

    #[test]
    fn application_profile_commitment_changes_for_every_committed_field() {
        let baseline = profile();
        baseline.validate().unwrap();
        let commitment = baseline.commitment();
        let mut mutations = Vec::new();
        let mut changed = baseline.clone();
        changed.application_id.push('x');
        mutations.push(changed);
        let mut changed = baseline.clone();
        changed.program_digest = [21; 32];
        mutations.push(changed);
        let mut changed = baseline.clone();
        changed.input_schema_hash = [22; 32];
        mutations.push(changed);
        let mut changed = baseline.clone();
        changed.state_codec_hash = [23; 32];
        mutations.push(changed);
        let mut changed = baseline.clone();
        changed.logical_time_policy_hash = [24; 32];
        mutations.push(changed);
        let mut changed = baseline.clone();
        changed.entropy_policy_hash = [25; 32];
        mutations.push(changed);
        let mut changed = baseline.clone();
        changed.vm.spawn_ipc_required = true;
        mutations.push(changed);
        let mut changed = baseline.clone();
        changed.resources.max_frame_inputs += 1;
        mutations.push(changed);
        let mut changed = baseline.clone();
        changed.resources.max_frame_input_bytes += 1;
        mutations.push(changed);
        let mut changed = baseline.clone();
        changed.resources.max_frame_cycles += 1;
        mutations.push(changed);
        let mut changed = baseline.clone();
        changed.resources.max_logical_time_span += 1;
        mutations.push(changed);
        let mut changed = baseline.clone();
        changed.resources.max_inspect_query_bytes += 1;
        mutations.push(changed);
        let mut changed = baseline.clone();
        changed.resources.max_inspect_result_bytes += 1;
        mutations.push(changed);
        let mut changed = baseline;
        changed.court_profile_hash = [26; 32];
        mutations.push(changed);
        let mut changed = profile();
        changed.handoff_policy_hash = [27; 32];
        mutations.push(changed);
        for changed in mutations {
            assert_ne!(commitment, changed.commitment());
        }
    }

    #[test]
    fn frame_binds_input_roots_ranges_state_txids_and_resources() {
        let profile = profile();
        let input = FrameInput {
            start_position: 1,
            end_position: 2,
            logical_time_start: 10,
            logical_time_end: 20,
            payload: b"command".to_vec(),
        };
        let baseline = frame(&profile, &input);
        baseline.validate(&profile, &input).unwrap();
        let commitment = baseline.commitment();
        let mut changed = baseline.clone();
        changed.input_root = [31; 32];
        assert_ne!(commitment, changed.commitment());
        assert!(changed.validate(&profile, &input).is_err());
        let mut changed = baseline.clone();
        changed.logical_time_end += 1;
        assert_ne!(commitment, changed.commitment());
        let mut changed = baseline.clone();
        changed.state_root_after = [32; 32];
        assert_ne!(commitment, changed.commitment());
        let mut changed = baseline.clone();
        changed.ordered_cell_tx_commitments.push([33; 32]);
        assert_ne!(commitment, changed.commitment());
        let mut changed = baseline;
        changed.resources.cycles += 1;
        assert_ne!(commitment, changed.commitment());
    }

    #[test]
    fn frame_input_fails_closed_on_ranges_and_bounds() {
        let profile = profile();
        let reversed = FrameInput { start_position: 2, end_position: 1, logical_time_start: 0, logical_time_end: 1, payload: vec![1] };
        assert!(reversed.validate(&profile).is_err());
        let inconsistent =
            FrameInput { start_position: 0, end_position: 0, logical_time_start: 0, logical_time_end: 0, payload: vec![1] };
        assert!(inconsistent.validate(&profile).is_err());
        let oversized =
            FrameInput { start_position: 0, end_position: 101, logical_time_start: 0, logical_time_end: 1, payload: vec![1] };
        assert!(oversized.validate(&profile).is_err());
    }
}
