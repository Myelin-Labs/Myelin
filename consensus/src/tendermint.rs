// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers

//! Finite-session Tendermint round state.
//!
//! The state is intentionally serializable: callers persist it as their WAL
//! boundary before signing the next proposal or vote. Networking and timeout
//! scheduling stay outside this deterministic state machine.

use super::{
    verify_schnorr, CommitteeSigner, ConsensusError, FinalisedWeightedPrecommitBlock, Hash32, MyelinBlock, Result, Signature64,
    WeightedPrecommit, WeightedPrecommitCertificate,
};
use secp256k1::{Keypair, Message, Secp256k1};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

const PROPOSAL_DOMAIN: &[u8] = b"myelin:tendermint-proposal:v1";
const VOTE_DOMAIN: &[u8] = b"myelin:tendermint-vote:v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TendermintStep {
    Prevote,
    Precommit,
}

impl TendermintStep {
    fn tag(self) -> u8 {
        match self {
            Self::Prevote => 1,
            Self::Precommit => 2,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TendermintProposal {
    pub height: u64,
    pub round: u32,
    pub block_hash: Hash32,
    pub valid_round: Option<u32>,
    pub proposer_id: String,
    pub signature: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TendermintVote {
    pub height: u64,
    pub round: u32,
    pub step: TendermintStep,
    pub block_hash: Option<Hash32>,
    pub validator_id: String,
    pub signature: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TendermintDecision {
    pub height: u64,
    pub round: u32,
    pub block_hash: Hash32,
    pub precommits: Vec<TendermintVote>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TendermintRoundState {
    pub height: u64,
    pub round: u32,
    pub locked_value: Option<Hash32>,
    pub locked_round: Option<u32>,
    pub valid_value: Option<Hash32>,
    pub valid_round: Option<u32>,
    pub proposals: Vec<TendermintProposal>,
    pub votes: Vec<TendermintVote>,
    pub decision: Option<TendermintDecision>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TendermintProgress {
    ProposalAccepted,
    VoteAccepted,
    PrevoteQuorum(Option<Hash32>),
    PrecommitQuorumNil,
    Decided(TendermintDecision),
}

impl WeightedPrecommit {
    pub fn new_round_state(&self, height: u64) -> Result<TendermintRoundState> {
        self.validate_tendermint_quorum()?;
        Ok(TendermintRoundState {
            height,
            round: 0,
            locked_value: None,
            locked_round: None,
            valid_value: None,
            valid_round: None,
            proposals: Vec::new(),
            votes: Vec::new(),
            decision: None,
        })
    }

    pub fn proposer_id(&self, height: u64, round: u32) -> Result<&str> {
        let total_power = self.total_power()?;
        let target = (height % total_power + u64::from(round) % total_power) % total_power;
        let mut validators = self.validators.values().collect::<Vec<_>>();
        validators.sort_by(|left, right| left.id.cmp(&right.id));
        let mut cursor = 0u64;
        for validator in validators {
            cursor = cursor
                .checked_add(validator.weight)
                .ok_or_else(|| ConsensusError::InvalidConfig("validator power overflow".to_owned()))?;
            if target < cursor {
                return Ok(&validator.id);
            }
        }
        Err(ConsensusError::InvalidTendermintState("proposer selection exhausted validator power".to_owned()))
    }

    pub fn proposal_from_signer(
        &self,
        height: u64,
        round: u32,
        block_hash: Hash32,
        valid_round: Option<u32>,
        signer: &CommitteeSigner,
    ) -> Result<TendermintProposal> {
        let expected = self.proposer_id(height, round)?;
        if signer.validator_id() != expected {
            return Err(ConsensusError::UnexpectedProposer {
                expected: expected.to_owned(),
                actual: signer.validator_id().to_owned(),
            });
        }
        self.validate_signer(signer)?;
        if valid_round.is_some_and(|valid_round| valid_round >= round) {
            return Err(ConsensusError::InvalidTendermintState("proposal valid_round must be lower than round".to_owned()));
        }
        let digest = proposal_digest(height, round, block_hash, valid_round, signer.validator_id());
        Ok(TendermintProposal {
            height,
            round,
            block_hash,
            valid_round,
            proposer_id: signer.validator_id().to_owned(),
            signature: signer.sign_tendermint_digest(digest).to_vec(),
        })
    }

    pub fn vote_from_signer(
        &self,
        height: u64,
        round: u32,
        step: TendermintStep,
        block_hash: Option<Hash32>,
        signer: &CommitteeSigner,
    ) -> Result<TendermintVote> {
        self.validate_signer(signer)?;
        let digest = vote_digest(height, round, step, block_hash, signer.validator_id());
        Ok(TendermintVote {
            height,
            round,
            step,
            block_hash,
            validator_id: signer.validator_id().to_owned(),
            signature: signer.sign_tendermint_digest(digest).to_vec(),
        })
    }

    pub fn verify_proposal(&self, proposal: &TendermintProposal) -> Result<()> {
        let expected = self.proposer_id(proposal.height, proposal.round)?;
        if proposal.proposer_id != expected {
            return Err(ConsensusError::UnexpectedProposer { expected: expected.to_owned(), actual: proposal.proposer_id.clone() });
        }
        if proposal.valid_round.is_some_and(|valid_round| valid_round >= proposal.round) {
            return Err(ConsensusError::InvalidTendermintState("proposal valid_round must be lower than round".to_owned()));
        }
        let validator = self
            .validators
            .get(&proposal.proposer_id)
            .ok_or_else(|| ConsensusError::UnknownValidator(proposal.proposer_id.clone()))?;
        let signature = signature64(&proposal.signature, &proposal.proposer_id)?;
        let digest =
            proposal_digest(proposal.height, proposal.round, proposal.block_hash, proposal.valid_round, &proposal.proposer_id);
        if !verify_schnorr(validator.public_key, digest, signature) {
            return Err(ConsensusError::InvalidSignature(proposal.proposer_id.clone()));
        }
        Ok(())
    }

    pub fn verify_vote(&self, vote: &TendermintVote) -> Result<()> {
        let validator =
            self.validators.get(&vote.validator_id).ok_or_else(|| ConsensusError::UnknownValidator(vote.validator_id.clone()))?;
        let signature = signature64(&vote.signature, &vote.validator_id)?;
        let digest = vote_digest(vote.height, vote.round, vote.step, vote.block_hash, &vote.validator_id);
        if !verify_schnorr(validator.public_key, digest, signature) {
            return Err(ConsensusError::InvalidSignature(vote.validator_id.clone()));
        }
        Ok(())
    }

    pub fn finalise_block_with_decision(
        &self,
        block: MyelinBlock,
        decision: TendermintDecision,
    ) -> Result<FinalisedWeightedPrecommitBlock> {
        if block.consensus_kind != super::ConsensusKind::WeightedPrecommit {
            return Err(ConsensusError::WrongEngine {
                expected: super::ConsensusKind::WeightedPrecommit.as_str(),
                actual: block.consensus_kind.as_str(),
            });
        }
        if decision.height != block.number {
            return Err(ConsensusError::WrongHeight { expected: block.number, actual: decision.height });
        }
        let block_hash = block.hash();
        if decision.block_hash != block_hash {
            return Err(ConsensusError::WrongBlockHash);
        }
        let signed_power =
            self.vote_power(&decision.precommits, decision.height, decision.round, TendermintStep::Precommit, Some(block_hash))?;
        if signed_power < self.quorum_power {
            return Err(ConsensusError::QuorumNotMet { signed_weight: signed_power, quorum_weight: self.quorum_power });
        }
        let signatures = decision
            .precommits
            .iter()
            .map(|vote| {
                Ok(super::CommitteeSignature {
                    validator_id: vote.validator_id.clone(),
                    signature: signature64(&vote.signature, &vote.validator_id)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let certificate = WeightedPrecommitCertificate { block_hash, height: decision.height, round: decision.round, signatures };
        Ok(FinalisedWeightedPrecommitBlock { block, block_hash, round: decision.round, certificate })
    }

    fn validate_tendermint_quorum(&self) -> Result<()> {
        let total = self.total_power()?;
        let quorum_times_three =
            self.quorum_power.checked_mul(3).ok_or_else(|| ConsensusError::InvalidConfig("quorum power overflow".to_owned()))?;
        let total_times_two =
            total.checked_mul(2).ok_or_else(|| ConsensusError::InvalidConfig("validator power overflow".to_owned()))?;
        if quorum_times_three <= total_times_two {
            return Err(ConsensusError::InvalidConfig(format!(
                "Tendermint quorum {} must be strictly greater than two-thirds of total power {total}",
                self.quorum_power
            )));
        }
        Ok(())
    }

    fn total_power(&self) -> Result<u64> {
        self.validators.values().try_fold(0u64, |total, validator| {
            total.checked_add(validator.weight).ok_or_else(|| ConsensusError::InvalidConfig("validator power overflow".to_owned()))
        })
    }

    fn validate_signer(&self, signer: &CommitteeSigner) -> Result<()> {
        let validator = self
            .validators
            .get(signer.validator_id())
            .ok_or_else(|| ConsensusError::UnknownValidator(signer.validator_id().to_owned()))?;
        if validator.public_key != signer.public_key() {
            return Err(ConsensusError::SignerKeyMismatch(signer.validator_id().to_owned()));
        }
        Ok(())
    }

    fn vote_power(
        &self,
        votes: &[TendermintVote],
        height: u64,
        round: u32,
        step: TendermintStep,
        block_hash: Option<Hash32>,
    ) -> Result<u64> {
        let mut seen = HashSet::new();
        let mut power = 0u64;
        for vote in votes
            .iter()
            .filter(|vote| vote.height == height && vote.round == round && vote.step == step && vote.block_hash == block_hash)
        {
            self.verify_vote(vote)?;
            if !seen.insert(vote.validator_id.as_str()) {
                return Err(ConsensusError::DuplicateValidator);
            }
            power = power
                .checked_add(self.validators[&vote.validator_id].weight)
                .ok_or_else(|| ConsensusError::InvalidConfig("vote power overflow".to_owned()))?;
        }
        Ok(power)
    }
}

impl CommitteeSigner {
    fn sign_tendermint_digest(&self, digest: Hash32) -> Signature64 {
        let secp = Secp256k1::new();
        let keypair = Keypair::from_secret_key(&secp, &self.secret_key);
        secp.sign_schnorr_no_aux_rand(&Message::from_digest(digest), &keypair).serialize()
    }
}

impl TendermintRoundState {
    pub fn receive_proposal(&mut self, engine: &WeightedPrecommit, proposal: TendermintProposal) -> Result<TendermintProgress> {
        self.ensure_live()?;
        if proposal.height != self.height {
            return Err(ConsensusError::WrongHeight { expected: self.height, actual: proposal.height });
        }
        if proposal.round != self.round {
            return Err(ConsensusError::WrongRound { expected: self.round, actual: proposal.round });
        }
        engine.verify_proposal(&proposal)?;
        if let Some(existing) = self.proposals.iter().find(|existing| existing.round == proposal.round) {
            if existing == &proposal {
                return Ok(TendermintProgress::ProposalAccepted);
            }
            return Err(ConsensusError::Equivocation { validator_id: proposal.proposer_id, round: proposal.round, step: "proposal" });
        }
        if let Some(valid_round) = proposal.valid_round {
            let power =
                engine.vote_power(&self.votes, self.height, valid_round, TendermintStep::Prevote, Some(proposal.block_hash))?;
            if power < engine.quorum_power {
                return Err(ConsensusError::InvalidTendermintState("proposal valid_round has no retained prevote quorum".to_owned()));
            }
        }
        self.proposals.push(proposal);
        Ok(TendermintProgress::ProposalAccepted)
    }

    pub fn prevote_value(&self, engine: &WeightedPrecommit) -> Result<Option<Hash32>> {
        self.ensure_live()?;
        let Some(proposal) = self.proposals.iter().find(|proposal| proposal.round == self.round) else {
            return Ok(None);
        };
        match (self.locked_value, self.locked_round) {
            (None, _) => Ok(Some(proposal.block_hash)),
            (Some(locked), _) if locked == proposal.block_hash => Ok(Some(proposal.block_hash)),
            (Some(_), Some(locked_round)) => {
                let Some(valid_round) = proposal.valid_round.filter(|valid_round| *valid_round >= locked_round) else {
                    return Ok(None);
                };
                let power =
                    engine.vote_power(&self.votes, self.height, valid_round, TendermintStep::Prevote, Some(proposal.block_hash))?;
                Ok((power >= engine.quorum_power).then_some(proposal.block_hash))
            }
            (Some(_), None) => Err(ConsensusError::InvalidTendermintState("locked value is missing locked round".to_owned())),
        }
    }

    pub fn receive_vote(&mut self, engine: &WeightedPrecommit, vote: TendermintVote) -> Result<TendermintProgress> {
        self.ensure_live()?;
        if vote.height != self.height {
            return Err(ConsensusError::WrongHeight { expected: self.height, actual: vote.height });
        }
        if vote.round > self.round {
            return Err(ConsensusError::WrongRound { expected: self.round, actual: vote.round });
        }
        engine.verify_vote(&vote)?;
        if let Some(existing) = self
            .votes
            .iter()
            .find(|existing| existing.round == vote.round && existing.step == vote.step && existing.validator_id == vote.validator_id)
        {
            if existing == &vote {
                return Ok(TendermintProgress::VoteAccepted);
            }
            return Err(ConsensusError::Equivocation {
                validator_id: vote.validator_id,
                round: vote.round,
                step: match vote.step {
                    TendermintStep::Prevote => "prevote",
                    TendermintStep::Precommit => "precommit",
                },
            });
        }
        let round = vote.round;
        let step = vote.step;
        let value = vote.block_hash;
        self.votes.push(vote);
        let power = engine.vote_power(&self.votes, self.height, round, step, value)?;
        if power < engine.quorum_power {
            return Ok(TendermintProgress::VoteAccepted);
        }
        match (step, value) {
            (TendermintStep::Prevote, value) => {
                if let Some(block_hash) = value {
                    self.valid_value = Some(block_hash);
                    self.valid_round = Some(round);
                }
                Ok(TendermintProgress::PrevoteQuorum(value))
            }
            (TendermintStep::Precommit, None) => Ok(TendermintProgress::PrecommitQuorumNil),
            (TendermintStep::Precommit, Some(block_hash)) => {
                let precommits = self
                    .votes
                    .iter()
                    .filter(|vote| {
                        vote.height == self.height
                            && vote.round == round
                            && vote.step == TendermintStep::Precommit
                            && vote.block_hash == Some(block_hash)
                    })
                    .cloned()
                    .collect();
                let decision = TendermintDecision { height: self.height, round, block_hash, precommits };
                self.decision = Some(decision.clone());
                Ok(TendermintProgress::Decided(decision))
            }
        }
    }

    pub fn precommit_value(&mut self, engine: &WeightedPrecommit) -> Result<Option<Hash32>> {
        self.ensure_live()?;
        let mut candidates = self
            .votes
            .iter()
            .filter(|vote| vote.height == self.height && vote.round == self.round && vote.step == TendermintStep::Prevote)
            .map(|vote| vote.block_hash)
            .collect::<Vec<_>>();
        candidates.sort_unstable();
        candidates.dedup();
        for value in candidates {
            let power = engine.vote_power(&self.votes, self.height, self.round, TendermintStep::Prevote, value)?;
            if power >= engine.quorum_power {
                if let Some(block_hash) = value {
                    self.locked_value = Some(block_hash);
                    self.locked_round = Some(self.round);
                    self.valid_value = Some(block_hash);
                    self.valid_round = Some(self.round);
                }
                return Ok(value);
            }
        }
        Err(ConsensusError::InvalidTendermintState("cannot precommit before a prevote quorum".to_owned()))
    }

    pub fn advance_round(&mut self) -> Result<u32> {
        self.ensure_live()?;
        self.round = self.round.checked_add(1).ok_or_else(|| ConsensusError::InvalidTendermintState("round overflow".to_owned()))?;
        Ok(self.round)
    }

    fn ensure_live(&self) -> Result<()> {
        if self.decision.is_some() {
            return Err(ConsensusError::InvalidTendermintState("height is already decided".to_owned()));
        }
        Ok(())
    }
}

fn proposal_digest(height: u64, round: u32, block_hash: Hash32, valid_round: Option<u32>, proposer_id: &str) -> Hash32 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(PROPOSAL_DOMAIN);
    hasher.update(&height.to_le_bytes());
    hasher.update(&round.to_le_bytes());
    hasher.update(&block_hash);
    match valid_round {
        Some(valid_round) => {
            hasher.update(&[1]);
            hasher.update(&valid_round.to_le_bytes());
        }
        None => {
            hasher.update(&[0]);
        }
    };
    hasher.update(&(proposer_id.len() as u32).to_le_bytes());
    hasher.update(proposer_id.as_bytes());
    *hasher.finalize().as_bytes()
}

fn vote_digest(height: u64, round: u32, step: TendermintStep, block_hash: Option<Hash32>, validator_id: &str) -> Hash32 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(VOTE_DOMAIN);
    hasher.update(&height.to_le_bytes());
    hasher.update(&round.to_le_bytes());
    hasher.update(&[step.tag()]);
    match block_hash {
        Some(block_hash) => {
            hasher.update(&[1]);
            hasher.update(&block_hash);
        }
        None => {
            hasher.update(&[0]);
        }
    };
    hasher.update(&(validator_id.len() as u32).to_le_bytes());
    hasher.update(validator_id.as_bytes());
    *hasher.finalize().as_bytes()
}

fn signature64(signature: &[u8], validator_id: &str) -> Result<Signature64> {
    signature.try_into().map_err(|_| ConsensusError::InvalidSignature(validator_id.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CommitteeValidator, ConsensusKind, WeightedPrecommitConfig};
    use secp256k1::{SecretKey, XOnlyPublicKey};

    fn signer(id: &str, seed: u8) -> CommitteeSigner {
        CommitteeSigner::new(id, [seed; 32]).unwrap()
    }

    fn signers() -> Vec<CommitteeSigner> {
        vec![signer("alice", 1), signer("bob", 2), signer("carol", 3), signer("dave", 4)]
    }

    fn engine() -> WeightedPrecommit {
        let validators = signers().iter().map(|signer| signer.validator(1)).collect::<Vec<CommitteeValidator>>();
        WeightedPrecommit::new(WeightedPrecommitConfig { validators, quorum_power: 3 }).unwrap()
    }

    fn block() -> MyelinBlock {
        MyelinBlock {
            version: 1,
            parent_hash: [0; 32],
            number: 8,
            timestamp_ms: 0,
            consensus_kind: ConsensusKind::WeightedPrecommit,
            state_root_before: [1; 32],
            state_root_after: [2; 32],
            ordered_cell_tx_commitments: vec![[3; 32]],
            data_commitments: vec![[4; 32]],
            scheduler_commitment: [5; 32],
        }
    }

    fn run_round(engine: &WeightedPrecommit, block: &MyelinBlock) -> (TendermintRoundState, TendermintDecision) {
        let signers = signers();
        let mut state = engine.new_round_state(block.number).unwrap();
        let proposer_id = engine.proposer_id(block.number, 0).unwrap();
        let proposer = signers.iter().find(|signer| signer.validator_id() == proposer_id).unwrap();
        let proposal = engine.proposal_from_signer(block.number, 0, block.hash(), None, proposer).unwrap();
        assert_eq!(state.receive_proposal(engine, proposal).unwrap(), TendermintProgress::ProposalAccepted);
        let prevote = state.prevote_value(engine).unwrap();
        assert_eq!(prevote, Some(block.hash()));
        for signer in signers.iter().take(3) {
            let vote = engine.vote_from_signer(block.number, 0, TendermintStep::Prevote, prevote, signer).unwrap();
            state.receive_vote(engine, vote).unwrap();
        }
        let precommit = state.precommit_value(engine).unwrap();
        assert_eq!(state.locked_value, Some(block.hash()));
        let mut decision = None;
        for signer in signers.iter().take(3) {
            let vote = engine.vote_from_signer(block.number, 0, TendermintStep::Precommit, precommit, signer).unwrap();
            if let TendermintProgress::Decided(value) = state.receive_vote(engine, vote).unwrap() {
                decision = Some(value);
            }
        }
        (state, decision.unwrap())
    }

    #[test]
    fn full_propose_prevote_precommit_round_finalises() {
        let engine = engine();
        let block = block();
        let (state, decision) = run_round(&engine, &block);
        assert_eq!(state.valid_value, Some(block.hash()));
        assert_eq!(state.locked_round, Some(0));
        let finalised = engine.finalise_block_with_decision(block.clone(), decision).unwrap();
        assert_eq!(finalised.block, block);
        assert_eq!(finalised.round, 0);
    }

    #[test]
    fn nil_precommit_advances_round_without_unlocking() {
        let engine = engine();
        let signers = signers();
        let mut state = engine.new_round_state(8).unwrap();
        for signer in signers.iter().take(3) {
            let prevote = engine.vote_from_signer(8, 0, TendermintStep::Prevote, None, signer).unwrap();
            state.receive_vote(&engine, prevote).unwrap();
        }
        assert_eq!(state.precommit_value(&engine).unwrap(), None);
        for signer in signers.iter().take(3) {
            let precommit = engine.vote_from_signer(8, 0, TendermintStep::Precommit, None, signer).unwrap();
            state.receive_vote(&engine, precommit).unwrap();
        }
        assert_eq!(state.advance_round().unwrap(), 1);
        assert_eq!(state.locked_value, None);
    }

    #[test]
    fn state_rejects_equivocation_and_preserves_lock_across_timeout() {
        let engine = engine();
        let block = block();
        let (mut state, _) = run_round(&engine, &block);
        state.decision = None;
        let alice = signer("alice", 1);
        let conflicting = engine.vote_from_signer(8, 0, TendermintStep::Prevote, Some([9; 32]), &alice).unwrap();
        assert!(matches!(
            state.receive_vote(&engine, conflicting),
            Err(ConsensusError::Equivocation { validator_id, step: "prevote", .. }) if validator_id == "alice"
        ));
        state.advance_round().unwrap();
        assert_eq!(state.locked_value, Some(block.hash()));
        assert_eq!(state.locked_round, Some(0));
    }

    #[test]
    fn state_is_json_wal_roundtrippable_and_rejects_unsafe_quorum() {
        let engine = engine();
        let state = engine.new_round_state(8).unwrap();
        let encoded = toml::to_string(&state).unwrap();
        assert_eq!(toml::from_str::<TendermintRoundState>(&encoded).unwrap(), state);

        let unsafe_engine = WeightedPrecommit::new(WeightedPrecommitConfig {
            validators: signers().iter().take(3).map(|signer| signer.validator(1)).collect(),
            quorum_power: 2,
        })
        .unwrap();
        assert!(
            matches!(unsafe_engine.new_round_state(8), Err(ConsensusError::InvalidConfig(message)) if message.contains("two-thirds"))
        );
    }

    #[test]
    fn invalid_proposer_and_signature_are_rejected() {
        let engine = engine();
        let mut state = engine.new_round_state(8).unwrap();
        let wrong = signer("alice", 1);
        if engine.proposer_id(8, 0).unwrap() != "alice" {
            let error = engine.proposal_from_signer(8, 0, [1; 32], None, &wrong).unwrap_err();
            assert!(matches!(error, ConsensusError::UnexpectedProposer { .. }));
        }
        let proposer_id = engine.proposer_id(8, 0).unwrap();
        let proposer = signers().into_iter().find(|signer| signer.validator_id() == proposer_id).unwrap();
        let mut proposal = engine.proposal_from_signer(8, 0, [1; 32], None, &proposer).unwrap();
        proposal.signature[0] ^= 1;
        assert!(matches!(state.receive_proposal(&engine, proposal), Err(ConsensusError::InvalidSignature(_))));

        let _ = SecretKey::from_slice(&[1; 32]).unwrap();
        let _ = XOnlyPublicKey::from_slice(&signer("alice", 1).public_key()).unwrap();
    }
}
