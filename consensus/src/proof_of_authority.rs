// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers

//! Deterministic rotating proof-of-authority finality.
//!
//! PoA is a closed-authority engine: exactly one configured authority is
//! eligible to seal each height. Authority ordering is part of configuration,
//! and the scheduled authority is `height % authorities.len()`.

use super::{
    poa_config_commitment, CkbPublicKey33, CkbSignature65, CommitteeSigner, ConsensusError, ConsensusKind, Hash32, MyelinBlock, Result,
};
use myelin_wallet_auth::{lock_arg_from_public_key, poa_seal_digest, recover_public_key, CkbLockArg};
use std::collections::HashSet;

/// One public signing authority in schedule order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Authority {
    /// Stable authority identifier carried by seals.
    pub id: String,
    /// Compressed secp256k1 key hashed by the standard CKB Blake160 rule.
    pub public_key: CkbPublicKey33,
}

impl Authority {
    /// Standard CKB secp256k1 lock args for this authority.
    pub fn ckb_lock_arg(&self) -> CkbLockArg {
        lock_arg_from_public_key(self.public_key).expect("PoA configuration validates every compressed public key")
    }
}

/// Ordered authority-set configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProofOfAuthorityConfig {
    /// Authorities in deterministic rotation order.
    pub authorities: Vec<Authority>,
}

/// Height-bound PoA block seal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProofOfAuthoritySeal {
    /// Hash of the sealed block.
    pub block_hash: Hash32,
    /// Session-local height of the sealed block.
    pub height: u64,
    /// Scheduled authority that produced the seal.
    pub authority_id: String,
    /// CKB-compatible compact recoverable ECDSA signature.
    pub signature: CkbSignature65,
}

/// A block finalised by its scheduled PoA authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FinalisedProofOfAuthorityBlock {
    /// Finalised block.
    pub block: MyelinBlock,
    /// Verified block hash.
    pub block_hash: Hash32,
    /// Verified authority seal.
    pub seal: ProofOfAuthoritySeal,
}

/// Deterministic rotating proof-of-authority engine.
#[derive(Clone, Debug)]
pub struct ProofOfAuthority {
    authorities: Vec<Authority>,
}

impl ProofOfAuthority {
    /// Validate an ordered authority set.
    pub fn new(config: ProofOfAuthorityConfig) -> Result<Self> {
        if config.authorities.is_empty() {
            return Err(ConsensusError::InvalidConfig("proof-of-authority must contain at least one authority".to_owned()));
        }

        let mut ids = HashSet::with_capacity(config.authorities.len());
        let mut lock_args = HashSet::with_capacity(config.authorities.len());
        for authority in &config.authorities {
            if authority.id.is_empty() {
                return Err(ConsensusError::InvalidConfig("authority id must not be empty".to_owned()));
            }
            if !ids.insert(authority.id.as_str()) {
                return Err(ConsensusError::DuplicateAuthority(authority.id.clone()));
            }
            let lock_arg = lock_arg_from_public_key(authority.public_key).map_err(|error| {
                ConsensusError::InvalidConfig(format!("authority {} has invalid compressed public key: {error}", authority.id))
            })?;
            if !lock_args.insert(lock_arg) {
                return Err(ConsensusError::DuplicateAuthorityKey);
            }
        }

        Ok(Self { authorities: config.authorities })
    }

    /// Canonical commitment preserving deterministic authority rotation order.
    pub fn config_commitment(&self) -> Hash32 {
        poa_config_commitment(&self.authorities)
    }

    /// Return the authority scheduled for a session-local height.
    pub fn expected_authority(&self, height: u64) -> &Authority {
        &self.authorities[(height % self.authorities.len() as u64) as usize]
    }

    /// Produce a seal with the authority scheduled for this height.
    pub fn seal_from_signer(&self, block_hash: Hash32, height: u64, signer: &CommitteeSigner) -> Result<ProofOfAuthoritySeal> {
        let expected = self.expected_authority(height);
        if signer.validator_id() != expected.id {
            return Err(ConsensusError::UnexpectedAuthority {
                expected: expected.id.clone(),
                actual: signer.validator_id().to_owned(),
            });
        }
        if signer.ckb_public_key() != expected.public_key {
            return Err(ConsensusError::SignerKeyMismatch(expected.id.clone()));
        }
        let authority_id = expected.id.clone();
        let signature = signer.sign_ckb_recoverable(poa_seal_digest(&authority_id, block_hash, height));
        Ok(ProofOfAuthoritySeal { block_hash, height, authority_id, signature })
    }

    /// Verify schedule, height, block binding, and signature for a seal.
    pub fn verify_seal(&self, block_hash: Hash32, height: u64, seal: &ProofOfAuthoritySeal) -> Result<()> {
        if seal.block_hash != block_hash {
            return Err(ConsensusError::WrongBlockHash);
        }
        if seal.height != height {
            return Err(ConsensusError::WrongHeight { expected: height, actual: seal.height });
        }
        let expected = self.expected_authority(height);
        if seal.authority_id != expected.id {
            return Err(ConsensusError::UnexpectedAuthority { expected: expected.id.clone(), actual: seal.authority_id.clone() });
        }
        let digest = poa_seal_digest(&seal.authority_id, block_hash, height);
        if recover_public_key(digest, seal.signature).ok() != Some(expected.public_key) {
            return Err(ConsensusError::InvalidSignature(seal.authority_id.clone()));
        }
        Ok(())
    }

    /// Verify a typed seal and finalise its block.
    pub fn finalise_block_with_seal(&self, block: MyelinBlock, seal: ProofOfAuthoritySeal) -> Result<FinalisedProofOfAuthorityBlock> {
        if block.consensus_kind != ConsensusKind::ProofOfAuthority {
            return Err(ConsensusError::WrongEngine {
                expected: ConsensusKind::ProofOfAuthority.as_str(),
                actual: block.consensus_kind.as_str(),
            });
        }
        let block_hash = block.hash();
        self.verify_seal(block_hash, block.number, &seal)?;
        Ok(FinalisedProofOfAuthorityBlock { block, block_hash, seal })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signer(id: &str, seed: u8) -> CommitteeSigner {
        CommitteeSigner::new(id, [seed; 32]).unwrap()
    }

    fn engine() -> ProofOfAuthority {
        let signers = [signer("alice", 1), signer("bob", 2), signer("carol", 3)];
        ProofOfAuthority::new(ProofOfAuthorityConfig {
            authorities: signers
                .iter()
                .map(|signer| Authority { id: signer.validator_id().to_owned(), public_key: signer.ckb_public_key() })
                .collect(),
        })
        .unwrap()
    }

    fn block(height: u64) -> MyelinBlock {
        MyelinBlock {
            version: 1,
            parent_hash: [0; 32],
            number: height,
            timestamp_ms: 1,
            consensus_kind: ConsensusKind::ProofOfAuthority,
            state_root_before: [1; 32],
            state_root_after: [2; 32],
            ordered_cell_tx_commitments: vec![[3; 32]],
            data_commitments: vec![],
            scheduler_commitment: [4; 32],
        }
    }

    #[test]
    fn rotation_and_typed_finality_are_deterministic() {
        let engine = engine();
        assert_eq!(engine.expected_authority(0).id, "alice");
        assert_eq!(engine.expected_authority(1).id, "bob");
        assert_eq!(engine.expected_authority(3).id, "alice");

        let block = block(1);
        let seal = engine.seal_from_signer(block.hash(), block.number, &signer("bob", 2)).unwrap();
        let finalised = engine.finalise_block_with_seal(block.clone(), seal.clone()).unwrap();
        assert_eq!(finalised.block, block);
        assert_eq!(finalised.seal, seal);
    }

    #[test]
    fn rejects_wrong_authority_height_hash_and_signature() {
        let engine = engine();
        let block = block(1);
        assert!(matches!(
            engine.seal_from_signer(block.hash(), block.number, &signer("alice", 1)),
            Err(ConsensusError::UnexpectedAuthority { .. })
        ));

        let seal = engine.seal_from_signer(block.hash(), block.number, &signer("bob", 2)).unwrap();
        assert!(matches!(engine.verify_seal([9; 32], block.number, &seal), Err(ConsensusError::WrongBlockHash)));
        assert!(matches!(engine.verify_seal(block.hash(), block.number + 1, &seal), Err(ConsensusError::WrongHeight { .. })));
        let mut invalid = seal;
        invalid.signature = [0; 65];
        assert!(matches!(engine.verify_seal(block.hash(), block.number, &invalid), Err(ConsensusError::InvalidSignature(_))));
    }

    #[test]
    fn rejects_empty_duplicate_and_aliased_authority_sets() {
        assert!(matches!(
            ProofOfAuthority::new(ProofOfAuthorityConfig { authorities: vec![] }),
            Err(ConsensusError::InvalidConfig(_))
        ));

        let alice = signer("alice", 1);
        let duplicate_id = ProofOfAuthorityConfig {
            authorities: vec![
                Authority { id: "alice".to_owned(), public_key: alice.ckb_public_key() },
                Authority { id: "alice".to_owned(), public_key: signer("bob", 2).ckb_public_key() },
            ],
        };
        assert!(matches!(
            ProofOfAuthority::new(duplicate_id),
            Err(ConsensusError::DuplicateAuthority(id)) if id == "alice"
        ));

        let duplicate_key = ProofOfAuthorityConfig {
            authorities: vec![
                Authority { id: "alice".to_owned(), public_key: alice.ckb_public_key() },
                Authority { id: "alias".to_owned(), public_key: alice.ckb_public_key() },
            ],
        };
        assert_eq!(ProofOfAuthority::new(duplicate_key).unwrap_err(), ConsensusError::DuplicateAuthorityKey);
    }
}
