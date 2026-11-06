// SPDX-License-Identifier: MIT
//! Original commands survive transaction reconstruction, restart and lost replies.
//!
//! This database is independent of SessionStore. Persist `begin_commit` before
//! calling a committer. A crash after that point requires receipt reconciliation;
//! Missing never releases the reservation. No cross-host fencing is implied.

mod fencing;
pub use fencing::{FencedRecovery, FencedSubmission};
mod remote;
pub use remote::{RemoteJournalError, RemoteSubmissionReport};
pub mod application;

use myelin_session::{
    CommandIdentity, CommandReceipt, CommandStatus, FinalisedBlockRecord, Hash32, SessionChain, SessionConfig, SessionHead,
    SessionStore, TransitionExecutor,
};
use rocksdb::{Options, WriteOptions, DB};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};

const KEY: &[u8] = b"myelin-command-journal";
const MAGIC: &[u8] = b"MJNL\x02";
const MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_ENTRIES: usize = 16_384;

#[derive(Debug, thiserror::Error)]
pub enum JournalError {
    #[error("invalid command journal data: {0}")]
    Invalid(String),
    #[error("command identity is already bound to different original work")]
    IdentityConflict,
    #[error("journal transition conflicts with its durable state: {0}")]
    Conflict(String),
    #[error("journal storage: {0}")]
    Storage(String),
    #[error("command authentication: {0}")]
    Authentication(String),
    #[error("session reconciliation: {0}")]
    Reconciliation(String),
}
pub type Result<T> = std::result::Result<T, JournalError>;

/// Tape and system-event authentication is application-defined. This enum does
/// not impose a wallet signature on each simulation sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputKind {
    UserCommand,
    InputTape,
    SystemEvent,
}

/// This scope is part of authentication, not an unsigned routing hint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandScope {
    pub session_id: Hash32,
    pub genesis_commitment: Hash32,
    pub application_profile: Hash32,
}
impl CommandScope {
    pub fn from_config(config: &SessionConfig) -> Self {
        Self {
            session_id: config.session_id,
            genesis_commitment: config.commitment(),
            application_profile: config.application_profile.commitment(),
        }
    }
}

/// Original application bytes and authentication material. A rebuilt CellTx is
/// deliberately stored elsewhere and never changes this economic intent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OriginalCommand {
    pub scope: CommandScope,
    pub identity: CommandIdentity,
    pub kind: InputKind,
    #[serde(deserialize_with = "myelin_session::bounded_codec::bounded_bytes")]
    pub canonical_command: Vec<u8>,
    #[serde(deserialize_with = "myelin_session::bounded_codec::bounded_bytes")]
    pub authentication_context: Vec<u8>,
    #[serde(deserialize_with = "myelin_session::bounded_codec::bounded_bytes")]
    pub authorization: Vec<u8>,
}
impl OriginalCommand {
    /// The caller's verifier must bind this digest (or an equally scoped
    /// application digest) to authorization. It must also authenticate the
    /// supplied payload hash against canonical_command, not trust metadata.
    pub fn authorization_digest(&self) -> Hash32 {
        let mut h = blake3::Hasher::new();
        h.update(b"myelin:command-journal-authorization:v1");
        h.update(&self.scope.session_id);
        h.update(&self.scope.genesis_commitment);
        h.update(&self.scope.application_profile);
        h.update(&self.identity.id);
        h.update(&self.identity.payload_hash);
        h.update(&[match self.kind {
            InputKind::UserCommand => 0,
            InputKind::InputTape => 1,
            InputKind::SystemEvent => 2,
        }]);
        for bytes in [&self.canonical_command, &self.authentication_context] {
            h.update(&(bytes.len() as u64).to_le_bytes());
            h.update(bytes);
        }
        *h.finalize().as_bytes()
    }

    fn same_intent(&self, other: &Self) -> bool {
        self.scope == other.scope
            && self.identity == other.identity
            && self.kind == other.kind
            && self.canonical_command == other.canonical_command
            && self.authentication_context == other.authentication_context
    }
}

/// Rechecked on enqueue and reconstruction. The verifier implements the
/// application's identity, signature, delegation and input-kind policy.
pub trait CommandAuthenticator: Send + Sync {
    fn verify(&self, command: &OriginalCommand) -> std::result::Result<(), String>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalLimits {
    pub max_entries: usize,
    pub max_encoded_bytes: usize,
    pub max_command_bytes: usize,
    pub max_authorization_bytes: usize,
    pub max_result_bytes: usize,
}
impl Default for JournalLimits {
    fn default() -> Self {
        Self {
            max_entries: 4096,
            max_encoded_bytes: 16 * 1024 * 1024,
            max_command_bytes: 1024 * 1024,
            max_authorization_bytes: 64 * 1024,
            max_result_bytes: 1024 * 1024,
        }
    }
}
impl JournalLimits {
    fn validate(self) -> Result<()> {
        if self.max_entries == 0
            || self.max_entries > MAX_ENTRIES
            || self.max_encoded_bytes == 0
            || self.max_encoded_bytes > MAX_BYTES
            || [self.max_command_bytes, self.max_authorization_bytes, self.max_result_bytes]
                .iter()
                .any(|n| *n == 0 || *n > self.max_encoded_bytes)
        {
            return Err(JournalError::Invalid("journal limits exceed bounded format".into()));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReservationPhase {
    /// No commit call may start until begin_commit is durable.
    Held,
    /// A commit may be in progress or finished. Never release on Missing.
    Submitting,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BusinessOutcome {
    Succeeded(Vec<u8>),
    Rejected(Vec<u8>),
}
impl BusinessOutcome {
    fn len(&self) -> usize {
        match self {
            Self::Succeeded(bytes) | Self::Rejected(bytes) => bytes.len(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryStatus {
    Queued,
    Stale(String),
    Rejected(String),
    Reserved {
        token: u64,
        phase: ReservationPhase,
    },
    /// Inclusion alone leaves outcome None. Only application interpretation of
    /// the exact committed block can establish a business result.
    Committed {
        receipt: CommandReceipt,
        outcome: Option<BusinessOutcome>,
        acknowledged: bool,
    },
    IdentityConflict(CommandReceipt),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalEntry {
    pub sequence: u64,
    pub original: OriginalCommand,
    pub authorization_digest: Hash32,
    pub status: EntryStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reservation {
    pub token: u64,
    /// Canonical SessionHead record, including state root, cursor and seal.
    construction_head: Vec<u8>,
    pub command_ids: Vec<Hash32>,
    /// Application-built work, in the exact command order, bound to the head.
    pub work: Vec<Vec<u8>>,
    pub phase: ReservationPhase,
    /// Present only when the exact signed authority request was persisted
    /// before it could be sent. Legacy/unfenced submissions cannot fabricate it.
    pub fenced_submission: Option<FencedSubmission>,
}
impl Reservation {
    pub fn base_head(&self) -> Result<SessionHead> {
        SessionHead::decode(&self.construction_head).map_err(|e| JournalError::Invalid(e.to_string()))
    }
    pub fn validate_base(&self, head: &SessionHead) -> Result<()> {
        if self.base_head()? != *head {
            return Err(JournalError::Conflict("construction head is stale; rebuild the same intent".into()));
        }
        Ok(())
    }
}

/// Builder decisions preserve an already valid prefix. Stale/Rejected describe
/// the first failing command; a later command never silently overtakes it.
pub enum BuildDecision {
    Ready(Vec<u8>),
    Stale(String),
    Rejected(String),
}

/// The committer's explicit result, separate from journal inclusion/business
/// interpretation. Success still requires receipt reconciliation before ACK.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Submission {
    Committed(myelin_session_producer::ProductionReceipt),
    Stale,
    Rejected(String),
    Unknown(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    scope: CommandScope,
    limits: JournalLimits,
    next_token: u64,
    entries: Vec<JournalEntry>,
    reservation: Option<Reservation>,
    reconciliations: Vec<FencedRecovery>,
    fencing_policy: Option<myelin_session_authority::FencingPolicy>,
}

/// A bounded synchronous-WAL journal. RocksDB's exclusive directory lock and
/// this mutex coordinate one local owner; neither fences writers on other hosts.
/// Original entries, including acknowledged results, are retained for retries.
pub struct CommandJournal {
    db: DB,
    state: Mutex<State>,
    recovery_required: AtomicBool,
    #[cfg(test)]
    fail_after_persist: AtomicBool,
}
impl CommandJournal {
    pub fn open(path: impl AsRef<Path>, config: &SessionConfig, limits: JournalLimits) -> Result<Self> {
        config.validate().map_err(|e| JournalError::Invalid(e.to_string()))?;
        limits.validate()?;
        if !config.application_profile.command_identity_required {
            return Err(JournalError::Invalid("journal requires an explicit command-identity application profile".into()));
        }
        let mut options = Options::default();
        options.create_if_missing(true);
        let db = DB::open(&options, path).map_err(storage)?;
        let scope = CommandScope::from_config(config);
        let state = match db.get(KEY).map_err(storage)? {
            Some(bytes) => {
                let state = decode(&bytes)?;
                if state.scope != scope || state.limits != limits {
                    return Err(JournalError::Invalid("journal genesis/profile/limits mismatch".into()));
                }
                state
            }
            None => {
                if db.iterator(rocksdb::IteratorMode::Start).next().transpose().map_err(storage)?.is_some() {
                    return Err(JournalError::Invalid("nonempty database has no command-journal record".into()));
                }
                let state = State {
                    scope,
                    limits,
                    next_token: 1,
                    entries: vec![],
                    reservation: None,
                    reconciliations: vec![],
                    fencing_policy: None,
                };
                validate(&state)?;
                persist(&db, &state)?;
                state
            }
        };
        Ok(Self {
            db,
            state: Mutex::new(state),
            recovery_required: AtomicBool::new(false),
            #[cfg(test)]
            fail_after_persist: AtomicBool::new(false),
        })
    }

    fn ensure_healthy(&self) -> Result<()> {
        if self.recovery_required.load(Ordering::Acquire) {
            return Err(JournalError::Storage(
                "journal write outcome is uncertain; close and reopen before further operations".into(),
            ));
        }
        Ok(())
    }

    fn update<T>(&self, f: impl FnOnce(&mut State) -> Result<T>) -> Result<T> {
        let mut current = self.state.lock().map_err(|_| JournalError::Storage("journal mutex poisoned".into()))?;
        self.ensure_healthy()?;
        let mut next = current.clone();
        let result = f(&mut next)?;
        // Successful retries/ACKs need no write when the durable state is
        // unchanged. Authentication still runs inside the operation above.
        if next == *current {
            return Ok(result);
        }
        validate_update(&next, Some(&current))?;
        if let Err(error) = persist(&self.db, &next) {
            if matches!(error, JournalError::Storage(_)) {
                self.recovery_required.store(true, Ordering::Release);
            }
            return Err(error);
        }
        #[cfg(test)]
        if self.fail_after_persist.swap(false, Ordering::AcqRel) {
            self.recovery_required.store(true, Ordering::Release);
            return Err(JournalError::Storage("injected ambiguous error after actual synchronous write".into()));
        }
        *current = next;
        Ok(result)
    }

    pub fn enqueue(&self, command: OriginalCommand, auth: &dyn CommandAuthenticator) -> Result<JournalEntry> {
        self.update(|state| {
            validate_command(state, &command)?;
            auth.verify(&command).map_err(JournalError::Authentication)?;
            if let Some(existing) = state.entries.iter().find(|e| e.original.identity.id == command.identity.id) {
                if !existing.original.same_intent(&command) {
                    return Err(JournalError::IdentityConflict);
                }
                // Keep the originally admitted authorization, even if a valid
                // retry carries another signature of the identical intent.
                return Ok(existing.clone());
            }
            if state.entries.len() >= state.limits.max_entries {
                return Err(JournalError::Invalid("journal entry capacity reached; retention must be explicit".into()));
            }
            let entry = JournalEntry {
                sequence: state.entries.len() as u64,
                authorization_digest: command.authorization_digest(),
                original: command,
                status: EntryStatus::Queued,
            };
            state.entries.push(entry.clone());
            Ok(entry)
        })
    }

    pub fn entry(&self, id: Hash32) -> Result<Option<JournalEntry>> {
        let state = self.state.lock().map_err(|_| JournalError::Storage("journal mutex poisoned".into()))?;
        self.ensure_healthy()?;
        Ok(state.entries.iter().find(|e| e.original.identity.id == id).cloned())
    }

    pub fn entries(&self, start_sequence: u64, limit: usize) -> Result<Vec<JournalEntry>> {
        if limit > 256 {
            return Err(JournalError::Invalid("journal page limit exceeds 256".into()));
        }
        let state = self.state.lock().map_err(|_| JournalError::Storage("journal mutex poisoned".into()))?;
        self.ensure_healthy()?;
        Ok(state.entries.iter().filter(|e| e.sequence >= start_sequence).take(limit).cloned().collect())
    }

    pub fn active_reservation(&self) -> Result<Option<Reservation>> {
        let state = self.state.lock().map_err(|_| JournalError::Storage("journal mutex poisoned".into()))?;
        self.ensure_healthy()?;
        Ok(state.reservation.clone())
    }

    pub fn reserve(
        &self,
        head: &SessionHead,
        max_commands: usize,
        max_work_bytes: usize,
        auth: &dyn CommandAuthenticator,
        mut build: impl FnMut(&OriginalCommand, &SessionHead) -> BuildDecision,
    ) -> Result<Option<Reservation>> {
        self.update(|state| {
            if head.session_id != state.scope.session_id
                || head.sealed_by_successor.is_some()
                || max_commands == 0
                || max_work_bytes == 0
            {
                return Err(JournalError::Invalid("wrong/sealed construction head or empty reservation limit".into()));
            }
            if state.reservation.is_some() {
                return Err(JournalError::Conflict("reconcile or release the existing reservation first".into()));
            }
            let mut reservation = Reservation {
                token: state.next_token,
                construction_head: head.encode().map_err(|e| JournalError::Invalid(e.to_string()))?,
                command_ids: vec![],
                work: vec![],
                phase: ReservationPhase::Held,
                fenced_submission: None,
            };
            let mut bytes = 0usize;
            for entry in &mut state.entries {
                match entry.status {
                    EntryStatus::Queued => {}
                    EntryStatus::Stale(_) => break,
                    _ => continue,
                }
                let decision = match auth.verify(&entry.original) {
                    Ok(()) => build(&entry.original, head),
                    Err(error) => BuildDecision::Rejected(error),
                };
                let work = match decision {
                    BuildDecision::Ready(work) => work,
                    BuildDecision::Stale(reason) => {
                        entry.status = EntryStatus::Stale(reason);
                        break;
                    }
                    BuildDecision::Rejected(reason) => {
                        entry.status = EntryStatus::Rejected(reason);
                        break;
                    }
                };
                if work.is_empty() || work.len() > state.limits.max_command_bytes {
                    entry.status = EntryStatus::Rejected("empty or oversized reconstructed work".into());
                    break;
                }
                let Some(total) = bytes.checked_add(work.len()).filter(|n| *n <= max_work_bytes) else { break };
                bytes = total;
                reservation.command_ids.push(entry.original.identity.id);
                reservation.work.push(work);
                entry.status = EntryStatus::Reserved { token: reservation.token, phase: ReservationPhase::Held };
                if reservation.command_ids.len() == max_commands {
                    break;
                }
            }
            if reservation.command_ids.is_empty() {
                return Ok(None);
            }
            state.next_token =
                state.next_token.checked_add(1).ok_or_else(|| JournalError::Invalid("reservation token overflow".into()))?;
            state.reservation = Some(reservation.clone());
            Ok(Some(reservation))
        })
    }

    pub fn retry_stale(&self, id: Hash32) -> Result<()> {
        self.update(|state| {
            let entry = find_entry(state, id)?;
            if !matches!(entry.status, EntryStatus::Stale(_)) {
                return Err(JournalError::Conflict("only stale, unsubmitted work may be rebuilt".into()));
            }
            entry.status = EntryStatus::Queued;
            Ok(())
        })
    }

    /// Call under the session writer lock after rechecking the exact head, and
    /// before the external commit. Persistence failure means do not submit.
    pub fn begin_commit(&self, token: u64, current: &SessionHead) -> Result<()> {
        self.update(|state| {
            if state.fencing_policy.is_some() {
                return Err(JournalError::Conflict("fenced journal requires a persisted signed authority submission".into()));
            }
            let reservation = find_reservation(state, token)?;
            reservation.validate_base(current)?;
            if reservation.phase != ReservationPhase::Held {
                return Err(JournalError::Conflict("commit was already started; reconcile".into()));
            }
            set_phase(state, ReservationPhase::Submitting);
            Ok(())
        })
    }

    pub fn mark_unknown(&self, token: u64) -> Result<()> {
        self.update(|state| {
            if find_reservation(state, token)?.phase == ReservationPhase::Held {
                return Err(JournalError::Conflict("commit has not started".into()));
            }
            set_phase(state, ReservationPhase::Unknown);
            Ok(())
        })
    }

    /// Drive the existing committer error contract without exposing a force
    /// release switch. The caller holds the session writer lock throughout;
    /// an uncertain backend result MUST be OutcomeUnknown, never Rejected.
    pub fn submit(
        &self,
        token: u64,
        current: &SessionHead,
        commit: impl FnOnce(
            &Reservation,
        )
            -> std::result::Result<myelin_session_producer::ProductionReceipt, myelin_session_producer::CommitError>,
    ) -> Result<Submission> {
        use myelin_session_producer::CommitError;
        self.begin_commit(token, current)?;
        let reservation = self.active_reservation()?.ok_or_else(|| JournalError::Conflict("reservation disappeared".into()))?;
        match commit(&reservation) {
            Ok(receipt) => Ok(Submission::Committed(receipt)),
            Err(CommitError::OutcomeUnknown(message)) => {
                self.mark_unknown(token)?;
                Ok(Submission::Unknown(message))
            }
            Err(error) => self.update(|state| {
                find_reservation(state, token)?;
                for entry in &mut state.entries {
                    if matches!(entry.status, EntryStatus::Reserved { token: t, .. } if t == token) {
                        entry.status = match &error {
                            CommitError::StaleCandidate => EntryStatus::Queued,
                            CommitError::Rejected(reason) => EntryStatus::Rejected(reason.clone()),
                            CommitError::OutcomeUnknown(_) => unreachable!(),
                        };
                    }
                }
                state.reservation = None;
                Ok(match error {
                    CommitError::StaleCandidate => Submission::Stale,
                    CommitError::Rejected(reason) => Submission::Rejected(reason),
                    CommitError::OutcomeUnknown(_) => unreachable!(),
                })
            }),
        }
    }

    /// Only work which has never crossed begin_commit can be released. There
    /// is deliberately no force/requeue-on-Missing flag. Fenced absence recovery
    /// requires the authoritative writer protocol tracked by P1 #15.
    pub fn release_unsubmitted(&self, token: u64) -> Result<()> {
        self.update(|state| {
            if find_reservation(state, token)?.phase != ReservationPhase::Held {
                return Err(JournalError::Conflict("commit may have happened; Missing does not permit release".into()));
            }
            for entry in &mut state.entries {
                if matches!(entry.status, EntryStatus::Reserved { token: t, .. } if t == token) {
                    entry.status = EntryStatus::Queued;
                }
            }
            state.reservation = None;
            Ok(())
        })
    }

    /// Receipt queries are validated by SessionChain against committed frames.
    /// The application derives an optional business outcome from that exact
    /// record. Missing commands retain their reservation even across restart.
    pub fn reconcile<E: TransitionExecutor, S: SessionStore>(
        &self,
        chain: &SessionChain<E, S>,
        mut interpret: impl FnMut(&FinalisedBlockRecord, &CommandReceipt) -> std::result::Result<Option<BusinessOutcome>, String>,
    ) -> Result<Vec<JournalEntry>> {
        self.update(|state| {
            if state.fencing_policy.is_some() {
                return Err(JournalError::Conflict(
                    "fenced journal requires canonical signed observations, not a replica receipt".into(),
                ));
            }
            if state.scope != CommandScope::from_config(chain.config()) {
                return Err(JournalError::Invalid("reconciliation genesis/profile differs".into()));
            }
            let mut changed = vec![];
            for entry in &mut state.entries {
                if matches!(entry.status, EntryStatus::Committed { outcome: Some(_), .. } | EntryStatus::IdentityConflict(_)) {
                    continue;
                }
                match chain.command_status(&entry.original.identity).map_err(|e| JournalError::Reconciliation(e.to_string()))? {
                    CommandStatus::Missing => continue,
                    CommandStatus::IdentityConflict(receipt) => entry.status = EntryStatus::IdentityConflict(receipt),
                    CommandStatus::Committed(receipt) => {
                        let record = chain
                            .command_record(&entry.original.identity)
                            .map_err(|e| JournalError::Reconciliation(e.to_string()))?
                            .ok_or_else(|| JournalError::Reconciliation("committed command disappeared".into()))?;
                        let outcome = interpret(&record, &receipt).map_err(JournalError::Reconciliation)?;
                        if outcome.as_ref().is_some_and(|value| value.len() > state.limits.max_result_bytes) {
                            return Err(JournalError::Invalid("business result exceeds journal limit".into()));
                        }
                        entry.status = EntryStatus::Committed { receipt, outcome, acknowledged: false };
                    }
                }
                changed.push(entry.clone());
            }
            if !state.entries.iter().any(|entry| matches!(entry.status, EntryStatus::Reserved { .. })) {
                state.reservation = None;
            }
            Ok(changed)
        })
    }

    /// ACK is local durable response bookkeeping; retry can still recover the
    /// original outcome after ACK or a lost HTTP reply.
    pub fn acknowledge(&self, id: Hash32) -> Result<BusinessOutcome> {
        self.update(|state| match &mut find_entry(state, id)?.status {
            EntryStatus::Committed { outcome: Some(outcome), acknowledged, .. } => {
                *acknowledged = true;
                Ok(outcome.clone())
            }
            _ => Err(JournalError::Conflict("no interpreted committed business outcome to acknowledge".into())),
        })
    }
}

fn find_entry(state: &mut State, id: Hash32) -> Result<&mut JournalEntry> {
    state.entries.iter_mut().find(|e| e.original.identity.id == id).ok_or_else(|| JournalError::Conflict("unknown command".into()))
}
fn find_reservation(state: &State, token: u64) -> Result<&Reservation> {
    state.reservation.as_ref().filter(|r| r.token == token).ok_or_else(|| JournalError::Conflict("unknown reservation token".into()))
}
fn set_phase(state: &mut State, phase: ReservationPhase) {
    if let Some(reservation) = &mut state.reservation {
        reservation.phase = phase;
        for entry in &mut state.entries {
            if let EntryStatus::Reserved { phase: p, .. } = &mut entry.status {
                *p = phase;
            }
        }
    }
}
fn validate_command(state: &State, command: &OriginalCommand) -> Result<()> {
    if command.scope != state.scope
        || command.identity.id == [0; 32]
        || command.identity.payload_hash == [0; 32]
        || command.canonical_command.is_empty()
        || command.canonical_command.len() > state.limits.max_command_bytes
        || command.authentication_context.len() > state.limits.max_authorization_bytes
        || command.authorization.len() > state.limits.max_authorization_bytes
    {
        return Err(JournalError::Invalid("command scope/identity or size invalid".into()));
    }
    Ok(())
}
fn validate(state: &State) -> Result<()> {
    validate_update(state, None)
}
fn validate_update(state: &State, previous: Option<&State>) -> Result<()> {
    state.limits.validate()?;
    if state.entries.len() > state.limits.max_entries || state.next_token == 0 {
        return Err(JournalError::Invalid("entry count or reservation counter invalid".into()));
    }
    let mut ids = std::collections::HashSet::new();
    for (index, entry) in state.entries.iter().enumerate() {
        validate_command(state, &entry.original)?;
        if entry.sequence != index as u64
            || !ids.insert(entry.original.identity.id)
            || entry.authorization_digest != entry.original.authorization_digest()
        {
            return Err(JournalError::Invalid("journal order, identity or authorization digest invalid".into()));
        }
        match &entry.status {
            EntryStatus::Reserved { token, phase } => {
                let r = find_reservation(state, *token)?;
                if !r.command_ids.contains(&entry.original.identity.id) || r.phase != *phase {
                    return Err(JournalError::Invalid("reservation/entry mismatch".into()));
                }
            }
            EntryStatus::Committed { receipt, outcome, acknowledged } => {
                if receipt.session_id != state.scope.session_id
                    || receipt.command != entry.original.identity
                    || (*acknowledged && outcome.is_none())
                    || outcome.as_ref().is_some_and(|value| value.len() > state.limits.max_result_bytes)
                {
                    return Err(JournalError::Invalid("committed receipt/result mismatch".into()));
                }
            }
            EntryStatus::IdentityConflict(receipt) => {
                if receipt.session_id != state.scope.session_id
                    || receipt.command.id != entry.original.identity.id
                    || receipt.command.payload_hash == entry.original.identity.payload_hash
                {
                    return Err(JournalError::Invalid("identity conflict receipt mismatch".into()));
                }
            }
            _ => {}
        }
    }
    if let Some(r) = &state.reservation {
        let head = r.base_head()?;
        let unique: std::collections::HashSet<_> = r.command_ids.iter().collect();
        if r.token == 0
            || r.token >= state.next_token
            || r.command_ids.is_empty()
            || r.command_ids.len() != r.work.len()
            || unique.len() != r.command_ids.len()
            || head.session_id != state.scope.session_id
            || head.sealed_by_successor.is_some()
            || r.work.iter().any(|w| w.is_empty() || w.len() > state.limits.max_command_bytes)
            || r.command_ids.iter().any(|id| {
                !state.entries.iter().any(|e| {
                    e.original.identity.id == *id
                        && (matches!(e.status, EntryStatus::Reserved { token, .. } if token == r.token)
                            || matches!(e.status, EntryStatus::Committed { .. } | EntryStatus::IdentityConflict(_)))
                })
            })
            || !state.entries.iter().any(|e| matches!(e.status, EntryStatus::Reserved { token, .. } if token == r.token))
        {
            return Err(JournalError::Invalid("reservation content invalid".into()));
        }
        if let Some(binding) = &r.fenced_submission {
            binding.validate(state, &r.command_ids)?;
            if r.phase == ReservationPhase::Held || binding.authorization.base_head != r.construction_head {
                return Err(JournalError::Invalid("fenced submission phase/construction head mismatch".into()));
            }
        }
    }
    fencing::validate_recoveries(state, previous)?;
    Ok(())
}
fn persist(db: &DB, state: &State) -> Result<()> {
    let body = serde_json::to_vec(state).map_err(|e| JournalError::Invalid(e.to_string()))?;
    if body.len().checked_add(MAGIC.len() + 32).is_none_or(|n| n > state.limits.max_encoded_bytes) {
        return Err(JournalError::Invalid("journal encoded byte capacity reached".into()));
    }
    let mut bytes = MAGIC.to_vec();
    bytes.extend_from_slice(blake3::hash(&body).as_bytes());
    bytes.extend_from_slice(&body);
    let mut options = WriteOptions::default();
    options.set_sync(true);
    db.put_opt(KEY, bytes, &options).map_err(storage)
}
fn decode(bytes: &[u8]) -> Result<State> {
    if bytes.len() < MAGIC.len() + 32 || bytes.len() > MAX_BYTES || &bytes[..MAGIC.len()] != MAGIC {
        return Err(JournalError::Invalid("unsupported journal format or byte bound".into()));
    }
    let body = &bytes[MAGIC.len() + 32..];
    if &bytes[MAGIC.len()..MAGIC.len() + 32] != blake3::hash(body).as_bytes() {
        return Err(JournalError::Invalid("journal checksum mismatch".into()));
    }
    let state: State = serde_json::from_slice(body).map_err(|e| JournalError::Invalid(e.to_string()))?;
    if bytes.len() > state.limits.max_encoded_bytes {
        return Err(JournalError::Invalid("stored journal exceeds configured bytes".into()));
    }
    validate(&state)?;
    Ok(state)
}
fn storage(error: rocksdb::Error) -> JournalError {
    JournalError::Storage(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uncertain_journal_write_requires_reopen_and_cannot_overwrite_durable_outcome() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = State {
            scope: CommandScope { session_id: [1; 32], genesis_commitment: [2; 32], application_profile: [3; 32] },
            limits: JournalLimits { max_entries: 12_000, max_encoded_bytes: MAX_BYTES, ..JournalLimits::default() },
            next_token: 1,
            entries: vec![],
            reservation: None,
            reconciliations: vec![],
            fencing_policy: None,
        };
        // Explicit larger application budgets must retain originals beyond the
        // default 4,096 entries without weakening checksum/byte/error recovery.
        for sequence in 0..12_000u64 {
            let payload = sequence.to_le_bytes().to_vec();
            let original = OriginalCommand {
                scope: state.scope.clone(),
                identity: CommandIdentity { id: *blake3::hash(&payload).as_bytes(), payload_hash: *blake3::hash(&payload).as_bytes() },
                kind: InputKind::UserCommand,
                canonical_command: payload,
                authentication_context: vec![7; 32],
                authorization: vec![8; 64],
            };
            state.entries.push(JournalEntry {
                sequence,
                authorization_digest: original.authorization_digest(),
                original,
                status: EntryStatus::Queued,
            });
        }
        let last = state.entries.last().unwrap().clone();
        let db = DB::open_default(dir.path()).unwrap();
        persist(&db, &state).unwrap();
        let j = CommandJournal {
            db,
            state: Mutex::new(state),
            recovery_required: AtomicBool::new(false),
            fail_after_persist: AtomicBool::new(true),
        };
        assert!(j
            .update(|s| {
                s.next_token = 2;
                Ok(())
            })
            .is_err());
        assert_eq!(j.state.lock().unwrap().next_token, 1);
        assert!(j
            .update(|s| {
                s.next_token = 3;
                Ok(())
            })
            .is_err());
        assert!(j.entries(0, 1).is_err());
        assert!(j.entry([1; 32]).is_err());
        assert!(j.active_reservation().is_err());
        drop(j);
        let db = DB::open_default(dir.path()).unwrap();
        let recovered = decode(&db.get(KEY).unwrap().unwrap()).unwrap();
        assert_eq!(recovered.next_token, 2);
        assert_eq!(recovered.entries.len(), 12_000);
        assert_eq!(recovered.entries.last(), Some(&last));
        let before = db.get(KEY).unwrap().unwrap();
        let mut too_small = recovered;
        too_small.limits.max_encoded_bytes = 1024;
        assert!(persist(&db, &too_small).is_err());
        assert_eq!(db.get(KEY).unwrap().unwrap(), before);
    }
}
