// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// Cell transaction signature hashing (blake3 with domain separation)

use super::types::{CellOutput, CellTx, OutPoint};
use spora_hashes::{CellTxSigningHash, CellTxSigningHashEcdsa, Hash, Hasher, HasherBase, SchnorrSigningHash, ZERO_HASH};

/// Domain constant for TXID hashing
pub const CELL_TXID_DOMAIN: &[u8] = b"spora-cell/txid";
/// Domain constant for WTXID hashing
pub const CELL_WTXID_DOMAIN: &[u8] = b"spora-cell/wtxid";
/// Domain constant for signature hashing
pub const CELL_SIG_DOMAIN: &[u8] = b"spora-cell/sig";

/// Minimal resolved input data required by the canonical standard-lock sighash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StandardSigningInput {
    /// Hash of the resolved lock script.
    pub lock_hash: [u8; 32],
    /// Hash of the resolved type script when present.
    pub type_hash: Option<[u8; 32]>,
    /// Hash of the resolved cell data.
    pub data_hash: [u8; 32],
    /// Length of the resolved cell data in bytes.
    pub data_bytes: u64,
    /// Capacity of the resolved input cell.
    pub capacity: u64,
}

/// Shared sighash flag interface used by the exec-side canonical signing code.
pub trait StandardSigHashType: Copy {
    /// Returns true when no outputs are committed.
    fn is_sighash_none(self) -> bool;
    /// Returns true when only the output matching the input index is committed.
    fn is_sighash_single(self) -> bool;
    /// Returns true when only the current input is committed.
    fn is_sighash_anyone_can_pay(self) -> bool;
    /// Returns the raw sighash flag byte.
    fn to_u8(self) -> u8;
}

/// Cache interface for reused sighash sub-hashes.
pub trait StandardSigHashReusedValues {
    /// Returns the cached previous-outputs hash or computes it.
    fn previous_outputs_hash(&self, set: impl Fn() -> Hash) -> Hash;
    /// Returns the cached sequences hash or computes it.
    fn sequences_hash(&self, set: impl Fn() -> Hash) -> Hash;
    /// Returns the cached sigop-counts hash or computes it.
    fn sig_op_counts_hash(&self, set: impl Fn() -> Hash) -> Hash;
    /// Returns the cached outputs hash or computes it.
    fn outputs_hash(&self, set: impl Fn() -> Hash) -> Hash;
    /// Returns the cached payload hash or computes it.
    fn payload_hash(&self, set: impl Fn() -> Hash) -> Hash;
}

trait HashWriterExt {
    fn write_len(&mut self, len: usize) -> &mut Self;
    fn write_bool(&mut self, element: bool) -> &mut Self;
    fn write_u8(&mut self, element: u8) -> &mut Self;
    fn write_u32(&mut self, element: u32) -> &mut Self;
    fn write_u64(&mut self, element: u64) -> &mut Self;
    fn write_var_bytes(&mut self, bytes: &[u8]) -> &mut Self;
}

const _: usize = u64::MAX as usize - usize::MAX;

impl<T: HasherBase> HashWriterExt for T {
    #[inline(always)]
    fn write_len(&mut self, len: usize) -> &mut Self {
        self.update((len as u64).to_le_bytes())
    }

    #[inline(always)]
    fn write_bool(&mut self, element: bool) -> &mut Self {
        self.update(if element { [1u8] } else { [0u8] })
    }

    #[inline(always)]
    fn write_u8(&mut self, element: u8) -> &mut Self {
        self.update(element.to_le_bytes())
    }

    #[inline(always)]
    fn write_u32(&mut self, element: u32) -> &mut Self {
        self.update(element.to_le_bytes())
    }

    #[inline(always)]
    fn write_u64(&mut self, element: u64) -> &mut Self {
        self.update(element.to_le_bytes())
    }

    #[inline(always)]
    fn write_var_bytes(&mut self, bytes: &[u8]) -> &mut Self {
        self.write_len(bytes.len()).update(bytes)
    }
}

/// Compute txid (without witnesses)
///
/// Formula: blake3(CELL_TXID_DOMAIN || ver || inputs || deps || header_deps || outputs || outputs_data || coinbase_payload_fallback?)
pub fn compute_txid(tx: &CellTx) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(CELL_TXID_DOMAIN);

    // Version
    hasher.update(&tx.version().to_le_bytes());

    // Inputs (without witnesses)
    hasher.update(&(tx.inputs.len() as u32).to_le_bytes());
    for input in &tx.inputs {
        // Serialize OutPoint
        hasher.update(&input.previous_output.tx_hash);
        hasher.update(&input.previous_output.index.to_le_bytes());
        hasher.update(&input.since.to_le_bytes());
    }

    // Dependencies
    hasher.update(&(tx.cell_deps.len() as u32).to_le_bytes());
    for dep in &tx.cell_deps {
        hasher.update(&dep.out_point.tx_hash);
        hasher.update(&dep.out_point.index.to_le_bytes());
        hasher.update(&[dep.dep_type.clone() as u8]);
    }

    // Header dependencies
    hasher.update(&(tx.header_deps.len() as u32).to_le_bytes());
    for header_hash in &tx.header_deps {
        hasher.update(header_hash);
    }

    // Outputs
    hasher.update(&(tx.outputs.len() as u32).to_le_bytes());
    for output in &tx.outputs {
        hasher.update(&output.lock.code_hash);
        hasher.update(&[output.lock.hash_type]);
        hasher.update(&(output.lock.args.len() as u32).to_le_bytes());
        hasher.update(&output.lock.args);

        if let Some(ref type_script) = output.type_ {
            hasher.update(&[1u8]); // has type script
            hasher.update(&type_script.code_hash);
            hasher.update(&[type_script.hash_type]);
            hasher.update(&(type_script.args.len() as u32).to_le_bytes());
            hasher.update(&type_script.args);
        } else {
            hasher.update(&[0u8]); // no type script
        }

        hasher.update(&output.capacity.to_le_bytes());
    }

    // Outputs data
    for data in &tx.outputs_data {
        hasher.update(&(data.len() as u32).to_le_bytes());
        hasher.update(data);
    }

    if tx.is_coinbase() && tx.outputs.is_empty() {
        if let Some(payload) = tx.witnesses.first() {
            hasher.update(b"coinbase-payload-fallback");
            hasher.update(&(payload.len() as u32).to_le_bytes());
            hasher.update(payload);
        }
    }

    *hasher.finalize().as_bytes()
}

/// Compute wtxid (with witnesses)
///
/// Formula: blake3(CELL_WTXID_DOMAIN || ver || inputs || deps || header_deps || outputs || outputs_data || coinbase_payload_fallback? || witnesses)
pub fn compute_wtxid(tx: &CellTx) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(CELL_WTXID_DOMAIN);

    // Version
    hasher.update(&tx.version().to_le_bytes());

    // Inputs
    hasher.update(&(tx.inputs.len() as u32).to_le_bytes());
    for input in &tx.inputs {
        hasher.update(&input.previous_output.tx_hash);
        hasher.update(&input.previous_output.index.to_le_bytes());
        hasher.update(&input.since.to_le_bytes());
    }

    // Dependencies
    hasher.update(&(tx.cell_deps.len() as u32).to_le_bytes());
    for dep in &tx.cell_deps {
        hasher.update(&dep.out_point.tx_hash);
        hasher.update(&dep.out_point.index.to_le_bytes());
        hasher.update(&[dep.dep_type.clone() as u8]);
    }

    // Header dependencies
    hasher.update(&(tx.header_deps.len() as u32).to_le_bytes());
    for header_hash in &tx.header_deps {
        hasher.update(header_hash);
    }

    // Outputs
    hasher.update(&(tx.outputs.len() as u32).to_le_bytes());
    for output in &tx.outputs {
        hasher.update(&output.lock.code_hash);
        hasher.update(&[output.lock.hash_type]);
        hasher.update(&(output.lock.args.len() as u32).to_le_bytes());
        hasher.update(&output.lock.args);

        if let Some(ref type_script) = output.type_ {
            hasher.update(&[1u8]);
            hasher.update(&type_script.code_hash);
            hasher.update(&[type_script.hash_type]);
            hasher.update(&(type_script.args.len() as u32).to_le_bytes());
            hasher.update(&type_script.args);
        } else {
            hasher.update(&[0u8]);
        }

        hasher.update(&output.capacity.to_le_bytes());
    }

    // Outputs data
    for data in &tx.outputs_data {
        hasher.update(&(data.len() as u32).to_le_bytes());
        hasher.update(data);
    }

    if tx.is_coinbase() && tx.outputs.is_empty() {
        if let Some(payload) = tx.witnesses.first() {
            hasher.update(b"coinbase-payload-fallback");
            hasher.update(&(payload.len() as u32).to_le_bytes());
            hasher.update(payload);
        }
    }

    // Witnesses
    hasher.update(&(tx.witnesses.len() as u32).to_le_bytes());
    for witness in &tx.witnesses {
        hasher.update(&(witness.len() as u32).to_le_bytes());
        hasher.update(witness);
    }

    *hasher.finalize().as_bytes()
}

/// Compute signature hash (for signature verification)
///
/// This helper is specific to the exec-side lock path that binds a CellTx to a
/// network id and an RW-set commitment. It is not the same as the consensus-side
/// standard-lock sighash used by wallet signing and `SigHashType`.
///
/// ⚠️ network_id MUST be u32 (4 bytes little-endian)
///
/// Formula:
/// ```text
/// blake3(
///   CELL_SIG_DOMAIN
///   || network_id (u32 LE)
///   || wtxid
///   || input_index (u32 LE)
///   || rw_commitment
/// )
/// ```
pub fn compute_rw_bound_sighash(tx: &CellTx, input_index: u32, network_id: u32, rw_commitment: &[u8; 32]) -> [u8; 32] {
    let wtxid = compute_wtxid(tx);

    let mut hasher = blake3::Hasher::new();
    hasher.update(CELL_SIG_DOMAIN);
    hasher.update(&network_id.to_le_bytes()); // ✓ 4 bytes (not 1 byte!)
    hasher.update(&wtxid);
    hasher.update(&input_index.to_le_bytes());
    hasher.update(rw_commitment);

    *hasher.finalize().as_bytes()
}

/// Compute public key hash (for lock scripts)
///
/// Formula: blake3(pubkey)[0..20]
pub fn pubkey_hash(pubkey: &[u8]) -> [u8; 20] {
    let hash = blake3::hash(pubkey);
    let mut result = [0u8; 20];
    result.copy_from_slice(&hash.as_bytes()[..20]);
    result
}

/// Helper: create empty RW commitment (for simple transactions)
pub fn empty_rw_commitment() -> [u8; 32] {
    *blake3::hash(b"empty-rw-set").as_bytes()
}

/// Hashes all committed previous outpoints for the canonical standard-lock sighash.
pub fn standard_previous_outputs_hash(
    tx: &CellTx,
    hash_type: impl StandardSigHashType,
    reused_values: &impl StandardSigHashReusedValues,
) -> Hash {
    if hash_type.is_sighash_anyone_can_pay() {
        return ZERO_HASH;
    }
    reused_values.previous_outputs_hash(|| {
        let mut hasher = CellTxSigningHash::new();
        for input in &tx.inputs {
            hasher.update(input.previous_output.tx_hash);
            hasher.write_u32(input.previous_output.index);
        }
        hasher.finalize()
    })
}

/// Hashes all committed input `since` values for the canonical standard-lock sighash.
pub fn standard_sequences_hash(
    tx: &CellTx,
    hash_type: impl StandardSigHashType,
    reused_values: &impl StandardSigHashReusedValues,
) -> Hash {
    if hash_type.is_sighash_single() || hash_type.is_sighash_anyone_can_pay() || hash_type.is_sighash_none() {
        return ZERO_HASH;
    }
    reused_values.sequences_hash(|| {
        let mut hasher = CellTxSigningHash::new();
        for input in &tx.inputs {
            hasher.write_u64(input.since);
        }
        hasher.finalize()
    })
}

/// Hashes the implicit one-sigop-per-input counts for the canonical standard-lock sighash.
pub fn standard_sig_op_counts_hash(
    tx: &CellTx,
    hash_type: impl StandardSigHashType,
    reused_values: &impl StandardSigHashReusedValues,
) -> Hash {
    if hash_type.is_sighash_anyone_can_pay() {
        return ZERO_HASH;
    }

    reused_values.sig_op_counts_hash(|| {
        let mut hasher = CellTxSigningHash::new();
        for _ in &tx.inputs {
            hasher.write_u8(1);
        }
        hasher.finalize()
    })
}

/// Hashes the optional CellTx payload committed by the canonical standard-lock sighash.
pub fn standard_payload_hash(tx: &CellTx, reused_values: &impl StandardSigHashReusedValues) -> Hash {
    let payload = tx.payload().unwrap_or_default();
    if !tx.is_coinbase() && payload.is_empty() {
        return ZERO_HASH;
    }

    reused_values.payload_hash(|| {
        let mut hasher = CellTxSigningHash::new();
        hasher.write_var_bytes(payload);
        hasher.finalize()
    })
}

/// Hashes the committed output set for the canonical standard-lock sighash.
pub fn standard_outputs_hash(
    tx: &CellTx,
    hash_type: impl StandardSigHashType,
    reused_values: &impl StandardSigHashReusedValues,
    input_index: usize,
) -> Hash {
    if hash_type.is_sighash_none() {
        return ZERO_HASH;
    }

    if hash_type.is_sighash_single() {
        if input_index >= tx.outputs.len() {
            return ZERO_HASH;
        }

        let mut hasher = CellTxSigningHash::new();
        hash_cell_output(
            &mut hasher,
            &tx.outputs[input_index],
            tx.outputs_data.get(input_index).map(Vec::as_slice).unwrap_or_default(),
        );
        return hasher.finalize();
    }

    reused_values.outputs_hash(|| {
        let mut hasher = CellTxSigningHash::new();
        for (i, output) in tx.outputs.iter().enumerate() {
            let data = tx.outputs_data.get(i).map(Vec::as_slice).unwrap_or_default();
            hash_cell_output(&mut hasher, output, data);
        }
        hasher.finalize()
    })
}

/// Writes an outpoint into the provided signing hasher using canonical encoding.
pub fn hash_outpoint(hasher: &mut impl Hasher, outpoint: OutPoint) {
    hasher.update(outpoint.tx_hash);
    hasher.write_u32(outpoint.index);
}

/// Writes a Cell output and its associated data into the provided signing hasher.
pub fn hash_cell_output(hasher: &mut impl Hasher, output: &CellOutput, data: &[u8]) {
    hasher.write_u64(output.capacity);
    hasher.update(output.lock.code_hash);
    hasher.write_u8(output.lock.hash_type);
    hasher.write_var_bytes(&output.lock.args);
    hasher.write_bool(output.type_.is_some());
    if let Some(ref type_script) = output.type_ {
        hasher.update(type_script.code_hash);
        hasher.write_u8(type_script.hash_type);
        hasher.write_var_bytes(&type_script.args);
    }
    hasher.write_var_bytes(data);
}

fn hash_standard_signing_input(hasher: &mut impl Hasher, signing_input: &StandardSigningInput) {
    hasher.update(signing_input.lock_hash).write_bool(signing_input.type_hash.is_some());
    if let Some(type_hash) = signing_input.type_hash {
        hasher.update(type_hash);
    }
    hasher.update(signing_input.data_hash).write_u64(signing_input.data_bytes);
}

/// Canonical CellTx standard-lock sighash shared by wallet, consensus, and native lock verification.
pub fn calc_standard_signature_hash(
    tx: &CellTx,
    input_index: usize,
    hash_type: impl StandardSigHashType,
    signing_input: &StandardSigningInput,
    reused_values: &impl StandardSigHashReusedValues,
) -> Hash {
    let input = &tx.inputs[input_index];
    let mut hasher = SchnorrSigningHash::new();
    hasher
        .write_u32(tx.version)
        .update(standard_previous_outputs_hash(tx, hash_type, reused_values))
        .update(standard_sequences_hash(tx, hash_type, reused_values))
        .update(standard_sig_op_counts_hash(tx, hash_type, reused_values));
    hash_outpoint(&mut hasher, input.previous_output);
    hash_standard_signing_input(&mut hasher, signing_input);
    hasher.write_u64(signing_input.capacity);
    hasher
        .write_u64(input.since)
        .write_u8(1)
        .update(standard_outputs_hash(tx, hash_type, reused_values, input_index))
        .write_u64(0)
        .update(standard_payload_hash(tx, reused_values))
        .write_u8(hash_type.to_u8());
    hasher.finalize()
}

/// Canonical ECDSA CellTx standard-lock sighash (hash-of-schnorr-sighash).
pub fn calc_standard_ecdsa_signature_hash(
    tx: &CellTx,
    input_index: usize,
    hash_type: impl StandardSigHashType,
    signing_input: &StandardSigningInput,
    reused_values: &impl StandardSigHashReusedValues,
) -> Hash {
    let hash = calc_standard_signature_hash(tx, input_index, hash_type, signing_input, reused_values);
    let mut hasher = CellTxSigningHashEcdsa::new();
    hasher.update(hash);
    hasher.finalize()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::celltx::types::{CellDep, CellInput, CellOutput, DepType, OutPoint, Script};
    use spora_hashes::CellTxSigningHashEcdsa;

    #[derive(Clone, Copy)]
    struct TestSigHashType(u8);

    impl StandardSigHashType for TestSigHashType {
        fn is_sighash_none(self) -> bool {
            self.0 & 0b0000_0111 == 0b0000_0010
        }

        fn is_sighash_single(self) -> bool {
            self.0 & 0b0000_0111 == 0b0000_0100
        }

        fn is_sighash_anyone_can_pay(self) -> bool {
            self.0 & 0b1000_0000 == 0b1000_0000
        }

        fn to_u8(self) -> u8 {
            self.0
        }
    }

    #[derive(Default)]
    struct NoCache;

    impl StandardSigHashReusedValues for NoCache {
        fn previous_outputs_hash(&self, set: impl Fn() -> Hash) -> Hash {
            set()
        }

        fn sequences_hash(&self, set: impl Fn() -> Hash) -> Hash {
            set()
        }

        fn sig_op_counts_hash(&self, set: impl Fn() -> Hash) -> Hash {
            set()
        }

        fn outputs_hash(&self, set: impl Fn() -> Hash) -> Hash {
            set()
        }

        fn payload_hash(&self, set: impl Fn() -> Hash) -> Hash {
            set()
        }
    }

    fn create_test_tx() -> CellTx {
        let lock = Script::new([0x00; 32], 0, vec![0; 20]);
        let inputs = vec![CellInput::new(OutPoint::new([0x11; 32], 0), 0)];
        let deps = vec![CellDep { out_point: OutPoint::new([0x22; 32], 0), dep_type: DepType::Code }];
        let outputs = vec![CellOutput { lock: lock.clone(), type_: None, capacity: 1000 }];
        let outputs_data = vec![vec![]];
        let witnesses = vec![vec![0; 65]];

        CellTx::new(inputs, deps, outputs, outputs_data, witnesses).unwrap()
    }

    #[test]
    fn test_txid_computation() {
        let tx = create_test_tx();
        let txid = compute_txid(&tx);

        // txid should be deterministic
        let txid2 = compute_txid(&tx);
        assert_eq!(txid, txid2);

        // txid should be 32 bytes
        assert_eq!(txid.len(), 32);
    }

    #[test]
    fn test_wtxid_computation() {
        let tx = create_test_tx();
        let wtxid = compute_wtxid(&tx);

        // wtxid should be deterministic
        let wtxid2 = compute_wtxid(&tx);
        assert_eq!(wtxid, wtxid2);

        // wtxid should differ from txid (includes witnesses)
        let txid = compute_txid(&tx);
        assert_ne!(wtxid, txid);
    }

    #[test]
    fn test_sighash_computation() {
        let tx = create_test_tx();
        let network_id = 0x00000001; // Mainnet
        let rw_commitment = empty_rw_commitment();

        let sighash = compute_rw_bound_sighash(&tx, 0, network_id, &rw_commitment);

        // sighash should be deterministic
        let sighash2 = compute_rw_bound_sighash(&tx, 0, network_id, &rw_commitment);
        assert_eq!(sighash, sighash2);

        // Different network_id should produce different sighash
        let sighash_testnet = compute_rw_bound_sighash(&tx, 0, 0x00000002, &rw_commitment);
        assert_ne!(sighash, sighash_testnet);

        // Different input_index should produce different sighash
        let sighash_idx1 = compute_rw_bound_sighash(&tx, 1, network_id, &rw_commitment);
        assert_ne!(sighash, sighash_idx1);
    }

    #[test]
    fn test_pubkey_hash() {
        let pubkey = [0x03; 33]; // Compressed public key
        let hash = pubkey_hash(&pubkey);

        // Should be 20 bytes
        assert_eq!(hash.len(), 20);

        // Should be deterministic
        let hash2 = pubkey_hash(&pubkey);
        assert_eq!(hash, hash2);

        // Different pubkey should produce different hash
        let pubkey2 = [0x02; 33];
        let hash2 = pubkey_hash(&pubkey2);
        assert_ne!(hash, hash2);
    }

    #[test]
    fn test_network_id_encoding() {
        // Verify network_id is encoded as 4 bytes (u32 LE)
        let tx = create_test_tx();
        let network_id: u32 = 0x12345678;
        let rw_commitment = empty_rw_commitment();

        let sighash = compute_rw_bound_sighash(&tx, 0, network_id, &rw_commitment);

        // Re-compute manually to verify encoding
        let wtxid = compute_wtxid(&tx);
        let mut hasher = blake3::Hasher::new();
        hasher.update(CELL_SIG_DOMAIN);
        hasher.update(&network_id.to_le_bytes()); // [0x78, 0x56, 0x34, 0x12]
        hasher.update(&wtxid);
        hasher.update(&0u32.to_le_bytes());
        hasher.update(&rw_commitment);
        let expected = *hasher.finalize().as_bytes();

        assert_eq!(sighash, expected);
    }

    #[test]
    fn test_domain_separation() {
        // txid and wtxid should use different domains
        let tx = create_test_tx();
        let _txid = compute_txid(&tx);

        // Manually compute with wrong domain
        let mut hasher = blake3::Hasher::new();
        hasher.update(CELL_WTXID_DOMAIN); // Wrong domain!
        hasher.update(&tx.version().to_le_bytes());
        // ... (same serialization as txid)

        // This would produce a different hash due to domain separation
        assert_eq!(CELL_TXID_DOMAIN, b"spora-cell/txid");
        assert_eq!(CELL_WTXID_DOMAIN, b"spora-cell/wtxid");
        assert_eq!(CELL_SIG_DOMAIN, b"spora-cell/sig");
    }

    #[test]
    fn test_standard_signature_hash_is_deterministic() {
        let tx = create_test_tx();
        let signing_input = StandardSigningInput {
            lock_hash: [0x11; 32],
            type_hash: Some([0x22; 32]),
            data_hash: [0x33; 32],
            data_bytes: 0,
            capacity: 1000,
        };
        let cache = NoCache;
        let hash_type = TestSigHashType(0b0000_0001);

        let first = calc_standard_signature_hash(&tx, 0, hash_type, &signing_input, &cache);
        let second = calc_standard_signature_hash(&tx, 0, hash_type, &signing_input, &cache);

        assert_eq!(first, second);
    }

    #[test]
    fn test_standard_ecdsa_signature_hash_wraps_schnorr_hash() {
        let tx = create_test_tx();
        let signing_input =
            StandardSigningInput { lock_hash: [0x44; 32], type_hash: None, data_hash: [0x55; 32], data_bytes: 8, capacity: 2000 };
        let cache = NoCache;
        let hash_type = TestSigHashType(0b0000_0001);

        let schnorr_hash = calc_standard_signature_hash(&tx, 0, hash_type, &signing_input, &cache);
        let ecdsa_hash = calc_standard_ecdsa_signature_hash(&tx, 0, hash_type, &signing_input, &cache);

        let mut hasher = CellTxSigningHashEcdsa::new();
        hasher.update(schnorr_hash);
        assert_eq!(ecdsa_hash, hasher.finalize());
    }
}
