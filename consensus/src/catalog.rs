use super::{ConsensusConfig, ConsensusKind, Hash32, Result, SelectedConsensus};

const MODULE_COMMITMENT_DOMAIN: &[u8] = b"myelin:consensus-module";
const SCHEMA_COMMITMENT_DOMAIN: &[u8] = b"myelin:consensus-schema";

/// Protocol version shared by compiled-in finality module descriptors.
pub const CONSENSUS_MODULE_PROTOCOL_VERSION: u32 = 1;

/// Version used by module-owned consensus driver messages.
pub const CONSENSUS_MESSAGE_FORMAT_VERSION: u32 = 1;

/// Owner-encoded consensus message handed to a module-neutral transport.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodedConsensusMessage {
    /// Exact module message format version.
    pub format_version: u32,
    /// Owner-defined type tag.
    pub type_tag: u32,
    /// Canonical bounded payload.
    pub payload: Vec<u8>,
}

const CAPABILITY_WEIGHTED_CERTIFICATE: u32 = 1 << 0;
const CAPABILITY_SCHEDULED_AUTHORITY: u32 = 1 << 1;
const CAPABILITY_ROUND_DRIVER: u32 = 1 << 2;

/// Immutable identity of one compiled-in finality module and configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsensusModuleDescriptor {
    /// Stable canonical module name.
    pub module_name: &'static str,
    /// Module protocol version.
    pub module_protocol_version: u32,
    /// Consensus kind committed by blocks and proofs.
    pub consensus_kind: ConsensusKind,
    /// Owner-defined finality proof schema commitment.
    pub proof_schema_hash: Hash32,
    /// Owner-defined driver message schema commitment.
    pub message_schema_hash: Hash32,
    /// Owner-defined durable round-state schema commitment.
    pub wal_schema_hash: Hash32,
    /// Exact validator/authority ordering and quorum commitment.
    pub config_commitment: Hash32,
    /// Audited capability bitmap.
    pub capabilities: u32,
}

impl ConsensusModuleDescriptor {
    /// Canonical commitment written into session genesis and reused by WAL and
    /// network bindings. Rust type names and process/runtime details are absent.
    pub fn commitment(&self) -> Hash32 {
        let mut hasher = blake3::Hasher::new();
        hasher.update(MODULE_COMMITMENT_DOMAIN);
        put_field(&mut hasher, self.module_name.as_bytes());
        hasher.update(&self.module_protocol_version.to_le_bytes());
        put_field(&mut hasher, self.consensus_kind.as_str().as_bytes());
        hasher.update(&self.proof_schema_hash);
        hasher.update(&self.message_schema_hash);
        hasher.update(&self.wal_schema_hash);
        hasher.update(&self.config_commitment);
        hasher.update(&self.capabilities.to_le_bytes());
        *hasher.finalize().as_bytes()
    }
}

/// Closed catalog of audited modules compiled into this Myelin build.
#[derive(Clone, Copy, Debug, Default)]
pub struct ConsensusCatalog;

impl ConsensusCatalog {
    /// Canonical kinds registered in this build.
    pub const fn registered_kinds() -> &'static [ConsensusKind] {
        &[ConsensusKind::StaticClosedCommittee, ConsensusKind::ProofOfAuthority, ConsensusKind::Tendermint]
    }

    /// Construct one registered module from strict validated config.
    pub fn build(config: ConsensusConfig) -> Result<SelectedConsensus> {
        SelectedConsensus::from_config(config)
    }

    /// Construct the descriptor for a registered kind and exact config.
    pub fn descriptor(kind: ConsensusKind, config_commitment: Hash32) -> ConsensusModuleDescriptor {
        let (module_name, capabilities) = match kind {
            ConsensusKind::StaticClosedCommittee => ("static-closed-committee", CAPABILITY_WEIGHTED_CERTIFICATE),
            ConsensusKind::ProofOfAuthority => ("proof-of-authority", CAPABILITY_SCHEDULED_AUTHORITY),
            ConsensusKind::Tendermint => ("tendermint", CAPABILITY_WEIGHTED_CERTIFICATE | CAPABILITY_ROUND_DRIVER),
        };
        ConsensusModuleDescriptor {
            module_name,
            module_protocol_version: CONSENSUS_MODULE_PROTOCOL_VERSION,
            consensus_kind: kind,
            proof_schema_hash: proof_schema_hash(kind),
            message_schema_hash: message_schema_hash(kind),
            wal_schema_hash: wal_schema_hash(kind),
            config_commitment,
            capabilities,
        }
    }
}

pub(crate) fn proof_schema_hash(kind: ConsensusKind) -> Hash32 {
    let label = match kind {
        ConsensusKind::StaticClosedCommittee => b"static-closed-committee-proof-v1".as_slice(),
        ConsensusKind::ProofOfAuthority => b"proof-of-authority-proof-v1".as_slice(),
        ConsensusKind::Tendermint => b"tendermint-decision-proof-v1".as_slice(),
    };
    schema_hash(b"proof", label)
}

fn message_schema_hash(kind: ConsensusKind) -> Hash32 {
    let label = match kind {
        ConsensusKind::StaticClosedCommittee => b"static-closed-committee-driver-message-v1".as_slice(),
        ConsensusKind::ProofOfAuthority => b"proof-of-authority-driver-message-v1".as_slice(),
        ConsensusKind::Tendermint => b"tendermint-proposal-vote-message-v1".as_slice(),
    };
    schema_hash(b"message", label)
}

fn wal_schema_hash(kind: ConsensusKind) -> Hash32 {
    let label = match kind {
        ConsensusKind::StaticClosedCommittee => b"static-closed-committee-wal-v1".as_slice(),
        ConsensusKind::ProofOfAuthority => b"proof-of-authority-wal-v1".as_slice(),
        ConsensusKind::Tendermint => b"tendermint-round-state-wal-v1".as_slice(),
    };
    schema_hash(b"wal", label)
}

fn schema_hash(class: &[u8], label: &[u8]) -> Hash32 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(SCHEMA_COMMITMENT_DOMAIN);
    put_field(&mut hasher, class);
    put_field(&mut hasher, label);
    *hasher.finalize().as_bytes()
}

fn put_field(hasher: &mut blake3::Hasher, value: &[u8]) {
    hasher.update(&(value.len() as u64).to_le_bytes());
    hasher.update(value);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_closed_unique_and_descriptor_is_mutation_sensitive() {
        let kinds = ConsensusCatalog::registered_kinds();
        assert_eq!(kinds.len(), 3);
        assert_ne!(kinds[0], kinds[1]);
        assert_ne!(kinds[0], kinds[2]);
        assert_ne!(kinds[1], kinds[2]);

        let left = ConsensusCatalog::descriptor(ConsensusKind::ProofOfAuthority, [7; 32]);
        let same = ConsensusCatalog::descriptor(ConsensusKind::ProofOfAuthority, [7; 32]);
        let changed_config = ConsensusCatalog::descriptor(ConsensusKind::ProofOfAuthority, [8; 32]);
        let changed_kind = ConsensusCatalog::descriptor(ConsensusKind::Tendermint, [7; 32]);
        assert_eq!(left.commitment(), same.commitment());
        assert_ne!(left.commitment(), changed_config.commitment());
        assert_ne!(left.commitment(), changed_kind.commitment());
    }
}
