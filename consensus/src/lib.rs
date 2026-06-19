// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Selectable finality engines for finite off-chain Cell sessions.

//! Myelin consensus selection.
//!
//! The first concrete engine is a static closed committee. It is intended for
//! session benchmarking, fixture generation, and the phase-one fast path. It is
//! not a permissionless consensus protocol.

use serde::Deserialize;
use std::collections::{HashMap, HashSet};

/// Fixed-width hash used by consensus artefacts.
pub type Hash32 = [u8; 32];

/// Fixed-width phase-one committee signature.
pub type Signature64 = [u8; 64];

const BLOCK_HASH_DOMAIN: &[u8] = b"myelin:block:v1";
const STATIC_SIGNATURE_DOMAIN: &[u8] = b"myelin:static-committee-signature:v1";
const TENDERMINT_PRECOMMIT_DOMAIN: &[u8] = b"myelin:tendermint-precommit:v1";

/// Consensus engine selected for a Myelin session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsensusKind {
    /// Configured validators finalise blocks once the quorum weight is reached.
    StaticClosedCommittee,
    /// Tendermint-style weighted precommit finality for finite sessions.
    Tendermint,
}

impl ConsensusKind {
    /// Stable config string for this consensus kind.
    pub const fn as_str(self) -> &'static str {
        match self {
            ConsensusKind::StaticClosedCommittee => "static-closed-committee",
            ConsensusKind::Tendermint => "tendermint",
        }
    }
}

/// Consensus configuration after parsing and validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsensusConfig {
    /// Selected consensus engine.
    pub kind: ConsensusKind,
    /// Static committee configuration when `kind` is `StaticClosedCommittee`.
    pub static_committee: Option<StaticCommitteeConfig>,
    /// Tendermint configuration when `kind` is `Tendermint`.
    pub tendermint: Option<TendermintConfig>,
}

impl ConsensusConfig {
    /// Build a static closed-committee config directly.
    pub fn static_closed_committee(static_committee: StaticCommitteeConfig) -> Self {
        Self { kind: ConsensusKind::StaticClosedCommittee, static_committee: Some(static_committee), tendermint: None }
    }

    /// Build a Tendermint config directly.
    pub fn tendermint(tendermint: TendermintConfig) -> Self {
        Self { kind: ConsensusKind::Tendermint, static_committee: None, tendermint: Some(tendermint) }
    }

    /// Parse a TOML consensus config.
    pub fn from_toml_str(input: &str) -> Result<Self> {
        let raw: RawConsensusConfig = toml::from_str(input).map_err(|err| ConsensusError::InvalidConfig(err.to_string()))?;
        let kind = parse_consensus_kind(&raw.kind)?;
        match kind {
            ConsensusKind::StaticClosedCommittee => {
                let raw_committee = raw
                    .static_committee
                    .ok_or_else(|| ConsensusError::InvalidConfig("static-closed-committee requires [static_committee]".to_owned()))?;
                Ok(Self::static_closed_committee(raw_committee.try_into()?))
            }
            ConsensusKind::Tendermint => {
                let raw_tendermint =
                    raw.tendermint.ok_or_else(|| ConsensusError::InvalidConfig("tendermint requires [tendermint]".to_owned()))?;
                Ok(Self::tendermint(raw_tendermint.try_into()?))
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawConsensusConfig {
    kind: String,
    static_committee: Option<RawStaticCommitteeConfig>,
    tendermint: Option<RawTendermintConfig>,
}

#[derive(Debug, Deserialize)]
struct RawStaticCommitteeConfig {
    quorum_weight: u64,
    validators: Vec<RawCommitteeValidator>,
}

#[derive(Debug, Deserialize)]
struct RawCommitteeValidator {
    id: String,
    public_key: String,
    weight: u64,
}

#[derive(Debug, Deserialize)]
struct RawTendermintConfig {
    quorum_power: u64,
    validators: Vec<RawCommitteeValidator>,
}

impl TryFrom<RawStaticCommitteeConfig> for StaticCommitteeConfig {
    type Error = ConsensusError;

    fn try_from(raw: RawStaticCommitteeConfig) -> Result<Self> {
        let validators = raw
            .validators
            .into_iter()
            .map(|validator| {
                Ok(CommitteeValidator { id: validator.id, public_key: parse_hex_32(&validator.public_key)?, weight: validator.weight })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { validators, quorum_weight: raw.quorum_weight })
    }
}

impl TryFrom<RawTendermintConfig> for TendermintConfig {
    type Error = ConsensusError;

    fn try_from(raw: RawTendermintConfig) -> Result<Self> {
        let validators = raw
            .validators
            .into_iter()
            .map(|validator| {
                Ok(CommitteeValidator { id: validator.id, public_key: parse_hex_32(&validator.public_key)?, weight: validator.weight })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { validators, quorum_power: raw.quorum_power })
    }
}

fn parse_consensus_kind(kind: &str) -> Result<ConsensusKind> {
    match kind {
        "static-closed-committee" | "static_closed_committee" => Ok(ConsensusKind::StaticClosedCommittee),
        "tendermint" => Ok(ConsensusKind::Tendermint),
        other => Err(ConsensusError::UnknownEngine(other.to_owned())),
    }
}

fn parse_hex_32(value: &str) -> Result<Hash32> {
    let stripped = value.strip_prefix("0x").unwrap_or(value);
    let decoded = hex::decode(stripped).map_err(|err| ConsensusError::InvalidConfig(format!("invalid hex key: {err}")))?;
    decoded.try_into().map_err(|_| ConsensusError::InvalidConfig("public key must be 32 bytes".to_owned()))
}

/// A finite-session block finalised by a selected Myelin consensus engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MyelinBlock {
    /// Block format version.
    pub version: u32,
    /// Parent session block hash, or zero for a session genesis block.
    pub parent_hash: Hash32,
    /// Session-local block number.
    pub number: u64,
    /// Millisecond timestamp supplied by the session runtime.
    pub timestamp_ms: u64,
    /// Consensus engine expected to finalise this block.
    pub consensus_kind: ConsensusKind,
    /// Cell state root before executing the ordered transition set.
    pub state_root_before: Hash32,
    /// Cell state root after executing the ordered transition set.
    pub state_root_after: Hash32,
    /// Ordered CellTx commitments admitted into this block.
    pub ordered_cell_tx_commitments: Vec<Hash32>,
    /// Published data-availability chunk commitments.
    pub data_commitments: Vec<Hash32>,
    /// Commitment to the CellDAG scheduler report.
    pub scheduler_commitment: Hash32,
}

impl MyelinBlock {
    /// Return the canonical Molecule-shaped byte representation used for hashing.
    pub fn to_molecule_bytes(&self) -> Vec<u8> {
        encode_table(&[
            self.version.to_le_bytes().to_vec(),
            self.parent_hash.to_vec(),
            self.number.to_le_bytes().to_vec(),
            self.timestamp_ms.to_le_bytes().to_vec(),
            self.consensus_kind.as_str().as_bytes().to_vec(),
            self.state_root_before.to_vec(),
            self.state_root_after.to_vec(),
            encode_hash_vec(&self.ordered_cell_tx_commitments),
            encode_hash_vec(&self.data_commitments),
            self.scheduler_commitment.to_vec(),
        ])
    }

    /// Hash the canonical block representation.
    pub fn hash(&self) -> Hash32 {
        let mut hasher = blake3::Hasher::new();
        hasher.update(BLOCK_HASH_DOMAIN);
        hasher.update(&self.to_molecule_bytes());
        *hasher.finalize().as_bytes()
    }
}

fn encode_hash_vec(values: &[Hash32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + values.len() * 32);
    out.extend_from_slice(&(values.len() as u32).to_le_bytes());
    for value in values {
        out.extend_from_slice(value);
    }
    out
}

fn encode_table(fields: &[Vec<u8>]) -> Vec<u8> {
    let header_size = 4 + fields.len() * 4;
    let total_size = header_size + fields.iter().map(Vec::len).sum::<usize>();
    assert!(u32::try_from(total_size).is_ok(), "Molecule table is too large");

    let mut out = Vec::with_capacity(total_size);
    out.extend_from_slice(&(total_size as u32).to_le_bytes());

    let mut offset = header_size as u32;
    for field in fields {
        out.extend_from_slice(&offset.to_le_bytes());
        offset = offset.checked_add(field.len() as u32).expect("Molecule table offset overflow");
    }
    for field in fields {
        out.extend_from_slice(field);
    }
    out
}

/// Configured validator in a static closed committee.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitteeValidator {
    /// Stable validator identifier used by certificates.
    pub id: String,
    /// Phase-one validator public key.
    pub public_key: Hash32,
    /// Voting weight counted towards quorum.
    pub weight: u64,
}

/// Static closed-committee configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticCommitteeConfig {
    /// Committee validators.
    pub validators: Vec<CommitteeValidator>,
    /// Weight required to finalise a block.
    pub quorum_weight: u64,
}

/// Tendermint-style validator set for finite Myelin sessions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TendermintConfig {
    /// Tendermint validators.
    pub validators: Vec<CommitteeValidator>,
    /// Voting power required for a block precommit certificate.
    pub quorum_power: u64,
}

/// One validator's block signature in a committee certificate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitteeSignature {
    /// Validator identifier.
    pub validator_id: String,
    /// Signature bytes.
    pub signature: Signature64,
}

/// Certificate attached to a finalised block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitteeCertificate {
    /// Hash being certified.
    pub block_hash: Hash32,
    /// Validator signatures.
    pub signatures: Vec<CommitteeSignature>,
}

/// Tendermint precommit certificate for one block height and round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TendermintPrecommitCertificate {
    /// Hash being precommitted.
    pub block_hash: Hash32,
    /// Session-local block height being precommitted.
    pub height: u64,
    /// Tendermint round being precommitted.
    pub round: u32,
    /// Validator precommit signatures.
    pub signatures: Vec<CommitteeSignature>,
}

/// Finalised block artefact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalisedBlock {
    /// Finalised block.
    pub block: MyelinBlock,
    /// Block hash verified by consensus.
    pub block_hash: Hash32,
    /// Certificate that reached quorum.
    pub certificate: CommitteeCertificate,
}

/// Common interface for selectable Myelin consensus engines.
pub trait ConsensusEngine {
    /// Engine kind.
    fn kind(&self) -> ConsensusKind;

    /// Verify a certificate for a block hash.
    fn verify_certificate(&self, block_hash: Hash32, certificate: &CommitteeCertificate) -> Result<()>;

    /// Verify and finalise a block.
    fn finalise_block(&self, block: MyelinBlock, certificate: CommitteeCertificate) -> Result<FinalisedBlock> {
        if block.consensus_kind != self.kind() {
            return Err(ConsensusError::WrongEngine { expected: self.kind().as_str(), actual: block.consensus_kind.as_str() });
        }
        let block_hash = block.hash();
        self.verify_certificate(block_hash, &certificate)?;
        Ok(FinalisedBlock { block, block_hash, certificate })
    }
}

/// A selected consensus engine.
#[derive(Debug, Clone)]
pub enum SelectedConsensus {
    /// Static closed committee.
    StaticClosedCommittee(StaticClosedCommittee),
    /// Tendermint-style precommit finality.
    Tendermint(Tendermint),
}

impl SelectedConsensus {
    /// Build a selected engine from config.
    pub fn from_config(config: ConsensusConfig) -> Result<Self> {
        match config.kind {
            ConsensusKind::StaticClosedCommittee => {
                let committee = config
                    .static_committee
                    .ok_or_else(|| ConsensusError::InvalidConfig("missing static committee config".to_owned()))?;
                Ok(Self::StaticClosedCommittee(StaticClosedCommittee::new(committee)?))
            }
            ConsensusKind::Tendermint => {
                let tendermint =
                    config.tendermint.ok_or_else(|| ConsensusError::InvalidConfig("missing tendermint config".to_owned()))?;
                Ok(Self::Tendermint(Tendermint::new(tendermint)?))
            }
        }
    }
}

impl ConsensusEngine for SelectedConsensus {
    fn kind(&self) -> ConsensusKind {
        match self {
            SelectedConsensus::StaticClosedCommittee(engine) => engine.kind(),
            SelectedConsensus::Tendermint(engine) => engine.kind(),
        }
    }

    fn verify_certificate(&self, block_hash: Hash32, certificate: &CommitteeCertificate) -> Result<()> {
        match self {
            SelectedConsensus::StaticClosedCommittee(engine) => engine.verify_certificate(block_hash, certificate),
            SelectedConsensus::Tendermint(engine) => engine.verify_certificate(block_hash, certificate),
        }
    }
}

/// Static closed-committee finality engine.
#[derive(Debug, Clone)]
pub struct StaticClosedCommittee {
    validators: HashMap<String, CommitteeValidator>,
    quorum_weight: u64,
}

impl StaticClosedCommittee {
    /// Validate and build a static closed committee.
    pub fn new(config: StaticCommitteeConfig) -> Result<Self> {
        if config.quorum_weight == 0 {
            return Err(ConsensusError::InvalidConfig("quorum_weight must be non-zero".to_owned()));
        }

        let mut validators = HashMap::with_capacity(config.validators.len());
        let mut total_weight = 0u64;
        for validator in config.validators {
            if validator.id.is_empty() {
                return Err(ConsensusError::InvalidConfig("validator id must not be empty".to_owned()));
            }
            if validator.weight == 0 {
                return Err(ConsensusError::ZeroWeight(validator.id));
            }
            total_weight = total_weight
                .checked_add(validator.weight)
                .ok_or_else(|| ConsensusError::InvalidConfig("committee weight overflow".to_owned()))?;
            if validators.insert(validator.id.clone(), validator).is_some() {
                return Err(ConsensusError::DuplicateValidator);
            }
        }

        if validators.is_empty() {
            return Err(ConsensusError::InvalidConfig("committee must contain at least one validator".to_owned()));
        }
        if config.quorum_weight > total_weight {
            return Err(ConsensusError::InvalidConfig("quorum_weight exceeds total committee weight".to_owned()));
        }

        Ok(Self { validators, quorum_weight: config.quorum_weight })
    }

    /// Create a deterministic phase-one signature for fixtures and local runs.
    ///
    /// This is deliberately a closed-committee development signature, not a
    /// permissionless cryptographic signature scheme.
    pub fn sign_fixture(&self, validator_id: &str, block_hash: Hash32) -> Result<CommitteeSignature> {
        let validator = self.validators.get(validator_id).ok_or_else(|| ConsensusError::UnknownValidator(validator_id.to_owned()))?;
        Ok(CommitteeSignature { validator_id: validator_id.to_owned(), signature: deterministic_signature(validator, block_hash) })
    }

    /// Build a quorum certificate from validator ids.
    pub fn certificate_for_fixture(&self, block_hash: Hash32, validator_ids: &[&str]) -> Result<CommitteeCertificate> {
        let signatures =
            validator_ids.iter().map(|validator_id| self.sign_fixture(validator_id, block_hash)).collect::<Result<Vec<_>>>()?;
        Ok(CommitteeCertificate { block_hash, signatures })
    }
}

impl ConsensusEngine for StaticClosedCommittee {
    fn kind(&self) -> ConsensusKind {
        ConsensusKind::StaticClosedCommittee
    }

    fn verify_certificate(&self, block_hash: Hash32, certificate: &CommitteeCertificate) -> Result<()> {
        if certificate.block_hash != block_hash {
            return Err(ConsensusError::WrongBlockHash);
        }

        let mut seen = HashSet::with_capacity(certificate.signatures.len());
        let mut signed_weight = 0u64;
        for signature in &certificate.signatures {
            if !seen.insert(signature.validator_id.as_str()) {
                return Err(ConsensusError::DuplicateValidator);
            }
            let validator = self
                .validators
                .get(&signature.validator_id)
                .ok_or_else(|| ConsensusError::UnknownValidator(signature.validator_id.clone()))?;
            let expected = deterministic_signature(validator, block_hash);
            if signature.signature != expected {
                return Err(ConsensusError::InvalidSignature(signature.validator_id.clone()));
            }
            signed_weight = signed_weight
                .checked_add(validator.weight)
                .ok_or_else(|| ConsensusError::InvalidConfig("certificate weight overflow".to_owned()))?;
        }

        if signed_weight < self.quorum_weight {
            return Err(ConsensusError::QuorumNotMet { signed_weight, quorum_weight: self.quorum_weight });
        }

        Ok(())
    }
}

fn deterministic_signature(validator: &CommitteeValidator, block_hash: Hash32) -> Signature64 {
    let mut first = blake3::Hasher::new();
    first.update(STATIC_SIGNATURE_DOMAIN);
    first.update(validator.id.as_bytes());
    first.update(&validator.public_key);
    first.update(&block_hash);

    let mut second = blake3::Hasher::new();
    second.update(STATIC_SIGNATURE_DOMAIN);
    second.update(b":tail");
    second.update(validator.id.as_bytes());
    second.update(&validator.public_key);
    second.update(&block_hash);

    let mut signature = [0u8; 64];
    signature[..32].copy_from_slice(first.finalize().as_bytes());
    signature[32..].copy_from_slice(second.finalize().as_bytes());
    signature
}

/// Tendermint-style weighted precommit finality engine.
#[derive(Debug, Clone)]
pub struct Tendermint {
    validators: HashMap<String, CommitteeValidator>,
    quorum_power: u64,
}

impl Tendermint {
    /// Validate and build a Tendermint validator set.
    pub fn new(config: TendermintConfig) -> Result<Self> {
        if config.quorum_power == 0 {
            return Err(ConsensusError::InvalidConfig("quorum_power must be non-zero".to_owned()));
        }

        let mut validators = HashMap::with_capacity(config.validators.len());
        let mut total_power = 0u64;
        for validator in config.validators {
            if validator.id.is_empty() {
                return Err(ConsensusError::InvalidConfig("validator id must not be empty".to_owned()));
            }
            if validator.weight == 0 {
                return Err(ConsensusError::ZeroWeight(validator.id));
            }
            total_power = total_power
                .checked_add(validator.weight)
                .ok_or_else(|| ConsensusError::InvalidConfig("validator power overflow".to_owned()))?;
            if validators.insert(validator.id.clone(), validator).is_some() {
                return Err(ConsensusError::DuplicateValidator);
            }
        }

        if validators.is_empty() {
            return Err(ConsensusError::InvalidConfig("tendermint validator set must contain at least one validator".to_owned()));
        }
        if config.quorum_power > total_power {
            return Err(ConsensusError::InvalidConfig("quorum_power exceeds total validator power".to_owned()));
        }

        Ok(Self { validators, quorum_power: config.quorum_power })
    }

    /// Create a deterministic Tendermint precommit for fixtures and local runs.
    pub fn sign_precommit_fixture(
        &self,
        validator_id: &str,
        block_hash: Hash32,
        height: u64,
        round: u32,
    ) -> Result<CommitteeSignature> {
        let validator = self.validators.get(validator_id).ok_or_else(|| ConsensusError::UnknownValidator(validator_id.to_owned()))?;
        Ok(CommitteeSignature {
            validator_id: validator_id.to_owned(),
            signature: deterministic_tendermint_precommit(validator, block_hash, height, round),
        })
    }

    /// Build a Tendermint precommit certificate from validator ids.
    pub fn precommit_certificate_for_fixture(
        &self,
        block_hash: Hash32,
        height: u64,
        round: u32,
        validator_ids: &[&str],
    ) -> Result<TendermintPrecommitCertificate> {
        let signatures = validator_ids
            .iter()
            .map(|validator_id| self.sign_precommit_fixture(validator_id, block_hash, height, round))
            .collect::<Result<Vec<_>>>()?;
        Ok(TendermintPrecommitCertificate { block_hash, height, round, signatures })
    }

    /// Verify a Tendermint precommit certificate for a block at height and round.
    pub fn verify_precommit_certificate(
        &self,
        block_hash: Hash32,
        height: u64,
        round: u32,
        certificate: &TendermintPrecommitCertificate,
    ) -> Result<()> {
        if certificate.block_hash != block_hash {
            return Err(ConsensusError::WrongBlockHash);
        }
        if certificate.height != height {
            return Err(ConsensusError::WrongHeight { expected: height, actual: certificate.height });
        }
        if certificate.round != round {
            return Err(ConsensusError::WrongRound { expected: round, actual: certificate.round });
        }

        let mut seen = HashSet::with_capacity(certificate.signatures.len());
        let mut signed_power = 0u64;
        for signature in &certificate.signatures {
            if !seen.insert(signature.validator_id.as_str()) {
                return Err(ConsensusError::DuplicateValidator);
            }
            let validator = self
                .validators
                .get(&signature.validator_id)
                .ok_or_else(|| ConsensusError::UnknownValidator(signature.validator_id.clone()))?;
            let expected = deterministic_tendermint_precommit(validator, block_hash, height, round);
            if signature.signature != expected {
                return Err(ConsensusError::InvalidSignature(signature.validator_id.clone()));
            }
            signed_power = signed_power
                .checked_add(validator.weight)
                .ok_or_else(|| ConsensusError::InvalidConfig("precommit power overflow".to_owned()))?;
        }

        if signed_power < self.quorum_power {
            return Err(ConsensusError::QuorumNotMet { signed_weight: signed_power, quorum_weight: self.quorum_power });
        }

        Ok(())
    }

    /// Verify and finalise a block with a Tendermint precommit certificate.
    pub fn finalise_block_with_precommit(
        &self,
        block: MyelinBlock,
        round: u32,
        certificate: TendermintPrecommitCertificate,
    ) -> Result<FinalisedTendermintBlock> {
        if block.consensus_kind != ConsensusKind::Tendermint {
            return Err(ConsensusError::WrongEngine {
                expected: ConsensusKind::Tendermint.as_str(),
                actual: block.consensus_kind.as_str(),
            });
        }
        let block_hash = block.hash();
        self.verify_precommit_certificate(block_hash, block.number, round, &certificate)?;
        Ok(FinalisedTendermintBlock { block, block_hash, round, certificate })
    }
}

/// Finalised Tendermint block artefact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalisedTendermintBlock {
    /// Finalised block.
    pub block: MyelinBlock,
    /// Block hash verified by Tendermint precommits.
    pub block_hash: Hash32,
    /// Tendermint round that reached quorum.
    pub round: u32,
    /// Precommit certificate that reached quorum.
    pub certificate: TendermintPrecommitCertificate,
}

impl ConsensusEngine for Tendermint {
    fn kind(&self) -> ConsensusKind {
        ConsensusKind::Tendermint
    }

    fn verify_certificate(&self, _block_hash: Hash32, _certificate: &CommitteeCertificate) -> Result<()> {
        // Tendermint finality is always height-bound, round-bound, and
        // block-hash-bound. The legacy CommitteeCertificate API is not
        // a structurally valid Tendermint certificate: it carries no
        // (height, round). Reject it explicitly so callers cannot
        // accidentally use the wrong API shape.
        Err(ConsensusError::LegacyCertificatePathUnsupported)
    }
}

fn deterministic_tendermint_precommit(validator: &CommitteeValidator, block_hash: Hash32, height: u64, round: u32) -> Signature64 {
    let mut first = blake3::Hasher::new();
    first.update(TENDERMINT_PRECOMMIT_DOMAIN);
    first.update(&height.to_le_bytes());
    first.update(&round.to_le_bytes());
    first.update(validator.id.as_bytes());
    first.update(&validator.public_key);
    first.update(&block_hash);

    let mut second = blake3::Hasher::new();
    second.update(TENDERMINT_PRECOMMIT_DOMAIN);
    second.update(b":tail");
    second.update(&height.to_le_bytes());
    second.update(&round.to_le_bytes());
    second.update(validator.id.as_bytes());
    second.update(&validator.public_key);
    second.update(&block_hash);

    let mut signature = [0u8; 64];
    signature[..32].copy_from_slice(first.finalize().as_bytes());
    signature[32..].copy_from_slice(second.finalize().as_bytes());
    signature
}

/// Consensus errors.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConsensusError {
    /// Config is malformed.
    #[error("invalid consensus config: {0}")]
    InvalidConfig(String),
    /// The selected engine name is unknown.
    #[error("unknown consensus engine: {0}")]
    UnknownEngine(String),
    /// The block was built for a different engine.
    #[error("wrong consensus engine: expected {expected}, got {actual}")]
    WrongEngine {
        /// Expected engine.
        expected: &'static str,
        /// Actual engine.
        actual: &'static str,
    },
    /// Certificate points at another block.
    #[error("certificate block hash does not match")]
    WrongBlockHash,
    /// Tendermint precommit certificate is for another height.
    #[error("wrong tendermint height: expected {expected}, got {actual}")]
    WrongHeight {
        /// Expected block height.
        expected: u64,
        /// Certificate height.
        actual: u64,
    },
    /// Tendermint precommit certificate is for another round.
    #[error("wrong tendermint round: expected {expected}, got {actual}")]
    WrongRound {
        /// Expected round.
        expected: u32,
        /// Certificate round.
        actual: u32,
    },
    /// Validator id is duplicated.
    #[error("duplicate validator")]
    DuplicateValidator,
    /// Validator is not part of the configured committee.
    #[error("unknown validator: {0}")]
    UnknownValidator(String),
    /// Validator weight is invalid.
    #[error("validator has zero weight: {0}")]
    ZeroWeight(String),
    /// Signature bytes do not match the phase-one fixture signature.
    #[error("invalid signature for validator: {0}")]
    InvalidSignature(String),
    /// Certificate has insufficient voting weight.
    #[error("quorum not met: signed {signed_weight}, required {quorum_weight}")]
    QuorumNotMet {
        /// Weight present in certificate.
        signed_weight: u64,
        /// Weight required for finality.
        quorum_weight: u64,
    },
    /// Tendermint does not implement the legacy generic CommitteeCertificate
    /// path. Use `verify_precommit_certificate` with a typed
    /// `TendermintPrecommitCertificate` instead.
    #[error("tendermint does not implement verify_certificate; use verify_precommit_certificate with a typed TendermintPrecommitCertificate")]
    LegacyCertificatePathUnsupported,
}

/// Consensus result type.
pub type Result<T> = std::result::Result<T, ConsensusError>;

#[cfg(test)]
mod tests {
    use super::*;

    fn validator(id: &str, seed: u8, weight: u64) -> CommitteeValidator {
        CommitteeValidator { id: id.to_owned(), public_key: [seed; 32], weight }
    }

    fn committee() -> StaticClosedCommittee {
        StaticClosedCommittee::new(StaticCommitteeConfig {
            validators: vec![validator("alice", 1, 1), validator("bob", 2, 1), validator("carol", 3, 1)],
            quorum_weight: 2,
        })
        .unwrap()
    }

    fn block() -> MyelinBlock {
        block_for(ConsensusKind::StaticClosedCommittee)
    }

    fn block_for(consensus_kind: ConsensusKind) -> MyelinBlock {
        MyelinBlock {
            version: 1,
            parent_hash: [0; 32],
            number: 7,
            timestamp_ms: 1_780_000_000_000,
            consensus_kind,
            state_root_before: [4; 32],
            state_root_after: [5; 32],
            ordered_cell_tx_commitments: vec![[6; 32], [7; 32]],
            data_commitments: vec![[8; 32]],
            scheduler_commitment: [9; 32],
        }
    }

    #[test]
    fn static_committee_finalises_with_quorum() {
        let engine = committee();
        let block = block();
        let cert = engine.certificate_for_fixture(block.hash(), &["alice", "bob"]).unwrap();
        let finalised = engine.finalise_block(block.clone(), cert).unwrap();
        assert_eq!(finalised.block, block);
        assert_eq!(finalised.block_hash, block.hash());
    }

    #[test]
    fn static_committee_rejects_below_quorum() {
        let engine = committee();
        let block_hash = block().hash();
        let cert = engine.certificate_for_fixture(block_hash, &["alice"]).unwrap();
        assert_eq!(
            engine.verify_certificate(block_hash, &cert),
            Err(ConsensusError::QuorumNotMet { signed_weight: 1, quorum_weight: 2 })
        );
    }

    #[test]
    fn selected_consensus_loads_from_toml() {
        let toml = r#"
kind = "static-closed-committee"

[static_committee]
quorum_weight = 2

[[static_committee.validators]]
id = "alice"
public_key = "0101010101010101010101010101010101010101010101010101010101010101"
weight = 1

[[static_committee.validators]]
id = "bob"
public_key = "0202020202020202020202020202020202020202020202020202020202020202"
weight = 1
"#;
        let selected = SelectedConsensus::from_config(ConsensusConfig::from_toml_str(toml).unwrap()).unwrap();
        assert_eq!(selected.kind(), ConsensusKind::StaticClosedCommittee);
    }

    fn tendermint() -> Tendermint {
        Tendermint::new(TendermintConfig {
            validators: vec![validator("alice", 1, 1), validator("bob", 2, 1), validator("carol", 3, 1)],
            quorum_power: 2,
        })
        .unwrap()
    }

    #[test]
    fn tendermint_finalises_with_precommit_quorum() {
        let engine = tendermint();
        let block = block_for(ConsensusKind::Tendermint);
        let cert = engine.precommit_certificate_for_fixture(block.hash(), block.number, 0, &["alice", "bob"]).unwrap();
        let finalised = engine.finalise_block_with_precommit(block.clone(), 0, cert).unwrap();
        assert_eq!(finalised.block, block);
        assert_eq!(finalised.block_hash, block.hash());
        assert_eq!(finalised.round, 0);
    }

    #[test]
    fn tendermint_rejects_below_quorum() {
        let engine = tendermint();
        let block = block_for(ConsensusKind::Tendermint);
        let block_hash = block.hash();
        let cert = engine.precommit_certificate_for_fixture(block_hash, block.number, 0, &["alice"]).unwrap();
        assert_eq!(
            engine.verify_precommit_certificate(block_hash, block.number, 0, &cert),
            Err(ConsensusError::QuorumNotMet { signed_weight: 1, quorum_weight: 2 })
        );
    }

    #[test]
    fn tendermint_rejects_wrong_height_and_round() {
        let engine = tendermint();
        let block = block_for(ConsensusKind::Tendermint);
        let block_hash = block.hash();
        let cert = engine.precommit_certificate_for_fixture(block_hash, block.number, 0, &["alice", "bob"]).unwrap();
        assert_eq!(
            engine.verify_precommit_certificate(block_hash, block.number + 1, 0, &cert),
            Err(ConsensusError::WrongHeight { expected: block.number + 1, actual: block.number })
        );
        assert_eq!(
            engine.verify_precommit_certificate(block_hash, block.number, 1, &cert),
            Err(ConsensusError::WrongRound { expected: 1, actual: 0 })
        );
    }

    #[test]
    fn selected_tendermint_loads_from_toml() {
        let toml = r#"
kind = "tendermint"

[tendermint]
quorum_power = 2

[[tendermint.validators]]
id = "alice"
public_key = "0101010101010101010101010101010101010101010101010101010101010101"
weight = 1

[[tendermint.validators]]
id = "bob"
public_key = "0202020202020202020202020202020202020202020202020202020202020202"
weight = 1
"#;
        let selected = SelectedConsensus::from_config(ConsensusConfig::from_toml_str(toml).unwrap()).unwrap();
        assert_eq!(selected.kind(), ConsensusKind::Tendermint);
    }

    // ─── Additional StaticClosedCommittee tests ──────────────────────────────────

    #[test]
    fn static_committee_rejects_duplicate_validator() {
        let engine = committee();
        let block_hash = block().hash();
        let mut cert = engine.certificate_for_fixture(block_hash, &["alice", "bob"]).unwrap();
        // Insert a duplicate alice signature so the validator appears twice
        cert.signatures.push(cert.signatures[0].clone());
        assert_eq!(engine.verify_certificate(block_hash, &cert), Err(ConsensusError::DuplicateValidator));
    }

    #[test]
    fn static_committee_rejects_unknown_validator() {
        let engine = committee();
        let block_hash = block().hash();
        let mut cert = engine.certificate_for_fixture(block_hash, &["alice"]).unwrap();
        cert.signatures.push(CommitteeSignature { validator_id: "ghost".to_owned(), signature: [0xAA; 64] });
        assert_eq!(engine.verify_certificate(block_hash, &cert), Err(ConsensusError::UnknownValidator("ghost".to_owned())));
    }

    #[test]
    fn static_committee_rejects_wrong_block_hash() {
        let engine = committee();
        let block = block();
        let cert = engine.certificate_for_fixture(block.hash(), &["alice", "bob"]).unwrap();
        let wrong_hash = [0xFF; 32];
        assert_eq!(engine.verify_certificate(wrong_hash, &cert), Err(ConsensusError::WrongBlockHash));
    }

    #[test]
    fn static_committee_rejects_invalid_signature() {
        let engine = committee();
        let block = block();
        let block_hash = block.hash();
        let mut cert = engine.certificate_for_fixture(block_hash, &["alice", "bob"]).unwrap();
        // Replace alice's signature with garbage
        cert.signatures[0].signature = [0xDE; 64];
        assert_eq!(engine.verify_certificate(block_hash, &cert), Err(ConsensusError::InvalidSignature("alice".to_owned())));
    }

    #[test]
    fn static_committee_finalised_block_is_stable() {
        let engine = committee();
        let block = block();
        let cert1 = engine.certificate_for_fixture(block.hash(), &["alice", "bob"]).unwrap();
        let cert2 = engine.certificate_for_fixture(block.hash(), &["alice", "bob"]).unwrap();
        let f1 = engine.finalise_block(block.clone(), cert1).unwrap();
        let f2 = engine.finalise_block(block.clone(), cert2).unwrap();
        assert_eq!(f1.block_hash, f2.block_hash);
        assert_eq!(f1.certificate.block_hash, f2.certificate.block_hash);
    }

    // ─── Additional Tendermint tests ──────────────────────────────────────────────

    #[test]
    fn tendermint_rejects_wrong_block_hash() {
        let engine = tendermint();
        let block = block_for(ConsensusKind::Tendermint);
        let block_hash = block.hash();
        let cert = engine.precommit_certificate_for_fixture(block_hash, block.number, 0, &["alice", "bob"]).unwrap();
        let wrong_hash = [0xEE; 32];
        assert_eq!(engine.verify_precommit_certificate(wrong_hash, block.number, 0, &cert), Err(ConsensusError::WrongBlockHash));
    }

    #[test]
    fn tendermint_rejects_duplicate_validator() {
        let engine = tendermint();
        let block = block_for(ConsensusKind::Tendermint);
        let block_hash = block.hash();
        let mut cert = engine.precommit_certificate_for_fixture(block_hash, block.number, 0, &["alice", "bob"]).unwrap();
        cert.signatures.push(cert.signatures[0].clone());
        assert_eq!(engine.verify_precommit_certificate(block_hash, block.number, 0, &cert), Err(ConsensusError::DuplicateValidator));
    }

    #[test]
    fn tendermint_rejects_unknown_validator() {
        let engine = tendermint();
        let block = block_for(ConsensusKind::Tendermint);
        let block_hash = block.hash();
        let mut cert = engine.precommit_certificate_for_fixture(block_hash, block.number, 0, &["alice", "bob"]).unwrap();
        cert.signatures.push(CommitteeSignature { validator_id: "ghost".to_owned(), signature: [0xAA; 64] });
        assert_eq!(
            engine.verify_precommit_certificate(block_hash, block.number, 0, &cert),
            Err(ConsensusError::UnknownValidator("ghost".to_owned()))
        );
    }

    #[test]
    fn tendermint_rejects_invalid_signature() {
        let engine = tendermint();
        let block = block_for(ConsensusKind::Tendermint);
        let block_hash = block.hash();
        let mut cert = engine.precommit_certificate_for_fixture(block_hash, block.number, 0, &["alice", "bob"]).unwrap();
        cert.signatures[0].signature = [0xCD; 64];
        assert_eq!(
            engine.verify_precommit_certificate(block_hash, block.number, 0, &cert),
            Err(ConsensusError::InvalidSignature("alice".to_owned()))
        );
    }

    #[test]
    fn tendermint_rejects_height_round_combination() {
        let engine = tendermint();
        let block = block_for(ConsensusKind::Tendermint);
        let block_hash = block.hash();
        let cert = engine.precommit_certificate_for_fixture(block_hash, block.number, 2, &["alice", "bob"]).unwrap();
        // Verify the same precommit under (height, round) = (number+1, 3) must fail
        assert_eq!(
            engine.verify_precommit_certificate(block_hash, block.number + 1, 3, &cert),
            Err(ConsensusError::WrongHeight { expected: block.number + 1, actual: block.number })
        );
    }

    #[test]
    fn tendermint_does_not_silently_fall_back_to_static_committee() {
        // Tendermint's legacy `verify_certificate` path is explicitly
        // rejected: a CommitteeCertificate carries no (height, round),
        // so it is not a structurally valid Tendermint precommit. A
        // caller who tries to use the legacy path must get an explicit
        // error, never a silent acceptance.
        let tendermint_engine = tendermint();
        let static_engine = committee();

        let block = block_for(ConsensusKind::StaticClosedCommittee);
        let block_hash = block.hash();

        // Build a Tendermint precommit certificate on the same block hash
        // and hand its signatures to the static-committee verifier as a
        // CommitteeCertificate. The signature domains differ, so the
        // static engine must reject as InvalidSignature.
        let precommit = tendermint_engine.precommit_certificate_for_fixture(block_hash, block.number, 0, &["alice", "bob"]).unwrap();
        let cross_cert = CommitteeCertificate { block_hash, signatures: precommit.signatures.clone() };
        assert!(matches!(static_engine.verify_certificate(block_hash, &cross_cert), Err(ConsensusError::InvalidSignature(_))));

        // The Tendermint engine's legacy `verify_certificate` path must
        // explicitly reject the generic CommitteeCertificate API shape.
        let static_cert = static_engine.certificate_for_fixture(block_hash, &["alice", "bob"]).unwrap();
        assert!(matches!(
            tendermint_engine.verify_certificate(block_hash, &static_cert),
            Err(ConsensusError::LegacyCertificatePathUnsupported)
        ));
    }

    #[test]
    fn tendermint_finalised_block_is_stable() {
        let engine = tendermint();
        let block = block_for(ConsensusKind::Tendermint);
        let cert1 = engine.precommit_certificate_for_fixture(block.hash(), block.number, 0, &["alice", "bob"]).unwrap();
        let cert2 = engine.precommit_certificate_for_fixture(block.hash(), block.number, 0, &["alice", "bob"]).unwrap();
        let f1 = engine.finalise_block_with_precommit(block.clone(), 0, cert1).unwrap();
        let f2 = engine.finalise_block_with_precommit(block.clone(), 0, cert2).unwrap();
        assert_eq!(f1.block_hash, f2.block_hash);
        assert_eq!(f1.certificate.block_hash, f2.certificate.block_hash);
        assert_eq!(f1.round, 0);
    }

    #[test]
    fn block_hash_is_stable_across_calls() {
        let b1 = block();
        let b2 = block();
        assert_eq!(b1.hash(), b2.hash());
        // Mutating a field must change the hash
        let mut b3 = block();
        b3.state_root_after = [0xAA; 32];
        assert_ne!(b1.hash(), b3.hash());
    }

    #[test]
    fn selected_consensus_rejects_wrong_engine_on_block() {
        // A static-committee block must not be finalised by the Tendermint
        // SelectedConsensus path. The Tendermint finalise path requires
        // (round, TendermintPrecommitCertificate), so we exercise the
        // wrong-engine guard via the precommit entry point.
        let tendermint_engine = tendermint();
        let static_block = block_for(ConsensusKind::StaticClosedCommittee);
        let cert = tendermint_engine
            .precommit_certificate_for_fixture(static_block.hash(), static_block.number, 0, &["alice", "bob"])
            .unwrap();
        assert!(matches!(
            tendermint_engine.finalise_block_with_precommit(static_block, 0, cert),
            Err(ConsensusError::WrongEngine { .. })
        ));
    }

    #[test]
    fn selected_consensus_static_committee_does_not_accept_tendermint_kind_block() {
        // A Tendermint block (consensus_kind = Tendermint) must not be
        // accepted by the static-committee finalise path either.
        let static_engine = committee();
        let selected = SelectedConsensus::StaticClosedCommittee(static_engine.clone());
        let tendermint_block = block_for(ConsensusKind::Tendermint);
        let cert = static_engine.certificate_for_fixture(tendermint_block.hash(), &["alice", "bob"]).unwrap();
        assert!(matches!(selected.finalise_block(tendermint_block, cert), Err(ConsensusError::WrongEngine { .. })));
    }
}
