//! Optional host input adapter. Rules and authorization stay application-owned.
//! New profiles explicitly opt into this frame/snapshot/time contract. This is
//! not a CKB court/settlement verifier.
use super::*;
use myelin_session::{
    bounded_codec, ApplicationProfile, ApplicationResourceEnvelope, ApplicationVmProfile, BlockRequest, ExecutionOutcome, FrameInput,
    PendingOutboxMessage, ProtocolEffectsPolicy, SessionHandoff,
};
use std::sync::Arc;
#[path = "application_recovery.rs"]
mod recovery;
pub use recovery::recover_local_commands;
#[path = "application_snapshot.rs"]
pub mod snapshot;
use snapshot::Snapshot;

const FRAME_MAGIC: &[u8; 4] = b"MCAF";
const RESULT_TOPIC: &str = "myelin-command-result-v1";
const MAX_COMMANDS: usize = 256;
type AppResult<T> = std::result::Result<T, String>;

fn digest(domain: &[u8], bytes: &[u8]) -> Hash32 {
    let mut h = blake3::Hasher::new();
    h.update(domain);
    h.update(bytes);
    *h.finalize().as_bytes()
}
fn state_codec(ticks: bool) -> Hash32 {
    digest(b"myelin:application-codec:v1", if ticks { b"fixed-tick:MCAS1" } else { b"commands:MCAS1" })
}
/// A tick packet is one durable input and one simulation tick. Commit time,
/// block height and browser presentation time never enter program callbacks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum ApplicationClock {
    CommandSteps,
    FixedTicks { tick_duration_us: u64, numeric_policy: Hash32, admission_policy: Hash32, replay_policy: Hash32 },
}
impl ApplicationClock {
    pub fn is_fixed_tick(&self) -> bool {
        matches!(self, Self::FixedTicks { .. })
    }
    fn commitment(&self) -> AppResult<Hash32> {
        if let Self::FixedTicks { tick_duration_us, numeric_policy, admission_policy, replay_policy } = self {
            if *tick_duration_us == 0 || [*numeric_policy, *admission_policy, *replay_policy].contains(&[0; 32]) {
                return Err("fixed ticks require a nonzero duration and numeric/admission/replay policies".into());
            }
        }
        Ok(digest(b"myelin:application-clock:v1", &postcard::to_allocvec(self).map_err(|e| e.to_string())?))
    }
}
fn no_effects() -> Hash32 {
    digest(b"myelin:command-application-effects:v1", b"disabled")
}

/// All fields affect the profile. The caller supplies an actual artifact digest
/// and its explicit authorization/rules identities; selected-source hashing is
/// not a reproducible-binary claim.
pub struct CommandProfile {
    pub application_id: String,
    pub program_digest: Hash32,
    pub command_schema: Hash32,
    pub authorization_policy: Hash32,
    pub entropy_policy: Hash32,
    pub clock: ApplicationClock,
    pub resources: ApplicationResourceEnvelope,
}
impl CommandProfile {
    pub fn build(self) -> AppResult<ApplicationProfile> {
        if [self.program_digest, self.command_schema, self.authorization_policy, self.entropy_policy].contains(&[0; 32]) {
            return Err("command profile requires exact nonzero program/schema/auth/entropy identities".into());
        }
        let mut schema = Vec::from(self.command_schema);
        schema.extend_from_slice(&self.authorization_policy);
        let profile = ApplicationProfile {
            application_id: self.application_id,
            command_identity_required: true,
            program_digest: self.program_digest,
            input_schema_hash: digest(b"myelin:command-application-input:v1", &schema),
            state_codec_hash: state_codec(self.clock.is_fixed_tick()),
            logical_time_policy_hash: self.clock.commitment()?,
            entropy_policy_hash: self.entropy_policy,
            vm: ApplicationVmProfile { spawn_ipc_required: false },
            resources: self.resources,
            court_profile_hash: digest(b"myelin:command-application-evidence:v1", b"host-replay-only"),
            handoff_policy_hash: no_effects(),
        };
        profile.validate().map_err(|e| e.to_string())?;
        Ok(profile)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandCapabilities {
    pub version: u16,
    pub profile: Hash32,
    pub original_commands: bool,
    pub authenticated_system_events: bool,
    pub tick_tapes: bool,
    pub handoffs: bool,
    pub successors: bool,
    pub settlement: bool,
    pub ckb_vm_execution: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct CommandContext {
    pub input_position: u64,
    pub logical_time: u64,
}
/// Implementations must be deterministic and side-effect free outside their
/// cloned state. System events still pass CommandAuthenticator. CommandSteps
/// commits business rejections after discarding speculative mutations. FixedTicks
/// rejects the entire candidate on rejection, so time cannot advance without physics.
pub trait CommandProgram: Clone + Send + Sync + 'static {
    fn clock(&self) -> ApplicationClock;
    fn snapshot(&self) -> AppResult<Vec<u8>>;
    fn apply(&mut self, context: CommandContext, command: &OriginalCommand) -> AppResult<BusinessOutcome>;
    fn inspect(&self, query: &[u8]) -> AppResult<Vec<u8>>;
}

fn encode<T: Serialize>(value: &T, limit: u64) -> AppResult<Vec<u8>> {
    bounded_codec::size_with_limit(value, "command_application_bytes", limit).map_err(|e| e.to_string())?;
    postcard::to_allocvec(value).map_err(|e| e.to_string())
}
fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8], limit: u64) -> AppResult<T> {
    if bytes.len() as u64 > limit || bytes.len() > bounded_codec::MAX_RECORD_BYTES {
        return Err("application byte limit".into());
    }
    let (value, rest) = postcard::take_from_bytes(bytes).map_err(|e| e.to_string())?;
    if !rest.is_empty() {
        return Err("trailing application bytes".into());
    }
    Ok(value)
}

#[derive(Clone)]
pub struct CommandExecutor<P: CommandProgram> {
    config: SessionConfig,
    scope: CommandScope,
    program: P,
    auth: Arc<dyn CommandAuthenticator>,
    cursor: u64,
    logical_time: u64,
    snapshot: Vec<u8>,
    root: Hash32,
}
impl<P: CommandProgram> CommandExecutor<P> {
    pub fn initial_root(profile: &ApplicationProfile, program: &P, cursor: u64, logical_time: u64, limit: u64) -> AppResult<Hash32> {
        validate_program(profile, program)?;
        let state = program.snapshot()?;
        let snapshot = Snapshot { profile: profile.commitment(), cursor, logical_time, state: &state };
        Ok(snapshot::state_root(&snapshot.encode(limit)?))
    }
    pub fn new(config: SessionConfig, program: P, auth: Arc<dyn CommandAuthenticator>) -> AppResult<Self> {
        let value = Self::at(config.clone(), program, auth, config.initial_input_position, config.initial_logical_time)?;
        if value.root != config.initial_state_root {
            return Err("command application genesis state mismatch".into());
        }
        Ok(value)
    }
    fn at(config: SessionConfig, program: P, auth: Arc<dyn CommandAuthenticator>, cursor: u64, logical_time: u64) -> AppResult<Self> {
        validate_config(&config)?;
        validate_program(&config.application_profile, &program)?;
        let state = program.snapshot()?;
        let snapshot = Snapshot { profile: config.application_profile.commitment(), cursor, logical_time, state: &state }
            .encode(config.commit_limits.max_snapshot_bytes)?;
        let root = snapshot::state_root(&snapshot);
        Ok(Self { scope: CommandScope::from_config(&config), config, program, auth, cursor, logical_time, snapshot, root })
    }
    pub fn restore(
        config: SessionConfig,
        bytes: &[u8],
        auth: Arc<dyn CommandAuthenticator>,
        restore: impl FnOnce(&[u8]) -> AppResult<P>,
    ) -> AppResult<Self> {
        validate_config(&config)?;
        let snapshot = snapshot::decode(bytes, config.application_profile.commitment(), config.commit_limits.max_snapshot_bytes)?;
        let program = restore(snapshot.state)?;
        let value = Self::at(config, program, auth, snapshot.cursor, snapshot.logical_time)?;
        if value.snapshot != bytes {
            return Err("noncanonical command snapshot or changed program interpretation".into());
        }
        Ok(value)
    }
    pub fn capabilities(&self) -> CommandCapabilities {
        CommandCapabilities {
            version: 1,
            profile: self.scope.application_profile,
            original_commands: true,
            authenticated_system_events: !self.program.clock().is_fixed_tick(),
            tick_tapes: self.program.clock().is_fixed_tick(),
            handoffs: false,
            successors: false,
            settlement: false,
            ckb_vm_execution: false,
        }
    }
    pub fn inspect(&self, query: &[u8]) -> AppResult<Vec<u8>> {
        let resources = &self.config.application_profile.resources;
        if query.len() as u64 > resources.max_inspect_query_bytes {
            return Err("inspection query limit".into());
        }
        let result = self.program.inspect(query)?;
        if result.len() as u64 > resources.max_inspect_result_bytes {
            return Err("inspection result limit".into());
        }
        Ok(result)
    }
}
fn validate_program(profile: &ApplicationProfile, program: &impl CommandProgram) -> AppResult<()> {
    let clock = program.clock();
    if profile.state_codec_hash != state_codec(clock.is_fixed_tick()) || profile.logical_time_policy_hash != clock.commitment()? {
        return Err("program clock/numeric/admission/replay policy differs from its profile".into());
    }
    Ok(())
}
fn validate_config(config: &SessionConfig) -> AppResult<bool> {
    config.validate().map_err(|e| e.to_string())?;
    let profile = &config.application_profile;
    if !profile.command_identity_required
        || ![state_codec(false), state_codec(true)].contains(&profile.state_codec_hash)
        || (profile.state_codec_hash == state_codec(false)
            && profile.logical_time_policy_hash != ApplicationClock::CommandSteps.commitment()?)
        || profile.handoff_policy_hash != no_effects()
        || profile.vm.spawn_ipc_required
        || profile.court_profile_hash != digest(b"myelin:command-application-evidence:v1", b"host-replay-only")
        || config.protocol_effects != ProtocolEffectsPolicy::Disabled
    {
        return Err("profile does not opt into the host command adapter and disabled protocol effects".into());
    }
    Ok(profile.state_codec_hash == state_codec(true))
}

#[derive(Serialize, Deserialize)]
struct Commands(#[serde(deserialize_with = "bounded_codec::bounded_vec")] Vec<OriginalCommand>);
fn validate_original(scope: &CommandScope, command: &OriginalCommand, ticks: bool) -> AppResult<()> {
    if &command.scope != scope
        || command.identity.id == [0; 32]
        || command.identity.payload_hash == [0; 32]
        || command.canonical_command.is_empty()
        || (command.kind == InputKind::InputTape) != ticks
    {
        return Err("command scope, identity or input kind differs from the profile".into());
    }
    Ok(())
}
pub fn command_frame(config: &SessionConfig, head: &SessionHead, commands: Vec<OriginalCommand>) -> AppResult<FrameInput> {
    let ticks = validate_config(config)?;
    if head.session_id != config.session_id || head.sealed_by_successor.is_some() || commands.len() > MAX_COMMANDS {
        return Err("foreign/sealed head or too many commands".into());
    }
    let scope = CommandScope::from_config(config);
    for command in &commands {
        validate_original(&scope, command, ticks)?;
    }
    let count = commands.len() as u64;
    let identities = commands.iter().map(|c| c.identity.clone()).collect();
    let payload = if commands.is_empty() {
        vec![]
    } else {
        let mut bytes = FRAME_MAGIC.to_vec();
        bytes.push(1);
        bytes.extend(encode(&Commands(commands), config.application_profile.resources.max_frame_input_bytes.saturating_sub(5))?);
        bytes
    };
    let frame = FrameInput {
        commands: identities,
        start_position: head.input_position,
        end_position: head.input_position.checked_add(count).ok_or("input cursor overflow")?,
        logical_time_start: head.logical_time,
        logical_time_end: head.logical_time.checked_add(count).ok_or("logical time overflow")?,
        payload,
    };
    frame.validate(&config.application_profile).map_err(|e| e.to_string())?;
    Ok(frame)
}
pub fn request_from_reservation(
    config: &SessionConfig,
    reservation: &Reservation,
    timestamp_ms: u64,
    local_now_ms: u64,
) -> AppResult<BlockRequest> {
    let commands: Vec<OriginalCommand> =
        reservation.work.iter().map(|work| decode(work, bounded_codec::MAX_RECORD_BYTES as u64)).collect::<AppResult<_>>()?;
    if commands.iter().map(|c| c.identity.id).collect::<Vec<_>>() != reservation.command_ids {
        return Err("reserved work differs from its original command identities".into());
    }
    let frame_input = command_frame(config, &reservation.base_head().map_err(|e| e.to_string())?, commands)?;
    Ok(BlockRequest {
        transactions: vec![],
        frame_input,
        scheduler_commitment: digest(b"myelin:command-application-order:v1", b"original-input-order"),
        data_commitments: vec![],
        timestamp_ms,
        local_now_ms,
        successor: None,
        handoffs: vec![],
        consume_handoffs: vec![],
        handoff_actions: vec![],
    })
}
impl CommandJournal {
    pub fn reserve_commands(
        &self,
        head: &SessionHead,
        max_commands: usize,
        max_bytes: usize,
        auth: &dyn CommandAuthenticator,
    ) -> Result<Option<Reservation>> {
        self.reserve(head, max_commands.min(MAX_COMMANDS), max_bytes, auth, |original, _| {
            match encode(original, bounded_codec::MAX_RECORD_BYTES as u64) {
                Ok(bytes) => BuildDecision::Ready(bytes),
                Err(error) => BuildDecision::Rejected(error),
            }
        })
    }
}
impl<P: CommandProgram> TransitionExecutor for CommandExecutor<P> {
    fn state_root(&self) -> Hash32 {
        self.root
    }
    fn snapshot(&self) -> AppResult<Vec<u8>> {
        Ok(self.snapshot.clone())
    }
    fn execute_block(
        &mut self,
        _height: u64,
        frame: &FrameInput,
        handoffs: &[SessionHandoff],
        transactions: &[Vec<u8>],
    ) -> AppResult<ExecutionOutcome> {
        frame.validate(&self.config.application_profile).map_err(|e| e.to_string())?;
        if !handoffs.is_empty()
            || !transactions.is_empty()
            || frame.start_position != self.cursor
            || frame.logical_time_start != self.logical_time
        {
            return Err("command executor requires its exact cursor and no CellTx/handoff intake".into());
        }
        let commands = if frame.payload.is_empty() {
            vec![]
        } else {
            if !frame.payload.starts_with(FRAME_MAGIC) || frame.payload.get(4) != Some(&1) {
                return Err("unknown command frame encoding".into());
            }
            decode::<Commands>(&frame.payload[5..], self.config.application_profile.resources.max_frame_input_bytes)?.0
        };
        if commands.len() > MAX_COMMANDS
            || commands.iter().map(|c| c.identity.clone()).collect::<Vec<_>>() != frame.commands
            || frame.logical_time_end.checked_sub(frame.logical_time_start) != Some(commands.len() as u64)
        {
            return Err("command identities or logical-time span mismatch".into());
        }
        let mut next = self.clone();
        let mut outbox = vec![];
        for command in commands {
            validate_original(&next.scope, &command, next.program.clock().is_fixed_tick())?;
            next.auth.verify(&command)?;
            let mut program = next.program.clone();
            let result = program.apply(CommandContext { input_position: next.cursor, logical_time: next.logical_time }, &command)?;
            if next.program.clock().is_fixed_tick() && matches!(result, BusinessOutcome::Rejected(_)) {
                return Err("a rejected tick cannot advance the simulation clock".into());
            }
            if matches!(result, BusinessOutcome::Succeeded(_)) {
                next.program = program;
            }
            let payload = encode(&(command.identity, result), self.config.commit_limits.max_outbox_bytes)?;
            outbox.push(PendingOutboxMessage { topic: RESULT_TOPIC.into(), payload });
            next.cursor = next.cursor.checked_add(1).ok_or("input cursor overflow")?;
            next.logical_time = next.logical_time.checked_add(1).ok_or("logical time overflow")?;
        }
        if next.cursor != frame.end_position || next.logical_time != frame.logical_time_end {
            return Err("command frame end mismatch".into());
        }
        next = Self::at(next.config, next.program, next.auth, next.cursor, next.logical_time)?;
        let outcome = ExecutionOutcome { ordered_cell_tx_commitments: vec![], data_commitments: vec![next.root], outbox, cycles: 0 };
        *self = next;
        Ok(outcome)
    }
}

/// Exact result contract shared by live, restarted and remotely reconciled hosts.
pub fn business_result(record: &FinalisedBlockRecord, receipt: &CommandReceipt) -> AppResult<Option<BusinessOutcome>> {
    let index = receipt.input_position.checked_sub(record.frame_input.start_position).ok_or("receipt before frame")?;
    let index = usize::try_from(index).map_err(|_| "receipt position overflow")?;
    if CommandReceipt::for_record(record, index).map_err(|e| e.to_string())? != *receipt {
        return Err("receipt does not name this frame".into());
    }
    let mut found = None;
    for event in record.outbox.iter().filter(|e| e.topic == RESULT_TOPIC) {
        let (identity, result): (CommandIdentity, BusinessOutcome) = decode(&event.payload, bounded_codec::MAX_RECORD_BYTES as u64)?;
        if identity == receipt.command && found.replace(result).is_some() {
            return Err("duplicate command business result".into());
        }
    }
    Ok(found)
}

/// A trusted, application-supplied restore function; never dynamically loads an
/// unknown native program. Queries verify the restored root and exact profile.
pub fn command_inspector<P: CommandProgram>(
    config: SessionConfig,
    auth: Arc<dyn CommandAuthenticator>,
    restore: impl Fn(&[u8]) -> AppResult<P> + Send + Sync + 'static,
) -> impl myelin_session::InspectPort {
    move |context: &myelin_session::InspectContext<'_>, snapshot: &[u8], query: &[u8]| {
        if context.session_id != config.session_id || context.application_profile != &config.application_profile {
            return Err("inspection scope/profile mismatch".into());
        }
        let executor = CommandExecutor::restore(config.clone(), snapshot, auth.clone(), &restore)?;
        if executor.state_root() != context.state_root {
            return Err("inspection snapshot root mismatch".into());
        }
        executor.inspect(query)
    }
}
