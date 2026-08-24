// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers

//! Provider-neutral data-availability certificates.
//!
//! This module deliberately owns no provider or auditor private keys. Storage
//! providers sign durable-storage receipts and independent auditors sign
//! retrieval probes outside the runtime. Myelin only verifies the resulting
//! certificate against a deterministic policy.

use secp256k1::{schnorr::Signature, Message, Secp256k1, XOnlyPublicKey};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

pub type Hash32 = [u8; 32];
pub type Signature64 = Vec<u8>;

const BLOB_ID_DOMAIN: &[u8] = b"myelin:da-blob";
const RECEIPT_DOMAIN: &[u8] = b"myelin:da-provider-receipt";
const RECEIPT_HASH_DOMAIN: &[u8] = b"myelin:da-provider-receipt-hash";
const PROBE_DOMAIN: &[u8] = b"myelin:da-retrieval-probe";
const CERTIFICATE_DOMAIN: &[u8] = b"myelin:da-certificate";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaErasureProfile {
    pub data_shards: u16,
    pub parity_shards: u16,
    pub shard_size: u32,
    pub shard_root: Hash32,
}

impl DaErasureProfile {
    pub fn replicated(payload_len: u64, payload_hash: Hash32) -> Result<Self, DaError> {
        let shard_size = u32::try_from(payload_len).map_err(|_| DaError::InvalidBlob("payload exceeds u32 shard size".to_owned()))?;
        Ok(Self { data_shards: 1, parity_shards: 0, shard_size, shard_root: payload_hash })
    }

    fn validate(&self) -> Result<(), DaError> {
        if self.data_shards == 0 {
            return Err(DaError::InvalidBlob("data_shards must be non-zero".to_owned()));
        }
        if self.shard_size == 0 {
            return Err(DaError::InvalidBlob("shard_size must be non-zero".to_owned()));
        }
        self.data_shards.checked_add(self.parity_shards).ok_or_else(|| DaError::InvalidBlob("shard count overflow".to_owned()))?;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaBlobCommitment {
    pub namespace: Hash32,
    pub session_id: Hash32,
    pub chunk_index: u64,
    pub payload_hash: Hash32,
    pub payload_len: u64,
    pub segment_root: Hash32,
    pub erasure: DaErasureProfile,
}

impl DaBlobCommitment {
    pub fn validate(&self) -> Result<(), DaError> {
        if self.payload_len == 0 {
            return Err(DaError::InvalidBlob("payload_len must be non-zero".to_owned()));
        }
        self.erasure.validate()
    }

    pub fn blob_id(&self) -> Hash32 {
        hash_fields(
            BLOB_ID_DOMAIN,
            &[
                &self.namespace,
                &self.session_id,
                &self.chunk_index.to_le_bytes(),
                &self.payload_hash,
                &self.payload_len.to_le_bytes(),
                &self.segment_root,
                &self.erasure.data_shards.to_le_bytes(),
                &self.erasure.parity_shards.to_le_bytes(),
                &self.erasure.shard_size.to_le_bytes(),
                &self.erasure.shard_root,
            ],
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaPolicy {
    pub min_distinct_providers: u16,
    pub min_distinct_fault_domains: u16,
    pub min_retention_epochs: u64,
    pub min_successful_probes: u16,
    pub min_probed_providers: u16,
    pub max_probe_age_epochs: u64,
    pub max_probe_latency_ms: u32,
    pub trusted_auditors: Vec<Hash32>,
}

impl DaPolicy {
    pub fn validate(&self) -> Result<(), DaError> {
        if self.min_distinct_providers == 0 || self.min_distinct_fault_domains == 0 {
            return Err(DaError::InvalidPolicy("provider and fault-domain quorum must be non-zero".to_owned()));
        }
        if self.min_distinct_fault_domains > self.min_distinct_providers {
            return Err(DaError::InvalidPolicy("fault-domain quorum exceeds provider quorum".to_owned()));
        }
        if self.min_retention_epochs == 0 {
            return Err(DaError::InvalidPolicy("retention must be non-zero".to_owned()));
        }
        if self.min_successful_probes == 0 || self.min_probed_providers == 0 {
            return Err(DaError::InvalidPolicy("retrieval-probe quorum must be non-zero".to_owned()));
        }
        if self.min_probed_providers > self.min_distinct_providers {
            return Err(DaError::InvalidPolicy("probed-provider quorum exceeds provider quorum".to_owned()));
        }
        if self.max_probe_age_epochs == 0 || self.max_probe_latency_ms == 0 {
            return Err(DaError::InvalidPolicy("probe age and latency bounds must be non-zero".to_owned()));
        }
        if self.trusted_auditors.is_empty() {
            return Err(DaError::InvalidPolicy("at least one trusted auditor is required".to_owned()));
        }
        if self.trusted_auditors.iter().copied().collect::<HashSet<_>>().len() != self.trusted_auditors.len() {
            return Err(DaError::InvalidPolicy("trusted auditors must be unique".to_owned()));
        }
        for auditor in &self.trusted_auditors {
            parse_public_key(*auditor).map_err(|_| DaError::InvalidPolicy("invalid trusted auditor key".to_owned()))?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaProviderReceipt {
    pub blob_id: Hash32,
    pub provider_id: String,
    pub fault_domain: String,
    pub retrieval_endpoint: String,
    pub stored_at_epoch: u64,
    pub retained_until_epoch: u64,
    pub provider_public_key: Hash32,
    pub signature: Signature64,
}

impl DaProviderReceipt {
    pub fn signing_digest(&self) -> Hash32 {
        hash_fields(
            RECEIPT_DOMAIN,
            &[
                &self.blob_id,
                self.provider_id.as_bytes(),
                self.fault_domain.as_bytes(),
                self.retrieval_endpoint.as_bytes(),
                &self.stored_at_epoch.to_le_bytes(),
                &self.retained_until_epoch.to_le_bytes(),
                &self.provider_public_key,
            ],
        )
    }

    pub fn receipt_hash(&self) -> Hash32 {
        hash_fields(RECEIPT_HASH_DOMAIN, &[&self.signing_digest(), &self.signature])
    }

    pub fn verify_signature(&self) -> bool {
        verify_schnorr(self.provider_public_key, self.signing_digest(), &self.signature)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaRetrievalProbe {
    pub blob_id: Hash32,
    pub provider_id: String,
    pub receipt_hash: Hash32,
    pub sample_index: u32,
    pub sample_hash: Hash32,
    pub sample_proof_hash: Hash32,
    pub observed_at_epoch: u64,
    pub latency_ms: u32,
    pub successful: bool,
    pub auditor_public_key: Hash32,
    pub signature: Signature64,
}

impl DaRetrievalProbe {
    pub fn signing_digest(&self) -> Hash32 {
        hash_fields(
            PROBE_DOMAIN,
            &[
                &self.blob_id,
                self.provider_id.as_bytes(),
                &self.receipt_hash,
                &self.sample_index.to_le_bytes(),
                &self.sample_hash,
                &self.sample_proof_hash,
                &self.observed_at_epoch.to_le_bytes(),
                &self.latency_ms.to_le_bytes(),
                &[u8::from(self.successful)],
                &self.auditor_public_key,
            ],
        )
    }

    pub fn verify_signature(&self) -> bool {
        verify_schnorr(self.auditor_public_key, self.signing_digest(), &self.signature)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaCertificate {
    pub blob: DaBlobCommitment,
    pub policy: DaPolicy,
    pub receipts: Vec<DaProviderReceipt>,
    pub probes: Vec<DaRetrievalProbe>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaCertificateVerification {
    pub blob_id: Hash32,
    pub certificate_commitment: Hash32,
    pub provider_count: u16,
    pub fault_domain_count: u16,
    pub successful_probe_count: u16,
    pub probed_provider_count: u16,
    pub evaluation_epoch: u64,
}

impl DaCertificate {
    pub fn commitment(&self) -> Hash32 {
        let blob_id = self.blob.blob_id();
        let mut receipt_hashes = self.receipts.iter().map(DaProviderReceipt::receipt_hash).collect::<Vec<_>>();
        receipt_hashes.sort_unstable();
        let mut probe_hashes = self
            .probes
            .iter()
            .map(|probe| hash_fields(PROBE_DOMAIN, &[&probe.signing_digest(), &probe.signature]))
            .collect::<Vec<_>>();
        probe_hashes.sort_unstable();
        let mut hasher = blake3::Hasher::new();
        hasher.update(CERTIFICATE_DOMAIN);
        hasher.update(&blob_id);
        hash_policy_into(&mut hasher, &self.policy);
        for hash in receipt_hashes {
            hasher.update(&hash);
        }
        for hash in probe_hashes {
            hasher.update(&hash);
        }
        *hasher.finalize().as_bytes()
    }

    pub fn verify(&self, evaluation_epoch: u64) -> Result<DaCertificateVerification, DaError> {
        self.blob.validate()?;
        self.policy.validate()?;
        let blob_id = self.blob.blob_id();
        let required_until = evaluation_epoch
            .checked_add(self.policy.min_retention_epochs)
            .ok_or_else(|| DaError::InvalidPolicy("retention epoch overflow".to_owned()))?;
        let mut providers = HashSet::new();
        let mut fault_domains = HashSet::new();
        let mut receipts = HashMap::new();
        for receipt in &self.receipts {
            if receipt.blob_id != blob_id {
                return Err(DaError::WrongBlob(receipt.provider_id.clone()));
            }
            if receipt.provider_id.is_empty() || receipt.fault_domain.is_empty() || receipt.retrieval_endpoint.is_empty() {
                return Err(DaError::InvalidReceipt(receipt.provider_id.clone()));
            }
            if !providers.insert(receipt.provider_id.as_str()) {
                return Err(DaError::DuplicateProvider(receipt.provider_id.clone()));
            }
            if receipt.stored_at_epoch > evaluation_epoch || receipt.retained_until_epoch < required_until {
                return Err(DaError::InsufficientRetention(receipt.provider_id.clone()));
            }
            if !receipt.verify_signature() {
                return Err(DaError::InvalidReceiptSignature(receipt.provider_id.clone()));
            }
            fault_domains.insert(receipt.fault_domain.as_str());
            receipts.insert(receipt.provider_id.as_str(), receipt.receipt_hash());
        }
        if providers.len() < usize::from(self.policy.min_distinct_providers) {
            return Err(DaError::ProviderQuorumNotMet { actual: providers.len(), required: self.policy.min_distinct_providers });
        }
        if fault_domains.len() < usize::from(self.policy.min_distinct_fault_domains) {
            return Err(DaError::FaultDomainQuorumNotMet {
                actual: fault_domains.len(),
                required: self.policy.min_distinct_fault_domains,
            });
        }

        let trusted_auditors = self.policy.trusted_auditors.iter().copied().collect::<HashSet<_>>();
        let mut seen_probes = HashSet::new();
        let mut successful_probes = 0usize;
        let mut probed_providers = HashSet::new();
        for probe in &self.probes {
            if probe.blob_id != blob_id {
                return Err(DaError::WrongBlob(probe.provider_id.clone()));
            }
            let Some(receipt_hash) = receipts.get(probe.provider_id.as_str()) else {
                return Err(DaError::UnknownProbeProvider(probe.provider_id.clone()));
            };
            if receipt_hash != &probe.receipt_hash {
                return Err(DaError::WrongReceipt(probe.provider_id.clone()));
            }
            if !trusted_auditors.contains(&probe.auditor_public_key) || !probe.verify_signature() {
                return Err(DaError::InvalidProbeSignature(probe.provider_id.clone()));
            }
            let probe_key = (probe.provider_id.as_str(), probe.sample_index, probe.auditor_public_key);
            if !seen_probes.insert(probe_key) {
                return Err(DaError::DuplicateProbe(probe.provider_id.clone()));
            }
            let probe_age = evaluation_epoch
                .checked_sub(probe.observed_at_epoch)
                .ok_or_else(|| DaError::ProbeFromFuture(probe.provider_id.clone()))?;
            if probe_age > self.policy.max_probe_age_epochs {
                return Err(DaError::StaleProbe(probe.provider_id.clone()));
            }
            if probe.successful && probe.latency_ms <= self.policy.max_probe_latency_ms {
                successful_probes += 1;
                probed_providers.insert(probe.provider_id.as_str());
            }
        }
        if successful_probes < usize::from(self.policy.min_successful_probes) {
            return Err(DaError::ProbeQuorumNotMet { actual: successful_probes, required: self.policy.min_successful_probes });
        }
        if probed_providers.len() < usize::from(self.policy.min_probed_providers) {
            return Err(DaError::ProbedProviderQuorumNotMet {
                actual: probed_providers.len(),
                required: self.policy.min_probed_providers,
            });
        }

        Ok(DaCertificateVerification {
            blob_id,
            certificate_commitment: self.commitment(),
            provider_count: u16::try_from(providers.len()).unwrap_or(u16::MAX),
            fault_domain_count: u16::try_from(fault_domains.len()).unwrap_or(u16::MAX),
            successful_probe_count: u16::try_from(successful_probes).unwrap_or(u16::MAX),
            probed_provider_count: u16::try_from(probed_providers.len()).unwrap_or(u16::MAX),
            evaluation_epoch,
        })
    }
}

fn hash_policy_into(hasher: &mut blake3::Hasher, policy: &DaPolicy) {
    hasher.update(&policy.min_distinct_providers.to_le_bytes());
    hasher.update(&policy.min_distinct_fault_domains.to_le_bytes());
    hasher.update(&policy.min_retention_epochs.to_le_bytes());
    hasher.update(&policy.min_successful_probes.to_le_bytes());
    hasher.update(&policy.min_probed_providers.to_le_bytes());
    hasher.update(&policy.max_probe_age_epochs.to_le_bytes());
    hasher.update(&policy.max_probe_latency_ms.to_le_bytes());
    let mut auditors = policy.trusted_auditors.clone();
    auditors.sort_unstable();
    for auditor in auditors {
        hasher.update(&auditor);
    }
}

fn hash_fields(domain: &[u8], fields: &[&[u8]]) -> Hash32 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain);
    for field in fields {
        hasher.update(&(field.len() as u64).to_le_bytes());
        hasher.update(field);
    }
    *hasher.finalize().as_bytes()
}

fn parse_public_key(public_key: Hash32) -> Result<XOnlyPublicKey, secp256k1::Error> {
    XOnlyPublicKey::from_slice(&public_key)
}

fn verify_schnorr(public_key: Hash32, digest: Hash32, signature: &[u8]) -> bool {
    let Ok(public_key) = parse_public_key(public_key) else {
        return false;
    };
    let Ok(signature) = Signature::from_slice(signature) else {
        return false;
    };
    Secp256k1::verification_only().verify_schnorr(&signature, &Message::from_digest(digest), &public_key).is_ok()
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DaError {
    #[error("invalid DA blob: {0}")]
    InvalidBlob(String),
    #[error("invalid DA policy: {0}")]
    InvalidPolicy(String),
    #[error("invalid provider receipt: {0}")]
    InvalidReceipt(String),
    #[error("DA evidence for {0} is bound to another blob")]
    WrongBlob(String),
    #[error("duplicate DA provider: {0}")]
    DuplicateProvider(String),
    #[error("provider {0} does not satisfy the retention policy")]
    InsufficientRetention(String),
    #[error("invalid provider receipt signature: {0}")]
    InvalidReceiptSignature(String),
    #[error("provider quorum not met: got {actual}, require {required}")]
    ProviderQuorumNotMet { actual: usize, required: u16 },
    #[error("fault-domain quorum not met: got {actual}, require {required}")]
    FaultDomainQuorumNotMet { actual: usize, required: u16 },
    #[error("retrieval probe refers to an unknown provider: {0}")]
    UnknownProbeProvider(String),
    #[error("retrieval probe refers to the wrong provider receipt: {0}")]
    WrongReceipt(String),
    #[error("invalid retrieval probe signature: {0}")]
    InvalidProbeSignature(String),
    #[error("duplicate retrieval probe: {0}")]
    DuplicateProbe(String),
    #[error("retrieval probe is from the future: {0}")]
    ProbeFromFuture(String),
    #[error("retrieval probe is stale: {0}")]
    StaleProbe(String),
    #[error("retrieval-probe quorum not met: got {actual}, require {required}")]
    ProbeQuorumNotMet { actual: usize, required: u16 },
    #[error("probed-provider quorum not met: got {actual}, require {required}")]
    ProbedProviderQuorumNotMet { actual: usize, required: u16 },
}

#[cfg(test)]
mod tests {
    use super::*;
    use secp256k1::{Keypair, SecretKey};

    fn key(seed: u8) -> (SecretKey, Hash32) {
        let secret = SecretKey::from_slice(&[seed; 32]).unwrap();
        let keypair = Keypair::from_secret_key(&Secp256k1::new(), &secret);
        (secret, XOnlyPublicKey::from_keypair(&keypair).0.serialize())
    }

    fn sign(secret: &SecretKey, digest: Hash32) -> Signature64 {
        let secp = Secp256k1::new();
        let keypair = Keypair::from_secret_key(&secp, secret);
        secp.sign_schnorr_no_aux_rand(&Message::from_digest(digest), &keypair).serialize().to_vec()
    }

    fn certificate() -> DaCertificate {
        let blob = DaBlobCommitment {
            namespace: [1; 32],
            session_id: [2; 32],
            chunk_index: 7,
            payload_hash: [3; 32],
            payload_len: 4096,
            segment_root: [4; 32],
            erasure: DaErasureProfile { data_shards: 4, parity_shards: 2, shard_size: 1024, shard_root: [5; 32] },
        };
        let blob_id = blob.blob_id();
        let (auditor_secret, auditor_public_key) = key(9);
        let mut receipts = Vec::new();
        let mut probes = Vec::new();
        for (index, (provider_id, fault_domain, seed)) in
            [("provider-a", "region-a", 1), ("provider-b", "region-b", 2)].into_iter().enumerate()
        {
            let (provider_secret, provider_public_key) = key(seed);
            let mut receipt = DaProviderReceipt {
                blob_id,
                provider_id: provider_id.to_owned(),
                fault_domain: fault_domain.to_owned(),
                retrieval_endpoint: format!("https://{provider_id}.example/blob"),
                stored_at_epoch: 90,
                retained_until_epoch: 200,
                provider_public_key,
                signature: vec![0; 64],
            };
            receipt.signature = sign(&provider_secret, receipt.signing_digest());
            let mut probe = DaRetrievalProbe {
                blob_id,
                provider_id: provider_id.to_owned(),
                receipt_hash: receipt.receipt_hash(),
                sample_index: index as u32,
                sample_hash: [seed; 32],
                sample_proof_hash: [seed + 10; 32],
                observed_at_epoch: 99,
                latency_ms: 100,
                successful: true,
                auditor_public_key,
                signature: vec![0; 64],
            };
            probe.signature = sign(&auditor_secret, probe.signing_digest());
            receipts.push(receipt);
            probes.push(probe);
        }
        DaCertificate {
            blob,
            policy: DaPolicy {
                min_distinct_providers: 2,
                min_distinct_fault_domains: 2,
                min_retention_epochs: 50,
                min_successful_probes: 2,
                min_probed_providers: 2,
                max_probe_age_epochs: 5,
                max_probe_latency_ms: 500,
                trusted_auditors: vec![auditor_public_key],
            },
            receipts,
            probes,
        }
    }

    #[test]
    fn provider_neutral_certificate_verifies_quorum_retention_and_probes() {
        let certificate = certificate();
        let verified = certificate.verify(100).unwrap();
        assert_eq!(verified.blob_id, certificate.blob.blob_id());
        assert_eq!(verified.provider_count, 2);
        assert_eq!(verified.fault_domain_count, 2);
        assert_eq!(verified.successful_probe_count, 2);
        assert_eq!(verified.probed_provider_count, 2);
        assert_eq!(verified.certificate_commitment, certificate.commitment());
    }

    #[test]
    fn receipt_signature_binds_retention_and_blob() {
        let mut certificate = certificate();
        certificate.receipts[0].retained_until_epoch += 1;
        assert!(matches!(certificate.verify(100), Err(DaError::InvalidReceiptSignature(provider)) if provider == "provider-a"));
    }

    #[test]
    fn certificate_rejects_same_fault_domain_and_stale_probe() {
        let mut same_domain = certificate();
        same_domain.receipts[1].fault_domain = same_domain.receipts[0].fault_domain.clone();
        let (provider_secret, _) = key(2);
        same_domain.receipts[1].signature = sign(&provider_secret, same_domain.receipts[1].signing_digest());
        same_domain.probes[1].receipt_hash = same_domain.receipts[1].receipt_hash();
        let (auditor_secret, _) = key(9);
        same_domain.probes[1].signature = sign(&auditor_secret, same_domain.probes[1].signing_digest());
        assert!(matches!(same_domain.verify(100), Err(DaError::FaultDomainQuorumNotMet { .. })));

        let mut stale = certificate();
        stale.probes[0].observed_at_epoch = 90;
        stale.probes[0].signature = sign(&auditor_secret, stale.probes[0].signing_digest());
        assert!(matches!(stale.verify(100), Err(DaError::StaleProbe(provider)) if provider == "provider-a"));
    }

    #[test]
    fn certificate_rejects_untrusted_probe_and_provider_aliases() {
        let mut untrusted = certificate();
        let (attacker_secret, attacker_public_key) = key(8);
        untrusted.probes[0].auditor_public_key = attacker_public_key;
        untrusted.probes[0].signature = sign(&attacker_secret, untrusted.probes[0].signing_digest());
        assert!(matches!(untrusted.verify(100), Err(DaError::InvalidProbeSignature(provider)) if provider == "provider-a"));

        let mut duplicate = certificate();
        duplicate.receipts[1].provider_id = duplicate.receipts[0].provider_id.clone();
        let (provider_secret, _) = key(2);
        duplicate.receipts[1].signature = sign(&provider_secret, duplicate.receipts[1].signing_digest());
        assert!(matches!(duplicate.verify(100), Err(DaError::DuplicateProvider(provider)) if provider == "provider-a"));
    }
}
