// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers

//! Optional conserved escrow and CKB exit capability for Myelin sessions.
//!
//! This crate does not turn a prepared transaction into a custody claim. An
//! attachment requires a verified finalized CKB open transaction; an exit is
//! `Finalized` only after [`myelin_ckb_adapter::verify_projection`] validates
//! the exact settlement transaction and its complete receipt chain.

use myelin_ckb_adapter::{verify_projection, CkbEvidenceProjection};
use myelin_consensus::{ConsensusEngine, SelectedConsensus};
use myelin_exec::{
    ckb_raw_transaction_hash_molecule, ckb_script_hash_molecule, serialize_script_molecule, CellDep, CellInput, CellOutput, CellTx,
    OutPoint, ProjectionStage, Script,
};
use myelin_session::{decode_cell_snapshot, CellScriptVerifier, FinalisedBlockRecord, Hash32};
use myelin_state::ResolvedStateInput;
use secp256k1::{schnorr::Signature, Message, Secp256k1, XOnlyPublicKey};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, sync::Arc};

const TERMS_DOMAIN: &[u8] = b"myelin:session-escrow:terms";
const AUTH_DOMAIN: &[u8] = b"myelin:session-escrow:authorization";
const STATE_DOMAIN: &[u8] = b"myelin:session-escrow:state";
const EXIT_DOMAIN: &[u8] = b"myelin:session-escrow:exit";
const STATE_MAGIC: &[u8; 4] = b"MESC";
const STATE_VERSION: u16 = 1;
const MAX_PARTICIPANTS: usize = 4096;
const MAX_ROLE_BYTES: usize = 4096;
const SHANNONS_PER_CKB: u64 = 100_000_000;

/// Asset held by a session escrow capability.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum AssetDescriptor {
    /// Native CKB capacity measured in shannons.
    CkbCapacity,
    /// A fungible typed token identified by its full type-script hash.
    TypedToken {
        /// Canonical CKB script hash.
        type_script_hash: Hash32,
    },
}

/// Participant authorization and bounded economic exposure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EscrowParticipant {
    /// Canonically ordered application-independent identity.
    pub participant_id: Hash32,
    /// Role committed by the participants, opaque to the capability.
    pub authorization_role: Vec<u8>,
    /// Public key that authorizes the terms.
    pub authorization_key: Hash32,
    /// Exact CKB payout lock.
    pub payout_lock: Script,
    /// Initial asset units deposited.
    pub deposit: u128,
    /// Maximum permitted loss relative to the initial deposit.
    pub max_debit: u128,
}

/// One concrete L1 escrow output created by the open transaction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EscrowDeposit {
    /// Participant funded by this output.
    pub participant_id: Hash32,
    /// Real CKB outpoint created by the open transaction.
    pub outpoint: OutPoint,
}

/// Immutable optional escrow capability terms.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EscrowTerms {
    /// Session chain this capability is attached to.
    pub session_id: Hash32,
    /// Participants sorted strictly by `participant_id`.
    pub participants: Vec<EscrowParticipant>,
    /// One L1 escrow output per participant, sorted by participant id.
    pub deposits: Vec<EscrowDeposit>,
    /// Asset semantics.
    pub asset: AssetDescriptor,
    /// Latest L1 timestamp at which activation may begin.
    pub session_start_deadline_ms: u64,
    /// Absolute session expiry timestamp.
    pub session_expiry_ms: u64,
    /// Challenge period for a unilateral close.
    pub challenge_window_ms: u64,
    /// Minimum CKB confirmations required for open and close.
    pub min_confirmations: u64,
    /// Full script hash required on every L1 escrow output.
    pub escrow_lock_script_hash: Hash32,
    /// Exact type script of the off-chain conserved ledger Cell.
    pub ledger_type_script: Script,
}

impl EscrowTerms {
    /// Validate canonical ordering, keys, bounds, and one-to-one deposits.
    pub fn validate(&self) -> Result<(), EscrowError> {
        if self.session_id == [0; 32] {
            return Err(EscrowError::InvalidTerms("session_id must not be zero".to_owned()));
        }
        if self.participants.is_empty() || self.participants.len() > MAX_PARTICIPANTS {
            return Err(EscrowError::InvalidTerms(format!("participant count must be 1..={MAX_PARTICIPANTS}")));
        }
        if self.deposits.len() != self.participants.len() {
            return Err(EscrowError::InvalidTerms("each participant must have exactly one escrow deposit".to_owned()));
        }
        if self.session_start_deadline_ms >= self.session_expiry_ms {
            return Err(EscrowError::InvalidTerms("session expiry must be later than start deadline".to_owned()));
        }
        if self.challenge_window_ms == 0 || self.min_confirmations == 0 {
            return Err(EscrowError::InvalidTerms("challenge window and confirmations must be non-zero".to_owned()));
        }
        if self.escrow_lock_script_hash == [0; 32] {
            return Err(EscrowError::InvalidTerms("escrow lock script hash must not be zero".to_owned()));
        }
        let mut previous = None;
        let mut keys = HashSet::new();
        for participant in &self.participants {
            if previous.is_some_and(|value| value >= participant.participant_id) {
                return Err(EscrowError::InvalidTerms("participants must be strictly ordered".to_owned()));
            }
            previous = Some(participant.participant_id);
            if participant.authorization_role.is_empty() || participant.authorization_role.len() > MAX_ROLE_BYTES {
                return Err(EscrowError::InvalidTerms("authorization role is empty or too large".to_owned()));
            }
            XOnlyPublicKey::from_slice(&participant.authorization_key)
                .map_err(|error| EscrowError::InvalidTerms(format!("participant authorization key is invalid: {error}")))?;
            if !keys.insert(participant.authorization_key) {
                return Err(EscrowError::InvalidTerms("participant authorization keys must be unique".to_owned()));
            }
            if participant.deposit == 0 || participant.max_debit > participant.deposit {
                return Err(EscrowError::InvalidTerms("deposit must be non-zero and max_debit <= deposit".to_owned()));
            }
            if matches!(&self.asset, AssetDescriptor::CkbCapacity) {
                let minimum = minimum_ckb_capacity_shannons(&participant.payout_lock, None, 0).map_err(EscrowError::InvalidTerms)?;
                if participant.deposit < u128::from(minimum) {
                    return Err(EscrowError::InvalidTerms(format!(
                        "CKB capacity deposit for participant {:?} is below the {} shannon payout minimum",
                        participant.participant_id, minimum
                    )));
                }
            }
        }
        let mut outpoints = HashSet::new();
        for (participant, deposit) in self.participants.iter().zip(&self.deposits) {
            if participant.participant_id != deposit.participant_id {
                return Err(EscrowError::InvalidTerms("deposits must follow participant ordering".to_owned()));
            }
            if !outpoints.insert(deposit.outpoint) {
                return Err(EscrowError::InvalidTerms("escrow outpoints must be unique".to_owned()));
            }
        }
        if let AssetDescriptor::TypedToken { type_script_hash } = self.asset {
            if type_script_hash == [0; 32] {
                return Err(EscrowError::InvalidTerms("typed-token script hash must not be zero".to_owned()));
            }
        }
        Ok(())
    }

    /// Canonical commitment signed by every participant.
    pub fn hash(&self) -> Result<Hash32, EscrowError> {
        self.validate()?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(TERMS_DOMAIN);
        hasher.update(&self.session_id);
        hasher.update(&(self.participants.len() as u32).to_le_bytes());
        for participant in &self.participants {
            hasher.update(&participant.participant_id);
            put_bytes(&mut hasher, &participant.authorization_role);
            hasher.update(&participant.authorization_key);
            put_bytes(&mut hasher, &serialize_script_molecule(&participant.payout_lock).map_err(codec)?);
            hasher.update(&participant.deposit.to_le_bytes());
            hasher.update(&participant.max_debit.to_le_bytes());
        }
        for deposit in &self.deposits {
            hasher.update(&deposit.participant_id);
            hasher.update(&deposit.outpoint.tx_hash);
            hasher.update(&deposit.outpoint.index.to_le_bytes());
        }
        match self.asset {
            AssetDescriptor::CkbCapacity => {
                hasher.update(&[0]);
            }
            AssetDescriptor::TypedToken { type_script_hash } => {
                hasher.update(&[1]);
                hasher.update(&type_script_hash);
            }
        };
        hasher.update(&self.session_start_deadline_ms.to_le_bytes());
        hasher.update(&self.session_expiry_ms.to_le_bytes());
        hasher.update(&self.challenge_window_ms.to_le_bytes());
        hasher.update(&self.min_confirmations.to_le_bytes());
        hasher.update(&self.escrow_lock_script_hash);
        put_bytes(&mut hasher, &serialize_script_molecule(&self.ledger_type_script).map_err(codec)?);
        Ok(*hasher.finalize().as_bytes())
    }

    /// Verify that every configured participant signed the exact terms.
    pub fn verify_authorizations(&self, authorizations: &[EscrowAuthorization]) -> Result<(), EscrowError> {
        let terms_hash = self.hash()?;
        if authorizations.len() != self.participants.len() {
            return Err(EscrowError::Authorization("all participants must authorize terms exactly once".to_owned()));
        }
        let mut seen = HashSet::new();
        for authorization in authorizations {
            if !seen.insert(authorization.participant_id) {
                return Err(EscrowError::Authorization("duplicate participant authorization".to_owned()));
            }
            let participant = self
                .participants
                .iter()
                .find(|participant| participant.participant_id == authorization.participant_id)
                .ok_or_else(|| EscrowError::Authorization("authorization is from an unknown participant".to_owned()))?;
            let signature = Signature::from_slice(&authorization.signature)
                .map_err(|_| EscrowError::Authorization("authorization signature must be exactly 64 bytes".to_owned()))?;
            let public_key = XOnlyPublicKey::from_slice(&participant.authorization_key)
                .map_err(|error| EscrowError::Authorization(error.to_string()))?;
            Secp256k1::verification_only()
                .verify_schnorr(
                    &signature,
                    &Message::from_digest(authorization_digest(terms_hash, participant.participant_id)),
                    &public_key,
                )
                .map_err(|_| EscrowError::Authorization("invalid terms authorization signature".to_owned()))?;
        }
        Ok(())
    }
}

/// One participant's signature over the escrow terms commitment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EscrowAuthorization {
    /// Authorized participant.
    pub participant_id: Hash32,
    /// 64-byte Schnorr signature.
    pub signature: Vec<u8>,
}

/// Canonically ordered participant balance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParticipantBalance {
    /// Participant identity.
    pub participant_id: Hash32,
    /// Current asset units.
    pub balance: u128,
}

/// Conserved state stored as full data in one Myelin ledger Cell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EscrowState {
    /// Session binding.
    pub session_id: Hash32,
    /// Terms binding.
    pub terms_hash: Hash32,
    /// Monotonic application-independent update number.
    pub epoch: u64,
    /// Hash of the exact previous state, or zero for genesis.
    pub previous_state_hash: Hash32,
    /// Canonically ordered balances.
    pub ledger: Vec<ParticipantBalance>,
    /// Application evidence root for this economic update.
    pub application_evidence_root: Hash32,
}

impl EscrowState {
    /// Construct the genesis ledger from deposits.
    pub fn genesis(terms: &EscrowTerms) -> Result<Self, EscrowError> {
        Ok(Self {
            session_id: terms.session_id,
            terms_hash: terms.hash()?,
            epoch: 0,
            previous_state_hash: [0; 32],
            ledger: terms
                .participants
                .iter()
                .map(|participant| ParticipantBalance { participant_id: participant.participant_id, balance: participant.deposit })
                .collect(),
            application_evidence_root: [0; 32],
        })
    }

    /// Validate conservation, identities, ordering, and debit caps.
    pub fn validate(&self, terms: &EscrowTerms) -> Result<(), EscrowError> {
        terms.validate()?;
        if self.session_id != terms.session_id || self.terms_hash != terms.hash()? {
            return Err(EscrowError::InvalidState("state is bound to different terms".to_owned()));
        }
        if self.ledger.len() != terms.participants.len() {
            return Err(EscrowError::InvalidState("ledger participant count mismatch".to_owned()));
        }
        let mut initial_total = 0u128;
        let mut current_total = 0u128;
        for (balance, participant) in self.ledger.iter().zip(&terms.participants) {
            if balance.participant_id != participant.participant_id {
                return Err(EscrowError::InvalidState("ledger identities are not in canonical terms order".to_owned()));
            }
            initial_total = initial_total.checked_add(participant.deposit).ok_or(EscrowError::AmountOverflow)?;
            current_total = current_total.checked_add(balance.balance).ok_or(EscrowError::AmountOverflow)?;
            let debit = participant.deposit.saturating_sub(balance.balance);
            if debit > participant.max_debit {
                return Err(EscrowError::InvalidState(format!("participant {:?} exceeds max debit", participant.participant_id)));
            }
            if matches!(&terms.asset, AssetDescriptor::CkbCapacity) && balance.balance != 0 {
                let minimum = minimum_ckb_capacity_shannons(&participant.payout_lock, None, 0).map_err(EscrowError::InvalidState)?;
                if balance.balance < u128::from(minimum) {
                    return Err(EscrowError::InvalidState(format!(
                        "participant {:?} has a non-zero CKB balance below the {} shannon payout minimum",
                        participant.participant_id, minimum
                    )));
                }
            }
        }
        if initial_total != current_total {
            return Err(EscrowError::InvalidState("ledger does not conserve deposited assets".to_owned()));
        }
        Ok(())
    }

    /// Validate exact successor linkage and terms invariants.
    pub fn validate_successor(&self, next: &Self, terms: &EscrowTerms) -> Result<(), EscrowError> {
        self.validate(terms)?;
        next.validate(terms)?;
        if next.epoch != self.epoch.checked_add(1).ok_or(EscrowError::AmountOverflow)? {
            return Err(EscrowError::InvalidState("escrow epoch must increment by one".to_owned()));
        }
        if next.previous_state_hash != self.hash()? {
            return Err(EscrowError::InvalidState("escrow successor is not linked to the exact prior state".to_owned()));
        }
        if next.application_evidence_root == [0; 32] {
            return Err(EscrowError::InvalidState("economic update must carry application evidence".to_owned()));
        }
        Ok(())
    }

    /// Canonical state data stored in the ledger Cell.
    pub fn encode(&self) -> Result<Vec<u8>, EscrowError> {
        let count = u32::try_from(self.ledger.len()).map_err(|_| EscrowError::Codec("too many balances".to_owned()))?;
        let mut bytes = Vec::with_capacity(4 + 2 + 32 * 4 + 8 + 4 + self.ledger.len() * 48);
        bytes.extend_from_slice(STATE_MAGIC);
        bytes.extend_from_slice(&STATE_VERSION.to_le_bytes());
        bytes.extend_from_slice(&self.session_id);
        bytes.extend_from_slice(&self.terms_hash);
        bytes.extend_from_slice(&self.epoch.to_le_bytes());
        bytes.extend_from_slice(&self.previous_state_hash);
        bytes.extend_from_slice(&self.application_evidence_root);
        bytes.extend_from_slice(&count.to_le_bytes());
        for balance in &self.ledger {
            bytes.extend_from_slice(&balance.participant_id);
            bytes.extend_from_slice(&balance.balance.to_le_bytes());
        }
        Ok(bytes)
    }

    /// Decode exact state data, rejecting trailing or oversized values.
    pub fn decode(bytes: &[u8]) -> Result<Self, EscrowError> {
        let mut decoder = Decoder::new(bytes);
        if decoder.take(4)? != STATE_MAGIC {
            return Err(EscrowError::Codec("wrong escrow state magic".to_owned()));
        }
        let version = u16::from_le_bytes(decoder.take(2)?.try_into().expect("fixed length"));
        if version != STATE_VERSION {
            return Err(EscrowError::Codec(format!("unsupported escrow state version {version}")));
        }
        let session_id = decoder.hash()?;
        let terms_hash = decoder.hash()?;
        let epoch = u64::from_le_bytes(decoder.take(8)?.try_into().expect("fixed length"));
        let previous_state_hash = decoder.hash()?;
        let application_evidence_root = decoder.hash()?;
        let count = u32::from_le_bytes(decoder.take(4)?.try_into().expect("fixed length")) as usize;
        if count == 0 || count > MAX_PARTICIPANTS {
            return Err(EscrowError::Codec("invalid escrow ledger count".to_owned()));
        }
        let mut ledger = Vec::with_capacity(count);
        for _ in 0..count {
            ledger.push(ParticipantBalance {
                participant_id: decoder.hash()?,
                balance: u128::from_le_bytes(decoder.take(16)?.try_into().expect("fixed length")),
            });
        }
        if !decoder.remaining.is_empty() {
            return Err(EscrowError::Codec("trailing escrow state bytes".to_owned()));
        }
        Ok(Self { session_id, terms_hash, epoch, previous_state_hash, ledger, application_evidence_root })
    }

    /// Domain-separated hash of encoded state.
    pub fn hash(&self) -> Result<Hash32, EscrowError> {
        let mut hasher = blake3::Hasher::new();
        hasher.update(STATE_DOMAIN);
        hasher.update(&self.encode()?);
        Ok(*hasher.finalize().as_bytes())
    }
}

/// Strict-VM verifier decorator that also enforces native escrow invariants.
///
/// The delegate always runs first. For session/court use it must be a CKB
/// strict verifier; this decorator never creates an always-success shortcut.
pub struct EscrowCellVerifier {
    terms: EscrowTerms,
    delegate: Arc<dyn CellScriptVerifier>,
}

impl EscrowCellVerifier {
    /// Bind exact terms to a mandatory base verifier.
    pub fn new(terms: EscrowTerms, delegate: Arc<dyn CellScriptVerifier>) -> Result<Self, EscrowError> {
        terms.validate()?;
        Ok(Self { terms, delegate })
    }

    fn verify_escrow_transition(&self, tx: &CellTx, inputs: &[ResolvedStateInput]) -> Result<(), EscrowError> {
        let escrow_inputs =
            inputs.iter().filter(|input| input.cell.type_script.as_ref() == Some(&self.terms.ledger_type_script)).collect::<Vec<_>>();
        let escrow_outputs = tx
            .outputs
            .iter()
            .zip(&tx.outputs_data)
            .filter(|(output, _)| output.type_.as_ref() == Some(&self.terms.ledger_type_script))
            .collect::<Vec<_>>();
        if escrow_inputs.is_empty() && escrow_outputs.is_empty() {
            return Ok(());
        }
        if escrow_inputs.len() != 1 || escrow_outputs.len() != 1 {
            return Err(EscrowError::InvalidTransition("an escrow update must consume and create exactly one ledger Cell".to_owned()));
        }
        let input = escrow_inputs[0];
        let input_data = input
            .cell
            .data
            .as_deref()
            .ok_or_else(|| EscrowError::InvalidTransition("escrow input is missing full data".to_owned()))?;
        let input_lock = input
            .cell
            .lock_script
            .as_ref()
            .ok_or_else(|| EscrowError::InvalidTransition("escrow input is missing full lock script".to_owned()))?;
        let (output, output_data) = escrow_outputs[0];
        if output.lock != *input_lock || output.capacity != input.cell.capacity {
            return Err(EscrowError::InvalidTransition("ledger Cell lock and capacity must be preserved".to_owned()));
        }
        let previous = EscrowState::decode(input_data)?;
        let next = EscrowState::decode(output_data)?;
        previous.validate_successor(&next, &self.terms)
    }
}

impl CellScriptVerifier for EscrowCellVerifier {
    fn verify(&self, tx: &CellTx, resolved_inputs: &[ResolvedStateInput]) -> std::result::Result<u64, String> {
        let cycles = self.delegate.verify(tx, resolved_inputs)?;
        self.verify_escrow_transition(tx, resolved_inputs).map_err(|error| error.to_string())?;
        Ok(cycles)
    }
}

/// Verified L1 funding attachment.
#[derive(Clone, Debug)]
pub struct EscrowAttachment {
    /// Validated immutable terms.
    terms: EscrowTerms,
    /// Participant authorizations over the exact terms hash.
    authorizations: Vec<EscrowAuthorization>,
    /// Finalized CKB receipt chain for the exact open transaction.
    open_evidence: CkbEvidenceProjection,
}

impl EscrowAttachment {
    /// Validated immutable terms.
    pub fn terms(&self) -> &EscrowTerms {
        &self.terms
    }

    /// Verified participant authorizations.
    pub fn authorizations(&self) -> &[EscrowAuthorization] {
        &self.authorizations
    }

    /// Finalized CKB receipt chain for the exact open transaction.
    pub fn open_evidence(&self) -> &CkbEvidenceProjection {
        &self.open_evidence
    }

    /// Verify authorizations, finalized receipt depth, outpoints, scripts, and amounts.
    pub fn attach(
        terms: EscrowTerms,
        authorizations: Vec<EscrowAuthorization>,
        open_tx: &CellTx,
        open_evidence: CkbEvidenceProjection,
    ) -> Result<Self, EscrowError> {
        terms.verify_authorizations(&authorizations)?;
        verify_projection(open_tx, &open_evidence).map_err(EscrowError::Ckb)?;
        if open_evidence.stage != ProjectionStage::Finalized {
            return Err(EscrowError::OpenEvidence("escrow open transaction is not finalized".to_owned()));
        }
        let finalized =
            open_evidence.finalized.as_ref().ok_or_else(|| EscrowError::OpenEvidence("finalized receipt is missing".to_owned()))?;
        if finalized.confirmations < terms.min_confirmations {
            return Err(EscrowError::OpenEvidence("open transaction has insufficient confirmation depth".to_owned()));
        }
        if finalized.tip.timestamp > terms.session_start_deadline_ms {
            return Err(EscrowError::OpenEvidence(
                "escrow funding did not reach required finality before the session start deadline".to_owned(),
            ));
        }
        let raw_tx_hash = open_evidence.raw_tx_hash;
        for (participant, deposit) in terms.participants.iter().zip(&terms.deposits) {
            if deposit.outpoint.tx_hash != raw_tx_hash {
                return Err(EscrowError::OpenEvidence("escrow outpoint does not belong to the finalized open transaction".to_owned()));
            }
            let output = open_tx
                .outputs
                .get(deposit.outpoint.index as usize)
                .ok_or_else(|| EscrowError::OpenEvidence("escrow outpoint index is out of bounds".to_owned()))?;
            let data = &open_tx.outputs_data[deposit.outpoint.index as usize];
            let lock_hash = ckb_script_hash_molecule(&output.lock).map_err(codec)?;
            if lock_hash != terms.escrow_lock_script_hash {
                return Err(EscrowError::OpenEvidence("escrow output uses an unpinned lock script".to_owned()));
            }
            match terms.asset {
                AssetDescriptor::CkbCapacity => {
                    if output.type_.is_some() || !data.is_empty() || u128::from(output.capacity) != participant.deposit {
                        return Err(EscrowError::OpenEvidence("CKB capacity escrow output does not match its deposit".to_owned()));
                    }
                }
                AssetDescriptor::TypedToken { type_script_hash } => {
                    let type_script = output
                        .type_
                        .as_ref()
                        .ok_or_else(|| EscrowError::OpenEvidence("typed-token deposit has no type script".to_owned()))?;
                    if ckb_script_hash_molecule(type_script).map_err(codec)? != type_script_hash
                        || decode_u128_token(data)? != participant.deposit
                    {
                        return Err(EscrowError::OpenEvidence("typed-token escrow output does not match its deposit".to_owned()));
                    }
                }
            }
        }
        Ok(Self { terms, authorizations, open_evidence })
    }
}

/// External capacity used to pay a CKB settlement fee without reducing escrow balances.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeeFunding {
    /// Live funding Cell.
    pub outpoint: OutPoint,
    /// Resolved capacity of the funding Cell.
    pub capacity: u64,
    /// Lock receiving remaining capacity.
    pub change_lock: Script,
}

/// Cooperative capacity settlement configuration.
#[derive(Clone, Debug)]
pub struct CapacityExitConfig {
    /// Script dependencies required by escrow and participant locks.
    pub cell_deps: Vec<CellDep>,
    /// Optional fee funding input.
    pub fee_funding: Option<FeeFunding>,
    /// Exact fee paid from the funding input.
    pub fee: u64,
}

/// Pluggable asset-specific exit transaction constructor.
pub trait EscrowExitBackend: Send + Sync {
    /// Asset descriptor accepted by this backend.
    fn asset(&self) -> &AssetDescriptor;

    /// Construct an unsigned, evidence-bound settlement transaction.
    fn prepare(
        &self,
        attachment: &EscrowAttachment,
        consensus: &SelectedConsensus,
        latest: &FinalisedBlockRecord,
    ) -> Result<PreparedExit, EscrowError>;
}

/// Asset-neutral prepared-exit dispatch value.
#[derive(Clone, Debug)]
pub enum PreparedExit {
    /// Native CKB-capacity settlement.
    CkbCapacity(PreparedCapacityExit),
    /// Plugin-owned typed-token transaction and proof payload.
    TypedToken {
        /// Exact asset descriptor handled by the plugin.
        asset: AssetDescriptor,
        /// Unsigned transaction.
        unsigned_tx: CellTx,
        /// Raw CKB transaction hash.
        raw_tx_hash: Hash32,
        /// Finalized Myelin proof payload.
        evidence: ExitEvidence,
    },
}

/// Built-in native-capacity exit backend.
#[derive(Clone, Debug)]
pub struct CkbCapacityExitBackend {
    /// Inputs, dependencies, and fee policy.
    pub config: CapacityExitConfig,
}

impl EscrowExitBackend for CkbCapacityExitBackend {
    fn asset(&self) -> &AssetDescriptor {
        static ASSET: AssetDescriptor = AssetDescriptor::CkbCapacity;
        &ASSET
    }

    fn prepare(
        &self,
        attachment: &EscrowAttachment,
        consensus: &SelectedConsensus,
        latest: &FinalisedBlockRecord,
    ) -> Result<PreparedExit, EscrowError> {
        attachment.prepare_capacity_exit(consensus, latest, self.config.clone()).map(PreparedExit::CkbCapacity)
    }
}

/// Proof payload that a real deployed settlement lock must validate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExitEvidence {
    /// Session identifier.
    pub session_id: Hash32,
    /// Escrow terms commitment.
    pub terms_hash: Hash32,
    /// Finalized Myelin block hash.
    pub block_hash: Hash32,
    /// Finalized session-local height.
    pub block_number: u64,
    /// Exact post-state root containing the ledger Cell.
    pub state_root: Hash32,
    /// Exact ledger state hash recovered from the persisted snapshot.
    pub escrow_state_hash: Hash32,
    /// Serialized finalized block and engine-specific proof.
    pub finalised_block_record: Vec<u8>,
}

impl ExitEvidence {
    /// Domain-separated commitment for witness assemblers and audit logs.
    pub fn hash(&self) -> Result<Hash32, EscrowError> {
        let bytes = serde_json::to_vec(self).map_err(codec)?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(EXIT_DOMAIN);
        hasher.update(&bytes);
        Ok(*hasher.finalize().as_bytes())
    }
}

/// Unsigned raw transaction plus the proof payload required by the escrow lock.
#[derive(Clone, Debug)]
pub struct PreparedCapacityExit {
    /// Transaction with final inputs/outputs/deps and no lock witnesses.
    pub unsigned_tx: CellTx,
    /// Raw CKB transaction hash participants must sign.
    pub raw_tx_hash: Hash32,
    /// Latest conserved ledger.
    pub state: EscrowState,
    /// Myelin finality/state binding for settlement script witnesses.
    pub evidence: ExitEvidence,
}

/// Pluggable lock-specific witness construction and signature collection.
pub trait SettlementWitnessAssembler: Send + Sync {
    /// Return the complete CKB witness vector for the exact unsigned raw transaction.
    fn assemble_witnesses(&self, unsigned_tx: &CellTx, evidence: &ExitEvidence) -> std::result::Result<Vec<Vec<u8>>, String>;
}

impl PreparedCapacityExit {
    /// Install lock-specific witnesses while proving the raw transaction did not change.
    pub fn assemble(self, assembler: &dyn SettlementWitnessAssembler) -> Result<CellTx, EscrowError> {
        let mut tx = self.unsigned_tx;
        tx.witnesses = assembler.assemble_witnesses(&tx, &self.evidence).map_err(EscrowError::Witness)?;
        let actual = ckb_raw_transaction_hash_molecule(&tx).map_err(codec)?;
        if actual != self.raw_tx_hash {
            return Err(EscrowError::Witness("witness assembly changed the raw transaction hash".to_owned()));
        }
        Ok(tx)
    }
}

impl EscrowAttachment {
    /// Build the cooperative CKB-capacity exit from an exactly recovered,
    /// finalized Myelin block. Typed-token settlement is provided by separate
    /// asset-specific builders so token data rules are never guessed.
    pub fn prepare_capacity_exit(
        &self,
        consensus: &SelectedConsensus,
        latest: &FinalisedBlockRecord,
        config: CapacityExitConfig,
    ) -> Result<PreparedCapacityExit, EscrowError> {
        if self.terms.asset != AssetDescriptor::CkbCapacity {
            return Err(EscrowError::UnsupportedAsset("capacity exit builder cannot settle typed tokens".to_owned()));
        }
        consensus.finalise_with_proof(latest.block.clone(), latest.proof.clone()).map_err(EscrowError::Consensus)?;
        if latest.block.consensus_kind != consensus.kind() {
            return Err(EscrowError::InvalidExit("finalized block uses a different consensus engine".to_owned()));
        }
        let mut state_tree =
            decode_cell_snapshot(&latest.state_snapshot).map_err(|error| EscrowError::InvalidExit(error.to_string()))?;
        if state_tree.root().as_bytes() != latest.block.state_root_after {
            return Err(EscrowError::InvalidExit("persisted snapshot does not match finalized state root".to_owned()));
        }
        let ledger_cells = state_tree
            .iter_by_outpoint()
            .filter(|(_, entry)| entry.type_script.as_ref() == Some(&self.terms.ledger_type_script))
            .collect::<Vec<_>>();
        if ledger_cells.len() != 1 {
            return Err(EscrowError::InvalidExit("finalized snapshot must contain exactly one escrow ledger Cell".to_owned()));
        }
        let state = EscrowState::decode(
            ledger_cells[0]
                .1
                .data
                .as_deref()
                .ok_or_else(|| EscrowError::InvalidExit("ledger Cell is missing full data".to_owned()))?,
        )?;
        state.validate(&self.terms)?;

        let mut inputs = self.terms.deposits.iter().map(|deposit| CellInput::new(deposit.outpoint, 0)).collect::<Vec<_>>();
        let mut outputs = Vec::with_capacity(self.terms.participants.len() + usize::from(config.fee_funding.is_some()));
        for (participant, balance) in self.terms.participants.iter().zip(&state.ledger) {
            if balance.balance == 0 {
                continue;
            }
            let capacity =
                u64::try_from(balance.balance).map_err(|_| EscrowError::InvalidExit("CKB capacity balance exceeds u64".to_owned()))?;
            let output = CellOutput { capacity, lock: participant.payout_lock.clone(), type_: None };
            verify_ckb_capacity(&output, 0).map_err(EscrowError::InvalidExit)?;
            outputs.push(output);
        }
        if let Some(funding) = &config.fee_funding {
            if self.terms.deposits.iter().any(|deposit| deposit.outpoint == funding.outpoint) {
                return Err(EscrowError::InvalidExit("fee input duplicates an escrow input".to_owned()));
            }
            if funding.capacity < config.fee {
                return Err(EscrowError::InvalidExit("fee funding capacity is below the requested fee".to_owned()));
            }
            inputs.push(CellInput::new(funding.outpoint, 0));
            let change = funding.capacity - config.fee;
            if change > 0 {
                let output = CellOutput { capacity: change, lock: funding.change_lock.clone(), type_: None };
                verify_ckb_capacity(&output, 0).map_err(EscrowError::InvalidExit)?;
                outputs.push(output);
            }
        } else if config.fee != 0 {
            return Err(EscrowError::InvalidExit("non-zero fee requires an external funding input".to_owned()));
        }
        let output_data = vec![Vec::new(); outputs.len()];
        let unsigned_tx = CellTx::new(inputs, config.cell_deps, outputs, output_data, Vec::new())
            .map_err(|error| EscrowError::InvalidExit(error.to_owned()))?;
        let raw_tx_hash = ckb_raw_transaction_hash_molecule(&unsigned_tx).map_err(codec)?;
        let evidence = ExitEvidence {
            session_id: self.terms.session_id,
            terms_hash: self.terms.hash()?,
            block_hash: latest.block.hash(),
            block_number: latest.block.number,
            state_root: latest.block.state_root_after,
            escrow_state_hash: state.hash()?,
            finalised_block_record: latest.encode().map_err(|error| EscrowError::InvalidExit(error.to_string()))?,
        };
        Ok(PreparedCapacityExit { unsigned_tx, raw_tx_hash, state, evidence })
    }
}

/// Verified finalized CKB settlement receipt.
#[derive(Clone, Debug)]
pub struct FinalizedSettlement {
    /// Exact transaction that reached configured depth.
    transaction: CellTx,
    /// Offline-verifiable receipt chain.
    evidence: CkbEvidenceProjection,
}

impl FinalizedSettlement {
    /// Exact transaction that reached configured CKB depth.
    pub fn transaction(&self) -> &CellTx {
        &self.transaction
    }

    /// Offline-verifiable finalized CKB receipt chain.
    pub fn evidence(&self) -> &CkbEvidenceProjection {
        &self.evidence
    }

    /// Accept only a finalized, internally linked projection for the exact transaction.
    pub fn verify(transaction: CellTx, evidence: CkbEvidenceProjection, min_confirmations: u64) -> Result<Self, EscrowError> {
        verify_projection(&transaction, &evidence).map_err(EscrowError::Ckb)?;
        let finalized = evidence
            .finalized
            .as_ref()
            .ok_or_else(|| EscrowError::InvalidExit("settlement has no finalized CKB receipt".to_owned()))?;
        if evidence.stage != ProjectionStage::Finalized || finalized.confirmations < min_confirmations {
            return Err(EscrowError::InvalidExit("settlement did not reach required CKB depth".to_owned()));
        }
        Ok(Self { transaction, evidence })
    }
}

fn authorization_digest(terms_hash: Hash32, participant_id: Hash32) -> Hash32 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(AUTH_DOMAIN);
    hasher.update(&terms_hash);
    hasher.update(&participant_id);
    *hasher.finalize().as_bytes()
}

fn decode_u128_token(data: &[u8]) -> Result<u128, EscrowError> {
    if data.len() != 16 {
        return Err(EscrowError::OpenEvidence("typed-token amount must be exactly one little-endian u128".to_owned()));
    }
    Ok(u128::from_le_bytes(data.try_into().expect("checked length")))
}

fn minimum_ckb_capacity_shannons(lock: &Script, type_script: Option<&Script>, data_len: usize) -> Result<u64, String> {
    let lock_bytes = 33usize.checked_add(lock.args.len()).ok_or_else(|| "CKB lock script size overflow".to_owned())?;
    let type_bytes = type_script.map_or(Ok(0usize), |script| {
        33usize.checked_add(script.args.len()).ok_or_else(|| "CKB type script size overflow".to_owned())
    })?;
    let occupied_bytes = 8usize
        .checked_add(lock_bytes)
        .and_then(|value| value.checked_add(type_bytes))
        .and_then(|value| value.checked_add(data_len))
        .ok_or_else(|| "CKB occupied capacity size overflow".to_owned())?;
    u64::try_from(occupied_bytes)
        .ok()
        .and_then(|value| value.checked_mul(SHANNONS_PER_CKB))
        .ok_or_else(|| "CKB occupied capacity exceeds u64 shannons".to_owned())
}

fn verify_ckb_capacity(output: &CellOutput, data_len: usize) -> Result<(), String> {
    let minimum = minimum_ckb_capacity_shannons(&output.lock, output.type_.as_ref(), data_len)?;
    if output.capacity < minimum {
        return Err(format!("CKB output capacity {} is below the {} shannon occupied-capacity minimum", output.capacity, minimum));
    }
    Ok(())
}

fn put_bytes(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    hasher.update(&(bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

fn codec(error: impl std::fmt::Display) -> EscrowError {
    EscrowError::Codec(error.to_string())
}

struct Decoder<'a> {
    remaining: &'a [u8],
}

impl<'a> Decoder<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { remaining: bytes }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], EscrowError> {
        if self.remaining.len() < len {
            return Err(EscrowError::Codec("truncated escrow state".to_owned()));
        }
        let (value, remaining) = self.remaining.split_at(len);
        self.remaining = remaining;
        Ok(value)
    }

    fn hash(&mut self) -> Result<Hash32, EscrowError> {
        Ok(self.take(32)?.try_into().expect("fixed length"))
    }
}

/// Escrow capability failures.
#[derive(Debug, thiserror::Error)]
pub enum EscrowError {
    /// Immutable terms are invalid.
    #[error("invalid escrow terms: {0}")]
    InvalidTerms(String),
    /// Participant authorization is absent or invalid.
    #[error("escrow authorization failed: {0}")]
    Authorization(String),
    /// Ledger state violates conservation/linkage/caps.
    #[error("invalid escrow state: {0}")]
    InvalidState(String),
    /// Escrow Cell transition is malformed.
    #[error("invalid escrow transition: {0}")]
    InvalidTransition(String),
    /// L1 open evidence is insufficient or inconsistent.
    #[error("invalid escrow open evidence: {0}")]
    OpenEvidence(String),
    /// Exit preparation or final receipt is inconsistent.
    #[error("invalid escrow exit: {0}")]
    InvalidExit(String),
    /// Asset requires a separately installed settlement implementation.
    #[error("unsupported escrow asset: {0}")]
    UnsupportedAsset(String),
    /// Asset arithmetic overflowed.
    #[error("escrow amount overflow")]
    AmountOverflow,
    /// Canonical encoding failed.
    #[error("escrow codec failed: {0}")]
    Codec(String),
    /// Lock-specific witness assembly failed.
    #[error("settlement witness assembly failed: {0}")]
    Witness(String),
    /// Myelin finality proof did not verify.
    #[error("Myelin consensus proof failed: {0}")]
    Consensus(myelin_consensus::ConsensusError),
    /// CKB adapter receipt chain did not verify.
    #[error("CKB evidence failed: {0}")]
    Ckb(myelin_ckb_adapter::CkbAdapterError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use myelin_exec::Script;
    use myelin_state::CellEntry;
    use secp256k1::{Keypair, SecretKey};

    fn participant(id: u8, key_seed: u8, deposit: u128, max_debit: u128) -> (EscrowParticipant, SecretKey) {
        let secret = SecretKey::from_slice(&[key_seed; 32]).unwrap();
        let keypair = Keypair::from_secret_key(&Secp256k1::new(), &secret);
        (
            EscrowParticipant {
                participant_id: [id; 32],
                authorization_role: vec![id],
                authorization_key: XOnlyPublicKey::from_keypair(&keypair).0.serialize(),
                payout_lock: Script::new([id; 32], 1, vec![id]),
                deposit,
                max_debit,
            },
            secret,
        )
    }

    fn terms() -> (EscrowTerms, Vec<SecretKey>) {
        let (alice, alice_key) = participant(1, 3, 10_000_000_000, 6_000_000_000);
        let (bob, bob_key) = participant(2, 4, 10_000_000_000, 6_000_000_000);
        (
            EscrowTerms {
                session_id: [9; 32],
                participants: vec![alice, bob],
                deposits: vec![
                    EscrowDeposit { participant_id: [1; 32], outpoint: OutPoint::new([7; 32], 0) },
                    EscrowDeposit { participant_id: [2; 32], outpoint: OutPoint::new([7; 32], 1) },
                ],
                asset: AssetDescriptor::CkbCapacity,
                session_start_deadline_ms: 10,
                session_expiry_ms: 100,
                challenge_window_ms: 10,
                min_confirmations: 6,
                escrow_lock_script_hash: [8; 32],
                ledger_type_script: Script::new([6; 32], 1, vec![9]),
            },
            vec![alice_key, bob_key],
        )
    }

    #[test]
    fn terms_authorizations_bind_every_field() {
        let (terms, keys) = terms();
        let hash = terms.hash().unwrap();
        let auth = terms
            .participants
            .iter()
            .zip(keys)
            .map(|(participant, secret)| {
                let keypair = Keypair::from_secret_key(&Secp256k1::new(), &secret);
                let signature = Secp256k1::new()
                    .sign_schnorr_no_aux_rand(&Message::from_digest(authorization_digest(hash, participant.participant_id)), &keypair)
                    .serialize()
                    .to_vec();
                EscrowAuthorization { participant_id: participant.participant_id, signature }
            })
            .collect::<Vec<_>>();
        terms.verify_authorizations(&auth).unwrap();
        let mut changed = terms;
        changed.participants[0].max_debit -= 1;
        assert!(changed.verify_authorizations(&auth).is_err());
    }

    #[test]
    fn ledger_successor_enforces_conservation_caps_and_linkage() {
        let (terms, _) = terms();
        let initial = EscrowState::genesis(&terms).unwrap();
        let mut next = initial.clone();
        next.epoch = 1;
        next.previous_state_hash = initial.hash().unwrap();
        next.application_evidence_root = [5; 32];
        next.ledger[0].balance = 8_000_000_000;
        next.ledger[1].balance = 12_000_000_000;
        initial.validate_successor(&next, &terms).unwrap();

        let bytes = next.encode().unwrap();
        assert_eq!(EscrowState::decode(&bytes).unwrap(), next);
        let mut excessive = next;
        excessive.ledger[0].balance = 3_000_000_000;
        excessive.ledger[1].balance = 17_000_000_000;
        assert!(initial.validate_successor(&excessive, &terms).is_err());

        let mut dust = initial.clone();
        dust.epoch = 1;
        dust.previous_state_hash = initial.hash().unwrap();
        dust.application_evidence_root = [5; 32];
        dust.ledger[0].balance = 4_100_000_000;
        dust.ledger[1].balance = 15_900_000_000;
        assert!(initial.validate_successor(&dust, &terms).is_err());
    }

    #[test]
    fn verifier_rejects_nonconserving_ledger_cell_transition() {
        let (terms, _) = terms();
        let initial = EscrowState::genesis(&terms).unwrap();
        let mut next = initial.clone();
        next.epoch = 1;
        next.previous_state_hash = initial.hash().unwrap();
        next.application_evidence_root = [5; 32];
        next.ledger[0].balance = 9_000_000_000;
        next.ledger[1].balance = 11_000_000_000;

        let lock = Script::new([4; 32], 1, vec![1]);
        let input_output = CellOutput { capacity: 10_000_000_000, lock: lock.clone(), type_: Some(terms.ledger_type_script.clone()) };
        let input_data = initial.encode().unwrap();
        let input = OutPoint::new([3; 32], 0);
        let resolved =
            vec![ResolvedStateInput { outpoint: input, cell: CellEntry::from_output(&input_output, &input_data, 0, false).unwrap() }];
        let transaction =
            CellTx::new(vec![CellInput::new(input, 0)], vec![], vec![input_output.clone()], vec![next.encode().unwrap()], vec![])
                .unwrap();
        let verifier = EscrowCellVerifier::new(terms.clone(), Arc::new(|_: &CellTx, _: &[ResolvedStateInput]| Ok(42))).unwrap();
        assert_eq!(verifier.verify(&transaction, &resolved).unwrap(), 42);

        next.ledger[0].balance = 8_900_000_000;
        let invalid =
            CellTx::new(vec![CellInput::new(input, 0)], vec![], vec![input_output], vec![next.encode().unwrap()], vec![]).unwrap();
        assert!(verifier.verify(&invalid, &resolved).is_err());
    }
}
