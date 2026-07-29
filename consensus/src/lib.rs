// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Selectable finality engines for finite off-chain Cell sessions.

//! Myelin consensus selection.
//!
//! The first concrete engine is a static closed committee. It is intended for
//! session benchmarking, fixture generation, and the phase-one fast path. It is
//! not a permissionless consensus protocol.

use secp256k1::{schnorr::Signature, Keypair, Message, Secp256k1, SecretKey, XOnlyPublicKey};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

/// Fixed-width hash used by consensus artefacts.
pub type Hash32 = [u8; 32];

/// Fixed-width phase-one committee signature.
pub type Signature64 = [u8; 64];

const BLOCK_HASH_DOMAIN: &[u8] = b"myelin:block:v1";
const STATIC_SIGNATURE_DOMAIN: &[u8] = b"myelin:static-committee-signature:v1";
const WEIGHTED_PRECOMMIT_DOMAIN: &[u8] = b"myelin:weighted-precommit:v1";

/// Consensus engine selected for a Myelin session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsensusKind {
    /// Configured validators finalise blocks once the quorum weight is reached.
    StaticClosedCommittee,
    /// Weighted precommit finality for finite sessions.
    WeightedPrecommit,
}

impl ConsensusKind {
    /// Stable config string for this consensus kind.
    pub const fn as_str(self) -> &'static str {
        match self {
            ConsensusKind::StaticClosedCommittee => "static-closed-committee",
            ConsensusKind::WeightedPrecommit => "weighted-precommit",
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
    /// WeightedPrecommit configuration when `kind` is `WeightedPrecommit`.
    pub weighted_precommit: Option<WeightedPrecommitConfig>,
}

impl ConsensusConfig {
    /// Build a static closed-committee config directly.
    pub fn static_closed_committee(static_committee: StaticCommitteeConfig) -> Self {
        Self { kind: ConsensusKind::StaticClosedCommittee, static_committee: Some(static_committee), weighted_precommit: None }
    }

    /// Build a WeightedPrecommit config directly.
    pub fn weighted_precommit(weighted_precommit: WeightedPrecommitConfig) -> Self {
        Self { kind: ConsensusKind::WeightedPrecommit, static_committee: None, weighted_precommit: Some(weighted_precommit) }
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
            ConsensusKind::WeightedPrecommit => {
                let raw_weighted_precommit = raw
                    .weighted_precommit
                    .ok_or_else(|| ConsensusError::InvalidConfig("weighted_precommit requires [weighted_precommit]".to_owned()))?;
                Ok(Self::weighted_precommit(raw_weighted_precommit.try_into()?))
            }
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConsensusConfig {
    kind: String,
    static_committee: Option<RawStaticCommitteeConfig>,
    weighted_precommit: Option<RawWeightedPrecommitConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStaticCommitteeConfig {
    quorum_weight: u64,
    validators: Vec<RawCommitteeValidator>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCommitteeValidator {
    id: String,
    public_key: String,
    weight: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWeightedPrecommitConfig {
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

impl TryFrom<RawWeightedPrecommitConfig> for WeightedPrecommitConfig {
    type Error = ConsensusError;

    fn try_from(raw: RawWeightedPrecommitConfig) -> Result<Self> {
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
        "static-closed-committee" => Ok(ConsensusKind::StaticClosedCommittee),
        "weighted-precommit" => Ok(ConsensusKind::WeightedPrecommit),
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

/// Local signing capability kept separate from the public validator set.
#[derive(Clone)]
pub struct CommitteeSigner {
    validator_id: String,
    secret_key: SecretKey,
}

impl CommitteeSigner {
    /// Construct a signer from a validator id and a 32-byte secp256k1 secret key.
    pub fn new(validator_id: impl Into<String>, secret_key: [u8; 32]) -> Result<Self> {
        let validator_id = validator_id.into();
        if validator_id.is_empty() {
            return Err(ConsensusError::InvalidConfig("validator id must not be empty".to_owned()));
        }
        let secret_key = SecretKey::from_slice(&secret_key)
            .map_err(|error| ConsensusError::InvalidConfig(format!("invalid validator secret key: {error}")))?;
        Ok(Self { validator_id, secret_key })
    }

    /// Validator id carried by signatures.
    pub fn validator_id(&self) -> &str {
        &self.validator_id
    }

    /// Derive the x-only Schnorr public key for committee configuration.
    pub fn public_key(&self) -> Hash32 {
        let secp = Secp256k1::new();
        let keypair = Keypair::from_secret_key(&secp, &self.secret_key);
        XOnlyPublicKey::from_keypair(&keypair).0.serialize()
    }

    /// Build the corresponding public validator entry.
    pub fn validator(&self, weight: u64) -> CommitteeValidator {
        CommitteeValidator { id: self.validator_id.clone(), public_key: self.public_key(), weight }
    }

    /// Sign a static-committee block vote.
    pub fn sign_static(&self, block_hash: Hash32) -> CommitteeSignature {
        self.sign_digest(static_signature_digest(&self.validator_id, block_hash))
    }

    /// Sign a height/round-bound weighted precommit.
    pub fn sign_precommit(&self, block_hash: Hash32, height: u64, round: u32) -> CommitteeSignature {
        self.sign_digest(weighted_precommit_digest(&self.validator_id, block_hash, height, round))
    }

    fn sign_digest(&self, digest: Hash32) -> CommitteeSignature {
        let secp = Secp256k1::new();
        let keypair = Keypair::from_secret_key(&secp, &self.secret_key);
        let message = Message::from_digest(digest);
        let signature = secp.sign_schnorr_no_aux_rand(&message, &keypair).serialize();
        CommitteeSignature { validator_id: self.validator_id.clone(), signature }
    }
}

/// Static closed-committee configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticCommitteeConfig {
    /// Committee validators.
    pub validators: Vec<CommitteeValidator>,
    /// Weight required to finalise a block.
    pub quorum_weight: u64,
}

/// WeightedPrecommit-style validator set for finite Myelin sessions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeightedPrecommitConfig {
    /// WeightedPrecommit validators.
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

/// WeightedPrecommit precommit certificate for one block height and round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeightedPrecommitCertificate {
    /// Hash being precommitted.
    pub block_hash: Hash32,
    /// Session-local block height being precommitted.
    pub height: u64,
    /// WeightedPrecommit round being precommitted.
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
    /// WeightedPrecommit-style precommit finality.
    WeightedPrecommit(WeightedPrecommit),
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
            ConsensusKind::WeightedPrecommit => {
                let weighted_precommit = config
                    .weighted_precommit
                    .ok_or_else(|| ConsensusError::InvalidConfig("missing weighted_precommit config".to_owned()))?;
                Ok(Self::WeightedPrecommit(WeightedPrecommit::new(weighted_precommit)?))
            }
        }
    }
}

impl ConsensusEngine for SelectedConsensus {
    fn kind(&self) -> ConsensusKind {
        match self {
            SelectedConsensus::StaticClosedCommittee(engine) => engine.kind(),
            SelectedConsensus::WeightedPrecommit(engine) => engine.kind(),
        }
    }

    fn verify_certificate(&self, block_hash: Hash32, certificate: &CommitteeCertificate) -> Result<()> {
        match self {
            SelectedConsensus::StaticClosedCommittee(engine) => engine.verify_certificate(block_hash, certificate),
            SelectedConsensus::WeightedPrecommit(engine) => engine.verify_certificate(block_hash, certificate),
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
            XOnlyPublicKey::from_slice(&validator.public_key).map_err(|error| {
                ConsensusError::InvalidConfig(format!("validator {} has invalid x-only public key: {error}", validator.id))
            })?;
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

    /// Build a certificate from externally held signing capabilities.
    pub fn certificate_from_signers(&self, block_hash: Hash32, signers: &[CommitteeSigner]) -> Result<CommitteeCertificate> {
        self.validate_signers(signers)?;
        Ok(CommitteeCertificate { block_hash, signatures: signers.iter().map(|signer| signer.sign_static(block_hash)).collect() })
    }

    fn validate_signers(&self, signers: &[CommitteeSigner]) -> Result<()> {
        for signer in signers {
            let validator = self
                .validators
                .get(signer.validator_id())
                .ok_or_else(|| ConsensusError::UnknownValidator(signer.validator_id().to_owned()))?;
            if validator.public_key != signer.public_key() {
                return Err(ConsensusError::SignerKeyMismatch(signer.validator_id().to_owned()));
            }
        }
        Ok(())
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
            let digest = static_signature_digest(&signature.validator_id, block_hash);
            if !verify_schnorr(validator.public_key, digest, signature.signature) {
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

fn static_signature_digest(validator_id: &str, block_hash: Hash32) -> Hash32 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(STATIC_SIGNATURE_DOMAIN);
    hasher.update(&(validator_id.len() as u32).to_le_bytes());
    hasher.update(validator_id.as_bytes());
    hasher.update(&block_hash);
    *hasher.finalize().as_bytes()
}

/// WeightedPrecommit-style weighted precommit finality engine.
#[derive(Debug, Clone)]
pub struct WeightedPrecommit {
    validators: HashMap<String, CommitteeValidator>,
    quorum_power: u64,
}

impl WeightedPrecommit {
    /// Validate and build a WeightedPrecommit validator set.
    pub fn new(config: WeightedPrecommitConfig) -> Result<Self> {
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
            XOnlyPublicKey::from_slice(&validator.public_key).map_err(|error| {
                ConsensusError::InvalidConfig(format!("validator {} has invalid x-only public key: {error}", validator.id))
            })?;
            total_power = total_power
                .checked_add(validator.weight)
                .ok_or_else(|| ConsensusError::InvalidConfig("validator power overflow".to_owned()))?;
            if validators.insert(validator.id.clone(), validator).is_some() {
                return Err(ConsensusError::DuplicateValidator);
            }
        }

        if validators.is_empty() {
            return Err(ConsensusError::InvalidConfig(
                "weighted_precommit validator set must contain at least one validator".to_owned(),
            ));
        }
        if config.quorum_power > total_power {
            return Err(ConsensusError::InvalidConfig("quorum_power exceeds total validator power".to_owned()));
        }

        Ok(Self { validators, quorum_power: config.quorum_power })
    }

    /// Build a weighted precommit certificate from externally held signers.
    pub fn precommit_certificate_from_signers(
        &self,
        block_hash: Hash32,
        height: u64,
        round: u32,
        signers: &[CommitteeSigner],
    ) -> Result<WeightedPrecommitCertificate> {
        for signer in signers {
            let validator = self
                .validators
                .get(signer.validator_id())
                .ok_or_else(|| ConsensusError::UnknownValidator(signer.validator_id().to_owned()))?;
            if validator.public_key != signer.public_key() {
                return Err(ConsensusError::SignerKeyMismatch(signer.validator_id().to_owned()));
            }
        }
        let signatures = signers.iter().map(|signer| signer.sign_precommit(block_hash, height, round)).collect();
        Ok(WeightedPrecommitCertificate { block_hash, height, round, signatures })
    }

    /// Verify a WeightedPrecommit precommit certificate for a block at height and round.
    pub fn verify_precommit_certificate(
        &self,
        block_hash: Hash32,
        height: u64,
        round: u32,
        certificate: &WeightedPrecommitCertificate,
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
            let digest = weighted_precommit_digest(&signature.validator_id, block_hash, height, round);
            if !verify_schnorr(validator.public_key, digest, signature.signature) {
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

    /// Verify and finalise a block with a WeightedPrecommit precommit certificate.
    pub fn finalise_block_with_precommit(
        &self,
        block: MyelinBlock,
        round: u32,
        certificate: WeightedPrecommitCertificate,
    ) -> Result<FinalisedWeightedPrecommitBlock> {
        if block.consensus_kind != ConsensusKind::WeightedPrecommit {
            return Err(ConsensusError::WrongEngine {
                expected: ConsensusKind::WeightedPrecommit.as_str(),
                actual: block.consensus_kind.as_str(),
            });
        }
        let block_hash = block.hash();
        self.verify_precommit_certificate(block_hash, block.number, round, &certificate)?;
        Ok(FinalisedWeightedPrecommitBlock { block, block_hash, round, certificate })
    }
}

/// Finalised WeightedPrecommit block artefact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalisedWeightedPrecommitBlock {
    /// Finalised block.
    pub block: MyelinBlock,
    /// Block hash verified by WeightedPrecommit precommits.
    pub block_hash: Hash32,
    /// WeightedPrecommit round that reached quorum.
    pub round: u32,
    /// Precommit certificate that reached quorum.
    pub certificate: WeightedPrecommitCertificate,
}

impl ConsensusEngine for WeightedPrecommit {
    fn kind(&self) -> ConsensusKind {
        ConsensusKind::WeightedPrecommit
    }

    fn verify_certificate(&self, _block_hash: Hash32, _certificate: &CommitteeCertificate) -> Result<()> {
        // WeightedPrecommit finality is always height-bound, round-bound, and
        // block-hash-bound. The legacy CommitteeCertificate API is not
        // a structurally valid WeightedPrecommit certificate: it carries no
        // (height, round). Reject it explicitly so callers cannot
        // accidentally use the wrong API shape.
        Err(ConsensusError::LegacyCertificatePathUnsupported)
    }
}

fn weighted_precommit_digest(validator_id: &str, block_hash: Hash32, height: u64, round: u32) -> Hash32 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(WEIGHTED_PRECOMMIT_DOMAIN);
    hasher.update(&height.to_le_bytes());
    hasher.update(&round.to_le_bytes());
    hasher.update(&(validator_id.len() as u32).to_le_bytes());
    hasher.update(validator_id.as_bytes());
    hasher.update(&block_hash);
    *hasher.finalize().as_bytes()
}

fn verify_schnorr(public_key: Hash32, digest: Hash32, signature: Signature64) -> bool {
    let Ok(public_key) = XOnlyPublicKey::from_slice(&public_key) else {
        return false;
    };
    let Ok(signature) = Signature::from_slice(&signature) else {
        return false;
    };
    Secp256k1::verification_only().verify_schnorr(&signature, &Message::from_digest(digest), &public_key).is_ok()
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
    /// WeightedPrecommit precommit certificate is for another height.
    #[error("wrong weighted_precommit height: expected {expected}, got {actual}")]
    WrongHeight {
        /// Expected block height.
        expected: u64,
        /// Certificate height.
        actual: u64,
    },
    /// WeightedPrecommit precommit certificate is for another round.
    #[error("wrong weighted_precommit round: expected {expected}, got {actual}")]
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
    /// Schnorr signature does not verify against the configured public key.
    #[error("invalid signature for validator: {0}")]
    InvalidSignature(String),
    /// Local signing key does not correspond to the configured validator key.
    #[error("signing key does not match configured validator: {0}")]
    SignerKeyMismatch(String),
    /// Certificate has insufficient voting weight.
    #[error("quorum not met: signed {signed_weight}, required {quorum_weight}")]
    QuorumNotMet {
        /// Weight present in certificate.
        signed_weight: u64,
        /// Weight required for finality.
        quorum_weight: u64,
    },
    /// WeightedPrecommit does not implement the legacy generic CommitteeCertificate
    /// path. Use `verify_precommit_certificate` with a typed
    /// `WeightedPrecommitCertificate` instead.
    #[error("weighted_precommit does not implement verify_certificate; use verify_precommit_certificate with a typed WeightedPrecommitCertificate")]
    LegacyCertificatePathUnsupported,
}

/// Consensus result type.
pub type Result<T> = std::result::Result<T, ConsensusError>;

#[cfg(test)]
mod tests {
    use super::*;

    fn signer(id: &str, seed: u8) -> CommitteeSigner {
        CommitteeSigner::new(id, [seed; 32]).unwrap()
    }

    fn signers(ids: &[&str]) -> Vec<CommitteeSigner> {
        ids.iter()
            .map(|id| match *id {
                "alice" => signer("alice", 1),
                "bob" => signer("bob", 2),
                "carol" => signer("carol", 3),
                other => panic!("unknown test signer {other}"),
            })
            .collect()
    }

    fn validator(id: &str, seed: u8, weight: u64) -> CommitteeValidator {
        signer(id, seed).validator(weight)
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
        let cert = engine.certificate_from_signers(block.hash(), &signers(&["alice", "bob"])).unwrap();
        let finalised = engine.finalise_block(block.clone(), cert).unwrap();
        assert_eq!(finalised.block, block);
        assert_eq!(finalised.block_hash, block.hash());
    }

    #[test]
    fn static_committee_rejects_below_quorum() {
        let engine = committee();
        let block_hash = block().hash();
        let cert = engine.certificate_from_signers(block_hash, &signers(&["alice"])).unwrap();
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
public_key = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
weight = 1

[[static_committee.validators]]
id = "bob"
public_key = "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5"
weight = 1
"#;
        let selected = SelectedConsensus::from_config(ConsensusConfig::from_toml_str(toml).unwrap()).unwrap();
        assert_eq!(selected.kind(), ConsensusKind::StaticClosedCommittee);
    }

    fn weighted_precommit() -> WeightedPrecommit {
        WeightedPrecommit::new(WeightedPrecommitConfig {
            validators: vec![validator("alice", 1, 1), validator("bob", 2, 1), validator("carol", 3, 1)],
            quorum_power: 2,
        })
        .unwrap()
    }

    #[test]
    fn weighted_precommit_finalises_with_precommit_quorum() {
        let engine = weighted_precommit();
        let block = block_for(ConsensusKind::WeightedPrecommit);
        let cert = engine.precommit_certificate_from_signers(block.hash(), block.number, 0, &signers(&["alice", "bob"])).unwrap();
        let finalised = engine.finalise_block_with_precommit(block.clone(), 0, cert).unwrap();
        assert_eq!(finalised.block, block);
        assert_eq!(finalised.block_hash, block.hash());
        assert_eq!(finalised.round, 0);
    }

    #[test]
    fn weighted_precommit_rejects_below_quorum() {
        let engine = weighted_precommit();
        let block = block_for(ConsensusKind::WeightedPrecommit);
        let block_hash = block.hash();
        let cert = engine.precommit_certificate_from_signers(block_hash, block.number, 0, &signers(&["alice"])).unwrap();
        assert_eq!(
            engine.verify_precommit_certificate(block_hash, block.number, 0, &cert),
            Err(ConsensusError::QuorumNotMet { signed_weight: 1, quorum_weight: 2 })
        );
    }

    #[test]
    fn weighted_precommit_rejects_wrong_height_and_round() {
        let engine = weighted_precommit();
        let block = block_for(ConsensusKind::WeightedPrecommit);
        let block_hash = block.hash();
        let cert = engine.precommit_certificate_from_signers(block_hash, block.number, 0, &signers(&["alice", "bob"])).unwrap();
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
    fn selected_weighted_precommit_loads_from_toml() {
        let toml = r#"
kind = "weighted-precommit"

[weighted_precommit]
quorum_power = 2

[[weighted_precommit.validators]]
id = "alice"
public_key = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798"
weight = 1

[[weighted_precommit.validators]]
id = "bob"
public_key = "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5"
weight = 1
"#;
        let selected = SelectedConsensus::from_config(ConsensusConfig::from_toml_str(toml).unwrap()).unwrap();
        assert_eq!(selected.kind(), ConsensusKind::WeightedPrecommit);
    }

    // ─── Additional StaticClosedCommittee tests ──────────────────────────────────

    #[test]
    fn static_committee_rejects_duplicate_validator() {
        let engine = committee();
        let block_hash = block().hash();
        let mut cert = engine.certificate_from_signers(block_hash, &signers(&["alice", "bob"])).unwrap();
        // Insert a duplicate alice signature so the validator appears twice
        cert.signatures.push(cert.signatures[0].clone());
        assert_eq!(engine.verify_certificate(block_hash, &cert), Err(ConsensusError::DuplicateValidator));
    }

    #[test]
    fn static_committee_rejects_unknown_validator() {
        let engine = committee();
        let block_hash = block().hash();
        let mut cert = engine.certificate_from_signers(block_hash, &signers(&["alice"])).unwrap();
        cert.signatures.push(CommitteeSignature { validator_id: "ghost".to_owned(), signature: [0xAA; 64] });
        assert_eq!(engine.verify_certificate(block_hash, &cert), Err(ConsensusError::UnknownValidator("ghost".to_owned())));
    }

    #[test]
    fn static_committee_rejects_wrong_block_hash() {
        let engine = committee();
        let block = block();
        let cert = engine.certificate_from_signers(block.hash(), &signers(&["alice", "bob"])).unwrap();
        let wrong_hash = [0xFF; 32];
        assert_eq!(engine.verify_certificate(wrong_hash, &cert), Err(ConsensusError::WrongBlockHash));
    }

    #[test]
    fn static_committee_rejects_invalid_signature() {
        let engine = committee();
        let block = block();
        let block_hash = block.hash();
        let mut cert = engine.certificate_from_signers(block_hash, &signers(&["alice", "bob"])).unwrap();
        // Replace alice's signature with garbage
        cert.signatures[0].signature = [0xDE; 64];
        assert_eq!(engine.verify_certificate(block_hash, &cert), Err(ConsensusError::InvalidSignature("alice".to_owned())));
    }

    #[test]
    fn static_committee_finalised_block_is_stable() {
        let engine = committee();
        let block = block();
        let cert1 = engine.certificate_from_signers(block.hash(), &signers(&["alice", "bob"])).unwrap();
        let cert2 = engine.certificate_from_signers(block.hash(), &signers(&["alice", "bob"])).unwrap();
        let f1 = engine.finalise_block(block.clone(), cert1).unwrap();
        let f2 = engine.finalise_block(block.clone(), cert2).unwrap();
        assert_eq!(f1.block_hash, f2.block_hash);
        assert_eq!(f1.certificate.block_hash, f2.certificate.block_hash);
    }

    // ─── Additional WeightedPrecommit tests ──────────────────────────────────────────────

    #[test]
    fn weighted_precommit_rejects_wrong_block_hash() {
        let engine = weighted_precommit();
        let block = block_for(ConsensusKind::WeightedPrecommit);
        let block_hash = block.hash();
        let cert = engine.precommit_certificate_from_signers(block_hash, block.number, 0, &signers(&["alice", "bob"])).unwrap();
        let wrong_hash = [0xEE; 32];
        assert_eq!(engine.verify_precommit_certificate(wrong_hash, block.number, 0, &cert), Err(ConsensusError::WrongBlockHash));
    }

    #[test]
    fn weighted_precommit_rejects_duplicate_validator() {
        let engine = weighted_precommit();
        let block = block_for(ConsensusKind::WeightedPrecommit);
        let block_hash = block.hash();
        let mut cert = engine.precommit_certificate_from_signers(block_hash, block.number, 0, &signers(&["alice", "bob"])).unwrap();
        cert.signatures.push(cert.signatures[0].clone());
        assert_eq!(engine.verify_precommit_certificate(block_hash, block.number, 0, &cert), Err(ConsensusError::DuplicateValidator));
    }

    #[test]
    fn weighted_precommit_rejects_unknown_validator() {
        let engine = weighted_precommit();
        let block = block_for(ConsensusKind::WeightedPrecommit);
        let block_hash = block.hash();
        let mut cert = engine.precommit_certificate_from_signers(block_hash, block.number, 0, &signers(&["alice", "bob"])).unwrap();
        cert.signatures.push(CommitteeSignature { validator_id: "ghost".to_owned(), signature: [0xAA; 64] });
        assert_eq!(
            engine.verify_precommit_certificate(block_hash, block.number, 0, &cert),
            Err(ConsensusError::UnknownValidator("ghost".to_owned()))
        );
    }

    #[test]
    fn weighted_precommit_rejects_invalid_signature() {
        let engine = weighted_precommit();
        let block = block_for(ConsensusKind::WeightedPrecommit);
        let block_hash = block.hash();
        let mut cert = engine.precommit_certificate_from_signers(block_hash, block.number, 0, &signers(&["alice", "bob"])).unwrap();
        cert.signatures[0].signature = [0xCD; 64];
        assert_eq!(
            engine.verify_precommit_certificate(block_hash, block.number, 0, &cert),
            Err(ConsensusError::InvalidSignature("alice".to_owned()))
        );
    }

    #[test]
    fn weighted_precommit_rejects_height_round_combination() {
        let engine = weighted_precommit();
        let block = block_for(ConsensusKind::WeightedPrecommit);
        let block_hash = block.hash();
        let cert = engine.precommit_certificate_from_signers(block_hash, block.number, 2, &signers(&["alice", "bob"])).unwrap();
        // Verify the same precommit under (height, round) = (number+1, 3) must fail
        assert_eq!(
            engine.verify_precommit_certificate(block_hash, block.number + 1, 3, &cert),
            Err(ConsensusError::WrongHeight { expected: block.number + 1, actual: block.number })
        );
    }

    #[test]
    fn weighted_precommit_does_not_silently_fall_back_to_static_committee() {
        // WeightedPrecommit's legacy `verify_certificate` path is explicitly
        // rejected: a CommitteeCertificate carries no (height, round),
        // so it is not a structurally valid WeightedPrecommit precommit. A
        // caller who tries to use the legacy path must get an explicit
        // error, never a silent acceptance.
        let weighted_precommit_engine = weighted_precommit();
        let static_engine = committee();

        let block = block_for(ConsensusKind::StaticClosedCommittee);
        let block_hash = block.hash();

        // Build a WeightedPrecommit precommit certificate on the same block hash
        // and hand its signatures to the static-committee verifier as a
        // CommitteeCertificate. The signature domains differ, so the
        // static engine must reject as InvalidSignature.
        let precommit = weighted_precommit_engine
            .precommit_certificate_from_signers(block_hash, block.number, 0, &signers(&["alice", "bob"]))
            .unwrap();
        let cross_cert = CommitteeCertificate { block_hash, signatures: precommit.signatures.clone() };
        assert!(matches!(static_engine.verify_certificate(block_hash, &cross_cert), Err(ConsensusError::InvalidSignature(_))));

        // The WeightedPrecommit engine's legacy `verify_certificate` path must
        // explicitly reject the generic CommitteeCertificate API shape.
        let static_cert = static_engine.certificate_from_signers(block_hash, &signers(&["alice", "bob"])).unwrap();
        assert!(matches!(
            weighted_precommit_engine.verify_certificate(block_hash, &static_cert),
            Err(ConsensusError::LegacyCertificatePathUnsupported)
        ));
    }

    #[test]
    fn weighted_precommit_finalised_block_is_stable() {
        let engine = weighted_precommit();
        let block = block_for(ConsensusKind::WeightedPrecommit);
        let cert1 = engine.precommit_certificate_from_signers(block.hash(), block.number, 0, &signers(&["alice", "bob"])).unwrap();
        let cert2 = engine.precommit_certificate_from_signers(block.hash(), block.number, 0, &signers(&["alice", "bob"])).unwrap();
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
        // A static-committee block must not be finalised by the WeightedPrecommit
        // SelectedConsensus path. The WeightedPrecommit finalise path requires
        // (round, WeightedPrecommitCertificate), so we exercise the
        // wrong-engine guard via the precommit entry point.
        let weighted_precommit_engine = weighted_precommit();
        let static_block = block_for(ConsensusKind::StaticClosedCommittee);
        let cert = weighted_precommit_engine
            .precommit_certificate_from_signers(static_block.hash(), static_block.number, 0, &signers(&["alice", "bob"]))
            .unwrap();
        assert!(matches!(
            weighted_precommit_engine.finalise_block_with_precommit(static_block, 0, cert),
            Err(ConsensusError::WrongEngine { .. })
        ));
    }

    #[test]
    fn selected_consensus_static_committee_does_not_accept_weighted_precommit_kind_block() {
        // A WeightedPrecommit block (consensus_kind = WeightedPrecommit) must not be
        // accepted by the static-committee finalise path either.
        let static_engine = committee();
        let selected = SelectedConsensus::StaticClosedCommittee(static_engine.clone());
        let weighted_precommit_block = block_for(ConsensusKind::WeightedPrecommit);
        let cert = static_engine.certificate_from_signers(weighted_precommit_block.hash(), &signers(&["alice", "bob"])).unwrap();
        assert!(matches!(selected.finalise_block(weighted_precommit_block, cert), Err(ConsensusError::WrongEngine { .. })));
    }
}
