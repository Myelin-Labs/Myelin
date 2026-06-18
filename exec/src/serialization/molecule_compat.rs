// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Spora developers
//
// VM Molecule ABI compatibility layer.

//! VM-visible Molecule ABI support.
//!
//! This module implements the canonical Molecule wire layout for Spora's
//! CKB-style VM-facing structures. Molecule ABI `0x8001` is the launch/public
//! VM ABI; Borsh/custom v1 remains available only through explicit legacy
//! selection for old tooling and tests.

use crate::celltx::{CellDep, CellInput, CellOutput, CellTx, DepType, OutPoint, Script};
use crate::serialization::{SerializationError, VmAbiError};
use crate::vm::{ResolvedCell, ResolvedHeader};

const NUMBER_SIZE: usize = 4;
const CKB_RAW_HEADER_SIZE: usize = 192;
const CKB_HEADER_SIZE: usize = 208;
const CKB_EPOCH_NUMBER_BITS: u64 = 24;
const CKB_EPOCH_INDEX_BITS: u64 = 16;
const CKB_EPOCH_LENGTH_BITS: u64 = 16;
const CKB_EPOCH_NUMBER_OFFSET: u64 = 0;
const CKB_EPOCH_INDEX_OFFSET: u64 = CKB_EPOCH_NUMBER_BITS;
const CKB_EPOCH_LENGTH_OFFSET: u64 = CKB_EPOCH_NUMBER_BITS + CKB_EPOCH_INDEX_BITS;
const CKB_EPOCH_NUMBER_MAXIMUM_VALUE: u64 = 1u64 << CKB_EPOCH_NUMBER_BITS;
const CKB_EPOCH_INDEX_MAXIMUM_VALUE: u64 = 1u64 << CKB_EPOCH_INDEX_BITS;
const CKB_EPOCH_LENGTH_MAXIMUM_VALUE: u64 = 1u64 << CKB_EPOCH_LENGTH_BITS;
const CKB_EPOCH_NUMBER_MASK: u64 = CKB_EPOCH_NUMBER_MAXIMUM_VALUE - 1;
const CKB_EPOCH_INDEX_MASK: u64 = CKB_EPOCH_INDEX_MAXIMUM_VALUE - 1;
const CKB_EPOCH_LENGTH_MASK: u64 = CKB_EPOCH_LENGTH_MAXIMUM_VALUE - 1;
const CKB_HASH_PERSONALIZATION: &[u8] = b"ckb-default-hash";
/// Blake160 lock-arg length used by CKB's default secp256k1 sighash-all lock.
pub const CKB_SECP256K1_BLAKE160_LOCK_ARG_SIZE: usize = 20;
/// Recoverable secp256k1 signature length used by CKB's default sighash-all lock.
pub const CKB_SECP256K1_SIGHASH_ALL_SIGNATURE_SIZE: usize = 65;
/// CKB `ScriptHashType::Type`.
pub const CKB_SCRIPT_HASH_TYPE_TYPE: u8 = 1;
/// CKB built-in TYPE_ID system script code hash.
///
/// Parent CKB defines this as consensus `TYPE_ID_CODE_HASH`, the 7-byte ASCII
/// string `TYPE_ID` left-padded into a 32-byte hash value.
pub const CKB_TYPE_ID_CODE_HASH: [u8; 32] =
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, b'T', b'Y', b'P', b'E', b'_', b'I', b'D'];

/// Chain-specific CKB default secp256k1-blake160-sighash-all lock configuration.
///
/// CKB standard lock scripts use the deployed system cell's type hash as
/// `code_hash` with `hash_type = Type`. The exact type hash and dependency
/// OutPoint come from the target chain spec/genesis, so callers must supply them
/// explicitly instead of relying on a Spora hard-coded value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CkbSecp256k1Blake160SighashAllLockConfig {
    /// Type hash of the deployed `secp256k1_blake160_sighash_all` system script.
    pub type_hash: [u8; 32],
    /// Cell dependency that makes the system script available.
    pub cell_dep: CellDep,
}

impl CkbSecp256k1Blake160SighashAllLockConfig {
    /// Build a config from an explicit script type hash and cell dependency.
    pub fn new(type_hash: [u8; 32], cell_dep: CellDep) -> Self {
        Self { type_hash, cell_dep }
    }

    /// Build a config using a CKB dep-group cell dependency.
    pub fn with_dep_group(type_hash: [u8; 32], dep_group_out_point: OutPoint) -> Self {
        Self::new(ckb_secp256k1_blake160_sighash_all_type_hash(type_hash), ckb_dep_group_cell_dep(dep_group_out_point))
    }

    /// Return the configured standard lock cell dependency.
    pub fn cell_dep(&self) -> CellDep {
        self.cell_dep.clone()
    }

    /// Build a CKB standard lock script for a 20-byte Blake160 pubkey hash.
    pub fn lock_script(&self, pubkey_hash: &[u8; CKB_SECP256K1_BLAKE160_LOCK_ARG_SIZE]) -> Script {
        ckb_secp256k1_blake160_sighash_all_lock_script(self.type_hash, pubkey_hash)
    }
}

/// Identity helper that documents that the argument must be the CKB system
/// script type hash, not a Spora Blake3 script hash.
pub fn ckb_secp256k1_blake160_sighash_all_type_hash(type_hash: [u8; 32]) -> [u8; 32] {
    type_hash
}

/// Build a CKB dep-group cell dependency.
pub fn ckb_dep_group_cell_dep(dep_group_out_point: OutPoint) -> CellDep {
    CellDep { out_point: dep_group_out_point, dep_type: DepType::DepGroup }
}

/// Build a CKB standard secp256k1-blake160-sighash-all lock script.
pub fn ckb_secp256k1_blake160_sighash_all_lock_script(
    type_hash: [u8; 32],
    pubkey_hash: &[u8; CKB_SECP256K1_BLAKE160_LOCK_ARG_SIZE],
) -> Script {
    Script::new(type_hash, CKB_SCRIPT_HASH_TYPE_TYPE, pubkey_hash.to_vec())
}

/// Build a CKB built-in TYPE_ID script from already-known 32-byte args.
pub fn ckb_type_id_script(args: &[u8; 32]) -> Script {
    Script::new(CKB_TYPE_ID_CODE_HASH, CKB_SCRIPT_HASH_TYPE_TYPE, args.to_vec())
}

/// Molecule serializer for VM-facing data.
pub struct MoleculeSerializer;

impl MoleculeSerializer {
    /// Molecule VM ABI support is available in this build.
    pub const fn is_available() -> bool {
        true
    }

    /// Molecule-based VM ABI v1.
    pub const fn abi_version() -> u16 {
        0x8001
    }
}

/// Molecule serialization errors.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum MoleculeError {
    /// Input bytes do not match the expected Molecule layout.
    #[error("invalid molecule bytes for {ty}: {reason}")]
    InvalidFormat {
        /// Type name being decoded.
        ty: &'static str,
        /// Concrete validation failure.
        reason: String,
    },
    /// Schema does not match the expected Spora VM ABI schema.
    #[error("schema mismatch: {0}")]
    SchemaMismatch(String),
    /// Validation failed after decoding.
    #[error("validation failed: {0}")]
    ValidationFailed(String),
}

impl From<MoleculeError> for SerializationError {
    fn from(e: MoleculeError) -> Self {
        SerializationError::DeserializationFailed(e.to_string())
    }
}

impl From<MoleculeError> for VmAbiError {
    fn from(e: MoleculeError) -> Self {
        VmAbiError::DeserializationFailed(e.to_string())
    }
}

/// Serialize `ResolvedHeader` as a Spora Molecule table.
pub fn serialize_resolved_header_molecule(header: &ResolvedHeader) -> Result<Vec<u8>, MoleculeError> {
    Ok(encode_table(&[
        header.hash.to_vec(),
        encode_u32(header.version),
        encode_parents_by_level(&header.parents_by_level),
        header.hash_merkle_root.to_vec(),
        header.accepted_id_merkle_root.to_vec(),
        header.cell_commitment.to_vec(),
        header.cell_root.to_vec(),
        header.segment_root.to_vec(),
        encode_u64(header.timestamp),
        encode_u32(header.bits),
        encode_u64(header.nonce),
        encode_u64(header.daa_score),
        header.blue_work.to_vec(),
        encode_u64(header.blue_score),
        header.pruning_point.to_vec(),
    ]))
}

/// Deserialize `ResolvedHeader` from the Spora Molecule table layout.
pub fn deserialize_resolved_header_molecule(bytes: &[u8]) -> Result<ResolvedHeader, MoleculeError> {
    let fields = decode_table(bytes, 15, "ResolvedHeader")?;
    Ok(ResolvedHeader {
        hash: decode_array_32(fields[0], "ResolvedHeader.hash")?,
        version: decode_u32(fields[1], "ResolvedHeader.version")?,
        parents_by_level: decode_parents_by_level(fields[2])?,
        hash_merkle_root: decode_array_32(fields[3], "ResolvedHeader.hash_merkle_root")?,
        accepted_id_merkle_root: decode_array_32(fields[4], "ResolvedHeader.accepted_id_merkle_root")?,
        cell_commitment: decode_array_32(fields[5], "ResolvedHeader.cell_commitment")?,
        cell_root: decode_array_32(fields[6], "ResolvedHeader.cell_root")?,
        segment_root: decode_array_32(fields[7], "ResolvedHeader.segment_root")?,
        timestamp: decode_u64(fields[8], "ResolvedHeader.timestamp")?,
        bits: decode_u32(fields[9], "ResolvedHeader.bits")?,
        nonce: decode_u64(fields[10], "ResolvedHeader.nonce")?,
        daa_score: decode_u64(fields[11], "ResolvedHeader.daa_score")?,
        blue_work: decode_array_24(fields[12], "ResolvedHeader.blue_work")?,
        blue_score: decode_u64(fields[13], "ResolvedHeader.blue_score")?,
        pruning_point: decode_array_32(fields[14], "ResolvedHeader.pruning_point")?,
    })
}

/// Serialize `ResolvedCell` as a Spora Molecule table.
pub fn serialize_resolved_cell_molecule(cell: &ResolvedCell) -> Result<Vec<u8>, MoleculeError> {
    Ok(encode_table(&[serialize_cell_output_molecule(&cell.cell_output)?, encode_bytes_opt(cell.data.as_deref())]))
}

/// Deserialize `ResolvedCell` from the Spora Molecule table layout.
pub fn deserialize_resolved_cell_molecule(bytes: &[u8]) -> Result<ResolvedCell, MoleculeError> {
    let fields = decode_table(bytes, 2, "ResolvedCell")?;
    Ok(ResolvedCell { cell_output: deserialize_cell_output_molecule(fields[0])?, data: decode_bytes_opt(fields[1])? })
}

/// Serialize `Script` using the CKB Molecule `Script` layout.
pub fn serialize_script_molecule(script: &Script) -> Result<Vec<u8>, MoleculeError> {
    Ok(encode_table(&[script.code_hash.to_vec(), vec![script.hash_type], encode_bytes(&script.args)]))
}

/// Deserialize `Script` from the CKB Molecule `Script` layout.
pub fn deserialize_script_molecule(bytes: &[u8]) -> Result<Script, MoleculeError> {
    let fields = decode_table(bytes, 3, "Script")?;
    Ok(Script {
        code_hash: decode_array_32(fields[0], "Script.code_hash")?,
        hash_type: decode_byte(fields[1], "Script.hash_type")?,
        args: decode_bytes(fields[2])?,
    })
}

/// CKB Blake2b-256 with `ckb-default-hash` personalization.
pub fn ckb_blake2b_256(bytes: &[u8]) -> [u8; 32] {
    ckb_blake2b_256_chunks(&[bytes])
}

/// CKB Blake160: first 20 bytes of CKB Blake2b-256.
pub fn ckb_blake160(bytes: &[u8]) -> [u8; 20] {
    let hash = ckb_blake2b_256(bytes);
    let mut out = [0u8; 20];
    out.copy_from_slice(&hash[..20]);
    out
}

/// CKB secp256k1 pubkey hash used by `secp256k1_blake160_sighash_all`.
pub fn ckb_secp256k1_blake160_pubkey_hash(compressed_pubkey: &[u8]) -> [u8; 20] {
    ckb_blake160(compressed_pubkey)
}

/// Verify a recoverable secp256k1 signature against a CKB Blake160 pubkey hash.
///
/// This mirrors the hash binding used by CKB's default
/// `secp256k1_blake160_sighash_all` lock: recover the compressed public key
/// from `signature` and `message_hash`, then compare
/// `blake160(compressed_pubkey)` with the expected 20-byte lock arg.
pub fn ckb_verify_secp256k1_blake160_recoverable_signature(
    expected_pubkey_hash: &[u8; 20],
    signature: &[u8; CKB_SECP256K1_SIGHASH_ALL_SIGNATURE_SIZE],
    message_hash: &[u8; 32],
) -> bool {
    if signature[64] > 3 {
        return false;
    }
    let recovery_id = match secp256k1::ecdsa::RecoveryId::from_i32(signature[64] as i32) {
        Ok(value) => value,
        Err(_) => return false,
    };
    let recoverable_signature = match secp256k1::ecdsa::RecoverableSignature::from_compact(&signature[..64], recovery_id) {
        Ok(value) => value,
        Err(_) => return false,
    };
    let standard_signature = recoverable_signature.to_standard();
    let mut normalized = standard_signature;
    normalized.normalize_s();
    if normalized != standard_signature {
        return false;
    }
    let message = match secp256k1::Message::from_digest_slice(message_hash) {
        Ok(value) => value,
        Err(_) => return false,
    };
    let secp = secp256k1::Secp256k1::new();
    let recovered_pubkey = match secp.recover_ecdsa(&message, &recoverable_signature) {
        Ok(value) => value,
        Err(_) => return false,
    };
    ckb_secp256k1_blake160_pubkey_hash(&recovered_pubkey.serialize()) == *expected_pubkey_hash
}

/// Verify a CKB default secp256k1-blake160-sighash-all witness.
///
/// This helper reads the recoverable signature from `signing_witness.lock`,
/// recomputes the CKB `SIGHASH_ALL` message with that lock field zeroed, and
/// verifies that the recovered compressed public key hashes to
/// `expected_pubkey_hash`.
pub fn ckb_verify_secp256k1_blake160_sighash_all_molecule(
    expected_pubkey_hash: &[u8; CKB_SECP256K1_BLAKE160_LOCK_ARG_SIZE],
    tx: &CellTx,
    signing_witness: &CkbWitnessArgs,
    extra_witnesses: &[&[u8]],
) -> Result<bool, MoleculeError> {
    let Some(signature) = signing_witness.lock.as_deref() else {
        return Ok(false);
    };
    if signature.len() != CKB_SECP256K1_SIGHASH_ALL_SIGNATURE_SIZE {
        return Ok(false);
    }
    let mut signature_bytes = [0u8; CKB_SECP256K1_SIGHASH_ALL_SIGNATURE_SIZE];
    signature_bytes.copy_from_slice(signature);
    let message_hash = ckb_sighash_all_message_with_zeroed_witness_lock_molecule(
        tx,
        signing_witness,
        CKB_SECP256K1_SIGHASH_ALL_SIGNATURE_SIZE,
        extra_witnesses,
    )?;
    Ok(ckb_verify_secp256k1_blake160_recoverable_signature(expected_pubkey_hash, &signature_bytes, &message_hash))
}

/// Sign a CKB default secp256k1-blake160-sighash-all witness.
///
/// The returned witness preserves `input_type` and `output_type`, replaces
/// `lock` with a 65-byte recoverable signature, and signs the CKB
/// `SIGHASH_ALL` message computed from `signing_witness` with its lock field
/// zeroed to the default secp256k1 signature length.
pub fn ckb_sign_secp256k1_blake160_sighash_all_molecule(
    tx: &CellTx,
    signing_witness: &CkbWitnessArgs,
    secret_key: &secp256k1::SecretKey,
    extra_witnesses: &[&[u8]],
) -> Result<CkbWitnessArgs, MoleculeError> {
    let message_hash = ckb_sighash_all_message_with_zeroed_witness_lock_molecule(
        tx,
        signing_witness,
        CKB_SECP256K1_SIGHASH_ALL_SIGNATURE_SIZE,
        extra_witnesses,
    )?;
    let message = secp256k1::Message::from_digest_slice(&message_hash)
        .map_err(|err| MoleculeError::ValidationFailed(format!("invalid CKB sighash message: {err}")))?;
    let secp = secp256k1::Secp256k1::new();
    let signature = secp.sign_ecdsa_recoverable(&message, secret_key);
    let (recovery_id, compact) = signature.serialize_compact();
    let mut signature_bytes = vec![0u8; CKB_SECP256K1_SIGHASH_ALL_SIGNATURE_SIZE];
    signature_bytes[..64].copy_from_slice(&compact);
    signature_bytes[64] = recovery_id.to_i32() as u8;

    Ok(CkbWitnessArgs {
        lock: Some(signature_bytes),
        input_type: signing_witness.input_type.clone(),
        output_type: signing_witness.output_type.clone(),
    })
}

/// Sign and place a CKB default secp256k1-blake160-sighash-all witness.
///
/// The signed `WitnessArgs` is serialized with CKB Molecule and written to
/// `tx.witnesses[input_index]`. Missing witness slots up to the input count are
/// filled with empty witnesses so the witness index matches the input index.
///
/// `extra_witnesses` must contain any additional witness bytes that belong to
/// the same CKB signing group and should be committed after the first signing
/// witness. The helper does not infer script groups.
pub fn ckb_sign_secp256k1_blake160_sighash_all_input_molecule(
    tx: &mut CellTx,
    input_index: usize,
    signing_witness: &CkbWitnessArgs,
    secret_key: &secp256k1::SecretKey,
    extra_witnesses: &[&[u8]],
) -> Result<CkbWitnessArgs, MoleculeError> {
    if input_index >= tx.inputs.len() {
        return Err(MoleculeError::ValidationFailed(format!(
            "CKB sighash-all input index {input_index} is outside transaction inputs length {}",
            tx.inputs.len()
        )));
    }

    let signed_witness = ckb_sign_secp256k1_blake160_sighash_all_molecule(tx, signing_witness, secret_key, extra_witnesses)?;
    let witness_bytes = serialize_ckb_witness_args_molecule(&signed_witness)?;
    while tx.witnesses.len() < tx.inputs.len() {
        tx.witnesses.push(Vec::new());
    }
    tx.witnesses[input_index] = witness_bytes;
    Ok(signed_witness)
}

/// Sign the first input in a discovered CKB secp256k1-blake160 lock group.
///
/// `resolved_inputs` must be aligned with `tx.inputs`, providing each spent
/// cell's lock script. The helper finds all inputs whose resolved lock equals
/// `lock_script`, signs the first group input, writes the serialized
/// `WitnessArgs` into that witness slot, and commits the remaining group input
/// witness bytes after the signing witness. `extra_witnesses` can carry any
/// additional CKB signing bytes the caller wants appended after the group input
/// witnesses, such as non-input tail witnesses.
pub fn ckb_sign_secp256k1_blake160_sighash_all_lock_group_molecule(
    tx: &mut CellTx,
    resolved_inputs: &[CellOutput],
    lock_script: &Script,
    signing_witness: &CkbWitnessArgs,
    secret_key: &secp256k1::SecretKey,
    extra_witnesses: &[&[u8]],
) -> Result<CkbWitnessArgs, MoleculeError> {
    let group_indices = ckb_lock_group_input_indices(tx, resolved_inputs, lock_script)?;
    while tx.witnesses.len() < tx.inputs.len() {
        tx.witnesses.push(Vec::new());
    }
    let signing_index = group_indices[0];
    let group_extra_witnesses = ckb_collect_group_extra_witnesses(tx, &group_indices, extra_witnesses);
    let group_extra_slices = group_extra_witnesses.iter().map(Vec::as_slice).collect::<Vec<_>>();
    ckb_sign_secp256k1_blake160_sighash_all_input_molecule(tx, signing_index, signing_witness, secret_key, &group_extra_slices)
}

/// Verify the first witness in a discovered CKB secp256k1-blake160 lock group.
///
/// The helper discovers input group membership from `resolved_inputs`, reads the
/// first group witness as CKB Molecule `WitnessArgs`, and verifies it with the
/// remaining group input witness bytes plus `extra_witnesses` committed after
/// the zeroed signing witness.
pub fn ckb_verify_secp256k1_blake160_sighash_all_lock_group_molecule(
    expected_pubkey_hash: &[u8; CKB_SECP256K1_BLAKE160_LOCK_ARG_SIZE],
    tx: &CellTx,
    resolved_inputs: &[CellOutput],
    lock_script: &Script,
    extra_witnesses: &[&[u8]],
) -> Result<bool, MoleculeError> {
    let group_indices = ckb_lock_group_input_indices(tx, resolved_inputs, lock_script)?;
    let signing_index = group_indices[0];
    let Some(witness_bytes) = tx.witnesses.get(signing_index).map(Vec::as_slice) else {
        return Ok(false);
    };
    if witness_bytes.is_empty() {
        return Ok(false);
    }
    let signing_witness = deserialize_ckb_witness_args_molecule(witness_bytes)?;
    let group_extra_witnesses = ckb_collect_group_extra_witnesses(tx, &group_indices, extra_witnesses);
    let group_extra_slices = group_extra_witnesses.iter().map(Vec::as_slice).collect::<Vec<_>>();
    ckb_verify_secp256k1_blake160_sighash_all_molecule(expected_pubkey_hash, tx, &signing_witness, &group_extra_slices)
}

/// CKB `Script::calc_script_hash`: Blake2b-256 over packed Molecule bytes.
pub fn ckb_script_hash_molecule(script: &Script) -> Result<[u8; 32], MoleculeError> {
    Ok(ckb_blake2b_256(&serialize_script_molecule(script)?))
}

/// CKB `CellOutput::calc_data_hash`.
///
/// CKB treats empty cell data as the all-zero hash and hashes non-empty data
/// with Blake2b-256 using the `ckb-default-hash` personalization.
pub fn ckb_cell_data_hash(data: &[u8]) -> [u8; 32] {
    if data.is_empty() {
        [0u8; 32]
    } else {
        ckb_blake2b_256(data)
    }
}

/// CKB `RawTransaction::calc_tx_hash`: Blake2b-256 over packed raw transaction bytes.
pub fn ckb_raw_transaction_hash_molecule(tx: &CellTx) -> Result<[u8; 32], MoleculeError> {
    Ok(ckb_blake2b_256(&serialize_raw_transaction_molecule(tx)?))
}

/// CKB transaction witness hash: Blake2b-256 over packed full transaction bytes.
pub fn ckb_transaction_witness_hash_molecule(tx: &CellTx) -> Result<[u8; 32], MoleculeError> {
    Ok(ckb_blake2b_256(&serialize_transaction_molecule(tx)?))
}

/// CKB `Header::calc_header_hash`: Blake2b-256 over packed header bytes.
pub fn ckb_header_hash_molecule(header: &CkbHeader) -> Result<[u8; 32], MoleculeError> {
    Ok(ckb_blake2b_256(&serialize_ckb_header_molecule(header)?))
}

/// Parse CKB's packed `EpochNumberWithFraction` field.
pub fn ckb_epoch_number_with_fraction_from_full_value(value: u64) -> CkbEpochNumberWithFraction {
    CkbEpochNumberWithFraction {
        full_value: value,
        number: (value >> CKB_EPOCH_NUMBER_OFFSET) & CKB_EPOCH_NUMBER_MASK,
        index: (value >> CKB_EPOCH_INDEX_OFFSET) & CKB_EPOCH_INDEX_MASK,
        length: (value >> CKB_EPOCH_LENGTH_OFFSET) & CKB_EPOCH_LENGTH_MASK,
    }
}

/// Pack a well-formed CKB `EpochNumberWithFraction` value.
pub fn ckb_epoch_number_with_fraction_full_value(number: u64, index: u64, length: u64) -> Result<u64, MoleculeError> {
    if number >= CKB_EPOCH_NUMBER_MAXIMUM_VALUE {
        return invalid("EpochNumberWithFraction", format!("number must be less than {CKB_EPOCH_NUMBER_MAXIMUM_VALUE}, got {number}"));
    }
    if index >= CKB_EPOCH_INDEX_MAXIMUM_VALUE {
        return invalid("EpochNumberWithFraction", format!("index must be less than {CKB_EPOCH_INDEX_MAXIMUM_VALUE}, got {index}"));
    }
    if length == 0 || length >= CKB_EPOCH_LENGTH_MAXIMUM_VALUE {
        return invalid("EpochNumberWithFraction", format!("length must be in 1..{CKB_EPOCH_LENGTH_MAXIMUM_VALUE}, got {length}"));
    }
    Ok((length << CKB_EPOCH_LENGTH_OFFSET) | (index << CKB_EPOCH_INDEX_OFFSET) | (number << CKB_EPOCH_NUMBER_OFFSET))
}

/// Return the epoch number component from a CKB raw header.
pub fn ckb_header_epoch_number(header: &CkbRawHeader) -> u64 {
    ckb_epoch_number_with_fraction_from_full_value(header.epoch).number
}

/// Return the epoch index component from a CKB raw header.
pub fn ckb_header_epoch_index(header: &CkbRawHeader) -> u64 {
    ckb_epoch_number_with_fraction_from_full_value(header.epoch).index
}

/// Return the epoch length component from a CKB raw header.
pub fn ckb_header_epoch_length(header: &CkbRawHeader) -> u64 {
    ckb_epoch_number_with_fraction_from_full_value(header.epoch).length
}

/// Return CKB `LOAD_HEADER_BY_FIELD` field 1 for a raw header.
///
/// CKB computes epoch start block number as `header.number - epoch.index()`.
pub fn ckb_header_epoch_start_block_number(header: &CkbRawHeader) -> Result<u64, MoleculeError> {
    let epoch = ckb_epoch_number_with_fraction_from_full_value(header.epoch);
    header.number.checked_sub(epoch.index).ok_or_else(|| {
        MoleculeError::ValidationFailed(format!("header number {} is smaller than epoch index {}", header.number, epoch.index))
    })
}

/// Compute the CKB `SIGHASH_ALL` message from explicit witness material.
///
/// This matches the CKB hasher update order:
/// `raw_tx_hash || len(signing_witness) || signing_witness || len(extra) || extra...`.
/// The caller is responsible for supplying the signable witness bytes, for
/// example a `WitnessArgs` value with the lock field zeroed according to the
/// lock script's signing rule.
pub fn ckb_sighash_all_message_molecule(
    tx: &CellTx,
    signing_witness: &[u8],
    extra_witnesses: &[&[u8]],
) -> Result<[u8; 32], MoleculeError> {
    let raw_hash = ckb_raw_transaction_hash_molecule(tx)?;
    let mut state = new_ckb_blake2b_state();
    state.update(&raw_hash);

    let signing_len = (signing_witness.len() as u64).to_le_bytes();
    state.update(&signing_len);
    state.update(signing_witness);

    for witness in extra_witnesses {
        let len = (witness.len() as u64).to_le_bytes();
        state.update(&len);
        state.update(witness);
    }

    Ok(finalize_ckb_blake2b_256(state))
}

/// Serialize `OutPoint` using the CKB Molecule `OutPoint` struct layout.
pub fn serialize_outpoint_molecule(outpoint: &OutPoint) -> Result<Vec<u8>, MoleculeError> {
    let mut out = Vec::with_capacity(36);
    out.extend_from_slice(&outpoint.tx_hash);
    out.extend_from_slice(&outpoint.index.to_le_bytes());
    Ok(out)
}

/// Deserialize `OutPoint` from the CKB Molecule `OutPoint` struct layout.
pub fn deserialize_outpoint_molecule(bytes: &[u8]) -> Result<OutPoint, MoleculeError> {
    if bytes.len() != 36 {
        return invalid("OutPoint", format!("expected 36 bytes, got {}", bytes.len()));
    }
    Ok(OutPoint { tx_hash: decode_array_32(&bytes[..32], "OutPoint.tx_hash")?, index: decode_u32(&bytes[32..36], "OutPoint.index")? })
}

/// Serialize CKB `OutPointVec`, the DepGroup cell data payload.
///
/// Molecule encodes a fixvec of fixed-size `OutPoint` structs as:
/// `u32 count || count * OutPoint`. CKB treats an empty DepGroup as invalid,
/// so this helper rejects empty input instead of mirroring Spora's permissive
/// count-prefixed DepGroup helper.
pub fn serialize_ckb_outpoint_vec_molecule(outpoints: &[OutPoint]) -> Result<Vec<u8>, MoleculeError> {
    if outpoints.is_empty() {
        return invalid("OutPointVec", "CKB DepGroup OutPointVec must not be empty");
    }
    let mut out = Vec::with_capacity(NUMBER_SIZE + outpoints.len() * 36);
    out.extend_from_slice(&pack_number(outpoints.len()));
    for outpoint in outpoints {
        out.extend_from_slice(&serialize_outpoint_molecule(outpoint)?);
    }
    Ok(out)
}

/// Deserialize CKB `OutPointVec`, the DepGroup cell data payload.
pub fn deserialize_ckb_outpoint_vec_molecule(bytes: &[u8]) -> Result<Vec<OutPoint>, MoleculeError> {
    let count = unpack_number(bytes, "OutPointVec")?;
    if count == 0 {
        return invalid("OutPointVec", "CKB DepGroup OutPointVec must not be empty");
    }
    let expected = NUMBER_SIZE + count * 36;
    if bytes.len() != expected {
        return invalid("OutPointVec", format!("expected {expected} bytes for {count} outpoints, got {}", bytes.len()));
    }

    bytes[NUMBER_SIZE..].chunks_exact(36).map(deserialize_outpoint_molecule).collect()
}

/// Compute CKB TYPE_ID creation args.
///
/// CKB's built-in TYPE_ID script hashes the first transaction input's packed
/// `CellInput` bytes followed by the first output index in the type-id script
/// group encoded as little-endian `u64`.
pub fn ckb_type_id_args(first_input: &CellInput, first_output_index: u64) -> Result<[u8; 32], MoleculeError> {
    let input_bytes = serialize_cell_input_molecule(first_input)?;
    let output_index_bytes = first_output_index.to_le_bytes();
    Ok(ckb_blake2b_256_chunks(&[input_bytes.as_slice(), output_index_bytes.as_slice()]))
}

/// Apply CKB TYPE_ID creation args to an output type script.
///
/// This is a transaction-builder helper for creating a TYPE_ID cell. The output
/// must already carry the intended TYPE_ID code hash and hash type; this helper
/// only fills the script args using CKB's first-input plus output-index rule.
pub fn ckb_apply_type_id_args_to_output_molecule(tx: &mut CellTx, output_index: usize) -> Result<[u8; 32], MoleculeError> {
    if output_index >= tx.outputs.len() {
        return Err(MoleculeError::ValidationFailed(format!(
            "CKB TYPE_ID output index {output_index} is outside transaction outputs length {}",
            tx.outputs.len()
        )));
    }
    let first_input = tx
        .inputs
        .first()
        .ok_or_else(|| MoleculeError::ValidationFailed("CKB TYPE_ID creation requires at least one transaction input".to_string()))?;
    let args = ckb_type_id_args(first_input, output_index as u64)?;
    let output = tx.outputs.get_mut(output_index).expect("output index checked above");
    let type_script = output.type_.as_mut().ok_or_else(|| {
        MoleculeError::ValidationFailed(format!("CKB TYPE_ID output index {output_index} does not have a type script"))
    })?;
    type_script.args = args.to_vec();
    Ok(args)
}

/// Apply a complete CKB built-in TYPE_ID type script to an output.
///
/// This is the builder-oriented counterpart to
/// [`ckb_apply_type_id_args_to_output_molecule`]. It computes the creation args
/// from the transaction's first input plus `output_index`, then installs the
/// canonical CKB built-in TYPE_ID script (`TYPE_ID_CODE_HASH`, hash type
/// `Type`) on that output.
pub fn ckb_apply_type_id_script_to_output_molecule(tx: &mut CellTx, output_index: usize) -> Result<[u8; 32], MoleculeError> {
    if output_index >= tx.outputs.len() {
        return Err(MoleculeError::ValidationFailed(format!(
            "CKB TYPE_ID output index {output_index} is outside transaction outputs length {}",
            tx.outputs.len()
        )));
    }
    let first_input = tx
        .inputs
        .first()
        .ok_or_else(|| MoleculeError::ValidationFailed("CKB TYPE_ID creation requires at least one transaction input".to_string()))?;
    let args = ckb_type_id_args(first_input, output_index as u64)?;
    tx.outputs.get_mut(output_index).expect("output index checked above").type_ = Some(ckb_type_id_script(&args));
    Ok(args)
}

/// Verify CKB built-in TYPE_ID script-group rules.
///
/// This mirrors CKB's native TYPE_ID system script rule for a precomputed
/// script group:
/// - script args must be exactly 32 bytes;
/// - the current TYPE_ID script group may contain at most one input and one
///   output cell;
/// - creation groups, which have no input cell, must have one output cell and
///   args equal to `ckb_type_id_args(tx.inputs[0], output_index)`.
///
/// The caller is responsible for constructing `input_indices` and
/// `output_indices` from cells using the same TYPE_ID script. CKB's script
/// grouping guarantees matching script identity; this helper verifies the
/// built-in TYPE_ID constraints and hash material.
pub fn ckb_verify_type_id_script_group_molecule(
    tx: &CellTx,
    script_args: &[u8],
    input_indices: &[usize],
    output_indices: &[usize],
) -> Result<(), MoleculeError> {
    if script_args.len() != 32 {
        return Err(MoleculeError::ValidationFailed(format!("CKB TYPE_ID args must be 32 bytes, got {}", script_args.len())));
    }
    if input_indices.len() > 1 || output_indices.len() > 1 {
        return Err(MoleculeError::ValidationFailed(format!(
            "CKB TYPE_ID group may contain at most one input and one output, got {} inputs and {} outputs",
            input_indices.len(),
            output_indices.len()
        )));
    }

    for input_index in input_indices {
        if *input_index >= tx.inputs.len() {
            return Err(MoleculeError::ValidationFailed(format!(
                "CKB TYPE_ID input index {input_index} is outside transaction inputs length {}",
                tx.inputs.len()
            )));
        }
    }
    for output_index in output_indices {
        if *output_index >= tx.outputs.len() {
            return Err(MoleculeError::ValidationFailed(format!(
                "CKB TYPE_ID output index {output_index} is outside transaction outputs length {}",
                tx.outputs.len()
            )));
        }
    }

    if input_indices.is_empty() {
        let first_input = tx.inputs.first().ok_or_else(|| {
            MoleculeError::ValidationFailed("CKB TYPE_ID creation requires at least one transaction input".to_string())
        })?;
        let output_index = output_indices.first().ok_or_else(|| {
            MoleculeError::ValidationFailed("CKB TYPE_ID creation requires one output in the script group".to_string())
        })?;
        let expected_args = ckb_type_id_args(first_input, *output_index as u64)?;
        if script_args != expected_args.as_slice() {
            return Err(MoleculeError::ValidationFailed(
                "CKB TYPE_ID creation args do not match first input and output index".to_string(),
            ));
        }
    }

    Ok(())
}

/// Verify CKB TYPE_ID rules for the script group discovered from resolved cells.
///
/// `resolved_inputs` must be aligned with `tx.inputs`, providing each spent
/// cell's output metadata. Input group membership is discovered from those
/// resolved input type scripts; output membership is discovered from
/// `tx.outputs[*].type_`. The discovered group is then checked with
/// [`ckb_verify_type_id_script_group_molecule`].
pub fn ckb_verify_type_id_script_molecule(
    tx: &CellTx,
    resolved_inputs: &[CellOutput],
    type_id_script: &Script,
) -> Result<(), MoleculeError> {
    if resolved_inputs.len() != tx.inputs.len() {
        return Err(MoleculeError::ValidationFailed(format!(
            "CKB TYPE_ID resolved input count {} does not match transaction input count {}",
            resolved_inputs.len(),
            tx.inputs.len()
        )));
    }

    let input_indices = resolved_inputs
        .iter()
        .enumerate()
        .filter_map(|(index, cell)| (cell.type_.as_ref() == Some(type_id_script)).then_some(index))
        .collect::<Vec<_>>();
    let output_indices = tx
        .outputs
        .iter()
        .enumerate()
        .filter_map(|(index, cell)| (cell.type_.as_ref() == Some(type_id_script)).then_some(index))
        .collect::<Vec<_>>();

    ckb_verify_type_id_script_group_molecule(tx, &type_id_script.args, &input_indices, &output_indices)
}

fn ckb_lock_group_input_indices(
    tx: &CellTx,
    resolved_inputs: &[CellOutput],
    lock_script: &Script,
) -> Result<Vec<usize>, MoleculeError> {
    if resolved_inputs.len() != tx.inputs.len() {
        return Err(MoleculeError::ValidationFailed(format!(
            "CKB lock group resolved input count {} does not match transaction input count {}",
            resolved_inputs.len(),
            tx.inputs.len()
        )));
    }
    let group_indices = resolved_inputs
        .iter()
        .enumerate()
        .filter_map(|(index, cell)| (cell.lock == *lock_script).then_some(index))
        .collect::<Vec<_>>();
    if group_indices.is_empty() {
        return Err(MoleculeError::ValidationFailed("CKB lock group has no matching resolved input cells".to_string()));
    }
    Ok(group_indices)
}

fn ckb_collect_group_extra_witnesses(tx: &CellTx, group_indices: &[usize], extra_witnesses: &[&[u8]]) -> Vec<Vec<u8>> {
    let mut witnesses =
        group_indices.iter().skip(1).map(|index| tx.witnesses.get(*index).cloned().unwrap_or_default()).collect::<Vec<_>>();
    witnesses.extend(extra_witnesses.iter().map(|witness| witness.to_vec()));
    witnesses
}

/// Serialize `CellInput` using the CKB Molecule `CellInput` struct layout.
pub fn serialize_cell_input_molecule(input: &CellInput) -> Result<Vec<u8>, MoleculeError> {
    let mut out = Vec::with_capacity(44);
    out.extend_from_slice(&input.since.to_le_bytes());
    out.extend_from_slice(&serialize_outpoint_molecule(&input.previous_output)?);
    Ok(out)
}

/// Deserialize `CellInput` from the CKB Molecule `CellInput` struct layout.
pub fn deserialize_cell_input_molecule(bytes: &[u8]) -> Result<CellInput, MoleculeError> {
    if bytes.len() != 44 {
        return invalid("CellInput", format!("expected 44 bytes, got {}", bytes.len()));
    }
    Ok(CellInput {
        since: decode_u64(&bytes[..8], "CellInput.since")?,
        previous_output: deserialize_outpoint_molecule(&bytes[8..44])?,
    })
}

/// Serialize `CellDep` using the CKB Molecule `CellDep` struct layout.
pub fn serialize_cell_dep_molecule(dep: &CellDep) -> Result<Vec<u8>, MoleculeError> {
    let mut out = Vec::with_capacity(37);
    out.extend_from_slice(&serialize_outpoint_molecule(&dep.out_point)?);
    out.push(encode_dep_type(&dep.dep_type));
    Ok(out)
}

/// Deserialize `CellDep` from the CKB Molecule `CellDep` struct layout.
pub fn deserialize_cell_dep_molecule(bytes: &[u8]) -> Result<CellDep, MoleculeError> {
    if bytes.len() != 37 {
        return invalid("CellDep", format!("expected 37 bytes, got {}", bytes.len()));
    }
    Ok(CellDep { out_point: deserialize_outpoint_molecule(&bytes[..36])?, dep_type: decode_dep_type(bytes[36])? })
}

/// Serialize `CellOutput` using the CKB Molecule `CellOutput` table layout.
pub fn serialize_cell_output_molecule(output: &CellOutput) -> Result<Vec<u8>, MoleculeError> {
    Ok(encode_table(&[
        encode_u64(output.capacity),
        serialize_script_molecule(&output.lock)?,
        encode_script_opt(output.type_.as_ref())?,
    ]))
}

/// Deserialize `CellOutput` from the CKB Molecule `CellOutput` table layout.
pub fn deserialize_cell_output_molecule(bytes: &[u8]) -> Result<CellOutput, MoleculeError> {
    let fields = decode_table(bytes, 3, "CellOutput")?;
    Ok(CellOutput {
        capacity: decode_u64(fields[0], "CellOutput.capacity")?,
        lock: deserialize_script_molecule(fields[1])?,
        type_: decode_script_opt(fields[2])?,
    })
}

/// Serialize `CellTx`'s raw transaction fields using the CKB Molecule `RawTransaction` table layout.
pub fn serialize_raw_transaction_molecule(tx: &CellTx) -> Result<Vec<u8>, MoleculeError> {
    Ok(encode_table(&[
        encode_u32(tx.version()),
        encode_fixvec_cell_deps(&tx.cell_deps)?,
        encode_fixvec_byte32(&tx.header_deps),
        encode_fixvec_cell_inputs(&tx.inputs)?,
        encode_dynvec_cell_outputs(&tx.outputs)?,
        encode_bytes_vec(&tx.outputs_data),
    ]))
}

/// Deserialize CKB Molecule `RawTransaction` bytes into a witness-free `CellTx`.
pub fn deserialize_raw_transaction_molecule(bytes: &[u8]) -> Result<CellTx, MoleculeError> {
    let fields = decode_table(bytes, 6, "RawTransaction")?;
    let outputs = decode_dynvec_cell_outputs(fields[4])?;
    let outputs_data = decode_bytes_vec(fields[5], "RawTransaction.outputs_data")?;
    if outputs.len() != outputs_data.len() {
        return Err(MoleculeError::ValidationFailed(format!(
            "RawTransaction outputs/outputs_data length mismatch: {} outputs, {} data entries",
            outputs.len(),
            outputs_data.len()
        )));
    }

    Ok(CellTx {
        version: decode_u32(fields[0], "RawTransaction.version")?,
        cell_deps: decode_fixvec_cell_deps(fields[1])?,
        header_deps: decode_fixvec_byte32(fields[2], "RawTransaction.header_deps")?,
        inputs: decode_fixvec_cell_inputs(fields[3])?,
        outputs,
        outputs_data,
        witnesses: vec![],
    })
}

/// Serialize `CellTx` using the CKB Molecule `Transaction` table layout.
pub fn serialize_transaction_molecule(tx: &CellTx) -> Result<Vec<u8>, MoleculeError> {
    Ok(encode_table(&[serialize_raw_transaction_molecule(tx)?, encode_bytes_vec(&tx.witnesses)]))
}

/// Deserialize `CellTx` from the CKB Molecule `Transaction` table layout.
pub fn deserialize_transaction_molecule(bytes: &[u8]) -> Result<CellTx, MoleculeError> {
    let fields = decode_table(bytes, 2, "Transaction")?;
    let mut tx = deserialize_raw_transaction_molecule(fields[0])?;
    tx.witnesses = decode_bytes_vec(fields[1], "Transaction.witnesses")?;
    Ok(tx)
}

/// CKB `RawHeader` fixed Molecule struct payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkbRawHeader {
    /// Header version.
    pub version: u32,
    /// Compact proof-of-work target.
    pub compact_target: u32,
    /// Unix timestamp in milliseconds.
    pub timestamp: u64,
    /// Block number.
    pub number: u64,
    /// Packed epoch field.
    pub epoch: u64,
    /// Parent block hash.
    pub parent_hash: [u8; 32],
    /// Transactions root hash.
    pub transactions_root: [u8; 32],
    /// Proposals hash.
    pub proposals_hash: [u8; 32],
    /// Extra hash.
    pub extra_hash: [u8; 32],
    /// DAO field bytes.
    pub dao: [u8; 32],
}

/// CKB `Header` fixed Molecule struct payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CkbHeader {
    /// Raw header fields.
    pub raw: CkbRawHeader,
    /// Proof-of-work nonce.
    pub nonce: u128,
}

/// CKB packed `EpochNumberWithFraction` components.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CkbEpochNumberWithFraction {
    /// Original packed 64-bit value.
    pub full_value: u64,
    /// Epoch number component.
    pub number: u64,
    /// Block index within the epoch.
    pub index: u64,
    /// Epoch length in blocks.
    pub length: u64,
}

/// Serialize CKB `RawHeader` using the canonical Molecule struct layout.
pub fn serialize_ckb_raw_header_molecule(header: &CkbRawHeader) -> Result<Vec<u8>, MoleculeError> {
    let mut out = Vec::with_capacity(CKB_RAW_HEADER_SIZE);
    out.extend_from_slice(&header.version.to_le_bytes());
    out.extend_from_slice(&header.compact_target.to_le_bytes());
    out.extend_from_slice(&header.timestamp.to_le_bytes());
    out.extend_from_slice(&header.number.to_le_bytes());
    out.extend_from_slice(&header.epoch.to_le_bytes());
    out.extend_from_slice(&header.parent_hash);
    out.extend_from_slice(&header.transactions_root);
    out.extend_from_slice(&header.proposals_hash);
    out.extend_from_slice(&header.extra_hash);
    out.extend_from_slice(&header.dao);
    debug_assert_eq!(out.len(), CKB_RAW_HEADER_SIZE);
    Ok(out)
}

/// Deserialize CKB Molecule `RawHeader` bytes.
pub fn deserialize_ckb_raw_header_molecule(bytes: &[u8]) -> Result<CkbRawHeader, MoleculeError> {
    if bytes.len() != CKB_RAW_HEADER_SIZE {
        return invalid("RawHeader", format!("expected {CKB_RAW_HEADER_SIZE} bytes, got {}", bytes.len()));
    }
    Ok(CkbRawHeader {
        version: decode_u32(&bytes[0..4], "RawHeader.version")?,
        compact_target: decode_u32(&bytes[4..8], "RawHeader.compact_target")?,
        timestamp: decode_u64(&bytes[8..16], "RawHeader.timestamp")?,
        number: decode_u64(&bytes[16..24], "RawHeader.number")?,
        epoch: decode_u64(&bytes[24..32], "RawHeader.epoch")?,
        parent_hash: decode_array_32(&bytes[32..64], "RawHeader.parent_hash")?,
        transactions_root: decode_array_32(&bytes[64..96], "RawHeader.transactions_root")?,
        proposals_hash: decode_array_32(&bytes[96..128], "RawHeader.proposals_hash")?,
        extra_hash: decode_array_32(&bytes[128..160], "RawHeader.extra_hash")?,
        dao: decode_array_32(&bytes[160..192], "RawHeader.dao")?,
    })
}

/// Serialize CKB `Header` using the canonical Molecule struct layout.
pub fn serialize_ckb_header_molecule(header: &CkbHeader) -> Result<Vec<u8>, MoleculeError> {
    let mut out = serialize_ckb_raw_header_molecule(&header.raw)?;
    out.extend_from_slice(&header.nonce.to_le_bytes());
    debug_assert_eq!(out.len(), CKB_HEADER_SIZE);
    Ok(out)
}

/// Deserialize CKB Molecule `Header` bytes.
pub fn deserialize_ckb_header_molecule(bytes: &[u8]) -> Result<CkbHeader, MoleculeError> {
    if bytes.len() != CKB_HEADER_SIZE {
        return invalid("Header", format!("expected {CKB_HEADER_SIZE} bytes, got {}", bytes.len()));
    }
    Ok(CkbHeader {
        raw: deserialize_ckb_raw_header_molecule(&bytes[..CKB_RAW_HEADER_SIZE])?,
        nonce: decode_u128(&bytes[CKB_RAW_HEADER_SIZE..CKB_HEADER_SIZE], "Header.nonce")?,
    })
}

/// CKB `WitnessArgs` table payload.
///
/// The fields use CKB `BytesOpt`: `None` is encoded as an empty field and
/// `Some(bytes)` is encoded as Molecule `Bytes`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CkbWitnessArgs {
    /// Lock witness bytes.
    pub lock: Option<Vec<u8>>,
    /// Input type witness bytes.
    pub input_type: Option<Vec<u8>>,
    /// Output type witness bytes.
    pub output_type: Option<Vec<u8>>,
}

impl CkbWitnessArgs {
    /// Create a CKB `WitnessArgs` value.
    pub fn new(lock: Option<Vec<u8>>, input_type: Option<Vec<u8>>, output_type: Option<Vec<u8>>) -> Self {
        Self { lock, input_type, output_type }
    }

    /// Return a copy with the `lock` field replaced by zero bytes.
    pub fn with_zeroed_lock(&self, lock_len: usize) -> Self {
        Self { lock: Some(vec![0u8; lock_len]), input_type: self.input_type.clone(), output_type: self.output_type.clone() }
    }
}

/// Serialize CKB `WitnessArgs` using the canonical Molecule table layout.
pub fn serialize_ckb_witness_args_molecule(args: &CkbWitnessArgs) -> Result<Vec<u8>, MoleculeError> {
    Ok(encode_table(&[
        encode_bytes_opt(args.lock.as_deref()),
        encode_bytes_opt(args.input_type.as_deref()),
        encode_bytes_opt(args.output_type.as_deref()),
    ]))
}

/// Deserialize CKB Molecule `WitnessArgs` bytes.
pub fn deserialize_ckb_witness_args_molecule(bytes: &[u8]) -> Result<CkbWitnessArgs, MoleculeError> {
    let fields = decode_table(bytes, 3, "WitnessArgs")?;
    Ok(CkbWitnessArgs {
        lock: decode_bytes_opt(fields[0])?,
        input_type: decode_bytes_opt(fields[1])?,
        output_type: decode_bytes_opt(fields[2])?,
    })
}

/// Compute the CKB `SIGHASH_ALL` message using typed `WitnessArgs` material.
pub fn ckb_sighash_all_message_from_witness_args_molecule(
    tx: &CellTx,
    signing_witness: &CkbWitnessArgs,
    extra_witnesses: &[&[u8]],
) -> Result<[u8; 32], MoleculeError> {
    let signing_witness = serialize_ckb_witness_args_molecule(signing_witness)?;
    ckb_sighash_all_message_molecule(tx, &signing_witness, extra_witnesses)
}

/// Compute CKB `SIGHASH_ALL` with the signing witness lock field zeroed.
///
/// CKB's standard sighash-all lock signs the first group witness as
/// `WitnessArgs` after replacing its `lock` field with zero bytes whose length
/// matches the final signature field. For the default secp256k1 lock this length
/// is [`CKB_SECP256K1_SIGHASH_ALL_SIGNATURE_SIZE`].
pub fn ckb_sighash_all_message_with_zeroed_witness_lock_molecule(
    tx: &CellTx,
    signing_witness: &CkbWitnessArgs,
    lock_len: usize,
    extra_witnesses: &[&[u8]],
) -> Result<[u8; 32], MoleculeError> {
    ckb_sighash_all_message_from_witness_args_molecule(tx, &signing_witness.with_zeroed_lock(lock_len), extra_witnesses)
}

fn encode_u32(value: u32) -> Vec<u8> {
    value.to_le_bytes().to_vec()
}

fn encode_u64(value: u64) -> Vec<u8> {
    value.to_le_bytes().to_vec()
}

fn pack_number(value: usize) -> [u8; NUMBER_SIZE] {
    (value as u32).to_le_bytes()
}

fn unpack_number(bytes: &[u8], ty: &'static str) -> Result<usize, MoleculeError> {
    if bytes.len() < NUMBER_SIZE {
        return invalid(ty, format!("expected at least 4 bytes for number, got {}", bytes.len()));
    }
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize)
}

fn encode_table(fields: &[Vec<u8>]) -> Vec<u8> {
    let header_size = NUMBER_SIZE * (fields.len() + 1);
    let total_size = header_size + fields.iter().map(Vec::len).sum::<usize>();
    let mut out = Vec::with_capacity(total_size);
    out.extend_from_slice(&pack_number(total_size));

    let mut offset = header_size;
    for field in fields {
        out.extend_from_slice(&pack_number(offset));
        offset += field.len();
    }
    for field in fields {
        out.extend_from_slice(field);
    }
    out
}

fn decode_table<'a>(bytes: &'a [u8], expected_fields: usize, ty: &'static str) -> Result<Vec<&'a [u8]>, MoleculeError> {
    if bytes.len() < NUMBER_SIZE * 2 {
        return invalid(ty, format!("table header is too short: {}", bytes.len()));
    }
    let total_size = unpack_number(bytes, ty)?;
    if total_size != bytes.len() {
        return invalid(ty, format!("total size mismatch: header {total_size}, actual {}", bytes.len()));
    }

    let first_offset = unpack_number(&bytes[NUMBER_SIZE..], ty)?;
    if first_offset % NUMBER_SIZE != 0 || first_offset < NUMBER_SIZE * 2 || first_offset > bytes.len() {
        return invalid(ty, format!("invalid first field offset {first_offset}"));
    }

    let field_count = first_offset / NUMBER_SIZE - 1;
    if field_count != expected_fields {
        return Err(MoleculeError::SchemaMismatch(format!("{ty}: expected {expected_fields} fields, got {field_count}")));
    }

    let mut offsets = Vec::with_capacity(field_count + 1);
    for chunk in bytes[NUMBER_SIZE..first_offset].chunks_exact(NUMBER_SIZE) {
        offsets.push(unpack_number(chunk, ty)?);
    }
    offsets.push(total_size);

    if offsets.windows(2).any(|pair| pair[0] > pair[1]) {
        return invalid(ty, "field offsets are not monotonic");
    }
    if offsets.iter().any(|offset| *offset < first_offset || *offset > total_size) {
        return invalid(ty, "field offset is outside table payload");
    }

    Ok(offsets.windows(2).map(|pair| &bytes[pair[0]..pair[1]]).collect())
}

fn encode_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(NUMBER_SIZE + bytes.len());
    out.extend_from_slice(&pack_number(bytes.len()));
    out.extend_from_slice(bytes);
    out
}

fn decode_bytes(bytes: &[u8]) -> Result<Vec<u8>, MoleculeError> {
    let len = unpack_number(bytes, "Bytes")?;
    let expected = NUMBER_SIZE + len;
    if bytes.len() != expected {
        return invalid("Bytes", format!("expected {expected} bytes, got {}", bytes.len()));
    }
    Ok(bytes[NUMBER_SIZE..].to_vec())
}

fn encode_bytes_opt(bytes: Option<&[u8]>) -> Vec<u8> {
    bytes.map(encode_bytes).unwrap_or_default()
}

fn decode_bytes_opt(bytes: &[u8]) -> Result<Option<Vec<u8>>, MoleculeError> {
    if bytes.is_empty() {
        Ok(None)
    } else {
        decode_bytes(bytes).map(Some)
    }
}

fn encode_script_opt(script: Option<&Script>) -> Result<Vec<u8>, MoleculeError> {
    script.map(serialize_script_molecule).transpose().map(Option::unwrap_or_default)
}

fn decode_script_opt(bytes: &[u8]) -> Result<Option<Script>, MoleculeError> {
    if bytes.is_empty() {
        Ok(None)
    } else {
        deserialize_script_molecule(bytes).map(Some)
    }
}

fn ckb_blake2b_256_chunks(chunks: &[&[u8]]) -> [u8; 32] {
    let mut state = new_ckb_blake2b_state();
    for chunk in chunks {
        state.update(chunk);
    }
    finalize_ckb_blake2b_256(state)
}

fn new_ckb_blake2b_state() -> blake2b_simd::State {
    let mut params = blake2b_simd::Params::new();
    params.hash_length(32).personal(CKB_HASH_PERSONALIZATION);
    params.to_state()
}

fn finalize_ckb_blake2b_256(state: blake2b_simd::State) -> [u8; 32] {
    let digest = state.finalize();
    let bytes = digest.as_bytes();
    let mut out = [0u8; 32];
    out.copy_from_slice(bytes);
    out
}

fn encode_dep_type(dep_type: &DepType) -> u8 {
    match dep_type {
        DepType::Code => 0,
        DepType::DepGroup => 1,
    }
}

fn decode_dep_type(byte: u8) -> Result<DepType, MoleculeError> {
    match byte {
        0 => Ok(DepType::Code),
        1 => Ok(DepType::DepGroup),
        other => invalid("DepType", format!("expected 0 or 1, got {other}")),
    }
}

fn encode_fixvec_byte32(values: &[[u8; 32]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(NUMBER_SIZE + values.len() * 32);
    out.extend_from_slice(&pack_number(values.len()));
    for value in values {
        out.extend_from_slice(value);
    }
    out
}

fn decode_fixvec_byte32(bytes: &[u8], ty: &'static str) -> Result<Vec<[u8; 32]>, MoleculeError> {
    let count = unpack_number(bytes, ty)?;
    let expected = NUMBER_SIZE + count * 32;
    if bytes.len() != expected {
        return invalid(ty, format!("expected {expected} bytes, got {}", bytes.len()));
    }

    bytes[NUMBER_SIZE..].chunks_exact(32).map(|chunk| decode_array_32(chunk, ty)).collect()
}

fn encode_fixvec_cell_deps(deps: &[CellDep]) -> Result<Vec<u8>, MoleculeError> {
    let mut out = Vec::with_capacity(NUMBER_SIZE + deps.len() * 37);
    out.extend_from_slice(&pack_number(deps.len()));
    for dep in deps {
        out.extend_from_slice(&serialize_cell_dep_molecule(dep)?);
    }
    Ok(out)
}

fn decode_fixvec_cell_deps(bytes: &[u8]) -> Result<Vec<CellDep>, MoleculeError> {
    let count = unpack_number(bytes, "CellDepVec")?;
    let expected = NUMBER_SIZE + count * 37;
    if bytes.len() != expected {
        return invalid("CellDepVec", format!("expected {expected} bytes, got {}", bytes.len()));
    }

    bytes[NUMBER_SIZE..].chunks_exact(37).map(deserialize_cell_dep_molecule).collect()
}

fn encode_fixvec_cell_inputs(inputs: &[CellInput]) -> Result<Vec<u8>, MoleculeError> {
    let mut out = Vec::with_capacity(NUMBER_SIZE + inputs.len() * 44);
    out.extend_from_slice(&pack_number(inputs.len()));
    for input in inputs {
        out.extend_from_slice(&serialize_cell_input_molecule(input)?);
    }
    Ok(out)
}

fn decode_fixvec_cell_inputs(bytes: &[u8]) -> Result<Vec<CellInput>, MoleculeError> {
    let count = unpack_number(bytes, "CellInputVec")?;
    let expected = NUMBER_SIZE + count * 44;
    if bytes.len() != expected {
        return invalid("CellInputVec", format!("expected {expected} bytes, got {}", bytes.len()));
    }

    bytes[NUMBER_SIZE..].chunks_exact(44).map(deserialize_cell_input_molecule).collect()
}

fn encode_dynvec_cell_outputs(outputs: &[CellOutput]) -> Result<Vec<u8>, MoleculeError> {
    let items = outputs.iter().map(serialize_cell_output_molecule).collect::<Result<Vec<_>, _>>()?;
    Ok(encode_dynvec(&items))
}

fn decode_dynvec_cell_outputs(bytes: &[u8]) -> Result<Vec<CellOutput>, MoleculeError> {
    decode_dynvec(bytes, "CellOutputVec")?.into_iter().map(deserialize_cell_output_molecule).collect()
}

fn encode_bytes_vec(bytes_vec: &[Vec<u8>]) -> Vec<u8> {
    let items = bytes_vec.iter().map(|bytes| encode_bytes(bytes)).collect::<Vec<_>>();
    encode_dynvec(&items)
}

fn decode_bytes_vec(bytes: &[u8], ty: &'static str) -> Result<Vec<Vec<u8>>, MoleculeError> {
    decode_dynvec(bytes, ty)?.into_iter().map(decode_bytes).collect()
}

fn encode_dynvec(items: &[Vec<u8>]) -> Vec<u8> {
    if items.is_empty() {
        return pack_number(NUMBER_SIZE).to_vec();
    }
    encode_table(items)
}

fn decode_dynvec<'a>(bytes: &'a [u8], ty: &'static str) -> Result<Vec<&'a [u8]>, MoleculeError> {
    if bytes.len() < NUMBER_SIZE {
        return invalid(ty, format!("dynvec header is too short: {}", bytes.len()));
    }
    let total_size = unpack_number(bytes, ty)?;
    if total_size != bytes.len() {
        return invalid(ty, format!("total size mismatch: header {total_size}, actual {}", bytes.len()));
    }
    if total_size == NUMBER_SIZE {
        return Ok(Vec::new());
    }
    if bytes.len() < NUMBER_SIZE * 2 {
        return invalid(ty, "non-empty dynvec missing first offset");
    }

    let first_offset = unpack_number(&bytes[NUMBER_SIZE..], ty)?;
    if first_offset % NUMBER_SIZE != 0 || first_offset < NUMBER_SIZE * 2 || first_offset > bytes.len() {
        return invalid(ty, format!("invalid first item offset {first_offset}"));
    }

    let item_count = first_offset / NUMBER_SIZE - 1;
    let mut offsets = Vec::with_capacity(item_count + 1);
    for chunk in bytes[NUMBER_SIZE..first_offset].chunks_exact(NUMBER_SIZE) {
        offsets.push(unpack_number(chunk, ty)?);
    }
    offsets.push(total_size);

    if offsets.windows(2).any(|pair| pair[0] > pair[1]) {
        return invalid(ty, "item offsets are not monotonic");
    }
    if offsets.iter().any(|offset| *offset < first_offset || *offset > total_size) {
        return invalid(ty, "item offset is outside dynvec payload");
    }

    Ok(offsets.windows(2).map(|pair| &bytes[pair[0]..pair[1]]).collect())
}

fn encode_parents_by_level(levels: &[Vec<[u8; 32]>]) -> Vec<u8> {
    let encoded_levels = levels.iter().map(|level| encode_fixvec_byte32(level)).collect::<Vec<_>>();
    encode_dynvec(&encoded_levels)
}

fn decode_parents_by_level(bytes: &[u8]) -> Result<Vec<Vec<[u8; 32]>>, MoleculeError> {
    decode_dynvec(bytes, "ParentsByLevel")?.into_iter().map(|level| decode_fixvec_byte32(level, "ParentLevel")).collect()
}

fn decode_u32(bytes: &[u8], ty: &'static str) -> Result<u32, MoleculeError> {
    if bytes.len() != 4 {
        return invalid(ty, format!("expected 4 bytes, got {}", bytes.len()));
    }
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn decode_u64(bytes: &[u8], ty: &'static str) -> Result<u64, MoleculeError> {
    if bytes.len() != 8 {
        return invalid(ty, format!("expected 8 bytes, got {}", bytes.len()));
    }
    Ok(u64::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7]]))
}

fn decode_u128(bytes: &[u8], ty: &'static str) -> Result<u128, MoleculeError> {
    if bytes.len() != 16 {
        return invalid(ty, format!("expected 16 bytes, got {}", bytes.len()));
    }
    let mut out = [0u8; 16];
    out.copy_from_slice(bytes);
    Ok(u128::from_le_bytes(out))
}

fn decode_byte(bytes: &[u8], ty: &'static str) -> Result<u8, MoleculeError> {
    if bytes.len() != 1 {
        return invalid(ty, format!("expected 1 byte, got {}", bytes.len()));
    }
    Ok(bytes[0])
}

fn decode_array_32(bytes: &[u8], ty: &'static str) -> Result<[u8; 32], MoleculeError> {
    if bytes.len() != 32 {
        return invalid(ty, format!("expected 32 bytes, got {}", bytes.len()));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(bytes);
    Ok(out)
}

fn decode_array_24(bytes: &[u8], ty: &'static str) -> Result<[u8; 24], MoleculeError> {
    if bytes.len() != 24 {
        return invalid(ty, format!("expected 24 bytes, got {}", bytes.len()));
    }
    let mut out = [0u8; 24];
    out.copy_from_slice(bytes);
    Ok(out)
}

fn invalid<T>(ty: &'static str, reason: impl Into<String>) -> Result<T, MoleculeError> {
    Err(MoleculeError::InvalidFormat { ty, reason: reason.into() })
}

/// Schema constants for the handwritten Molecule layouts.
pub mod schema {
    /// ResolvedHeader schema version.
    pub const RESOLVED_HEADER_SCHEMA_VERSION: u8 = 1;

    /// ResolvedCell schema version.
    pub const RESOLVED_CELL_SCHEMA_VERSION: u8 = 1;

    /// Number of fields in the Spora Molecule `ResolvedHeader` table.
    pub const RESOLVED_HEADER_FIELD_COUNT: usize = 15;

    /// Number of fields in the Spora Molecule `ResolvedCell` table.
    pub const RESOLVED_CELL_FIELD_COUNT: usize = 2;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_script(tag: u8, args: Vec<u8>) -> Script {
        Script::new([tag; 32], tag, args)
    }

    fn sample_header() -> ResolvedHeader {
        ResolvedHeader {
            hash: [1; 32],
            version: 2,
            parents_by_level: vec![vec![[3; 32], [4; 32]], vec![[5; 32]]],
            hash_merkle_root: [6; 32],
            accepted_id_merkle_root: [7; 32],
            cell_commitment: [8; 32],
            cell_root: [9; 32],
            segment_root: [10; 32],
            timestamp: 11,
            bits: 12,
            nonce: 13,
            daa_score: 14,
            blue_work: [15; 24],
            blue_score: 16,
            pruning_point: [17; 32],
        }
    }

    fn sample_tx() -> CellTx {
        let inputs = vec![CellInput::new(OutPoint::new([0x11; 32], 7), 0x1122_3344_5566_7788)];
        let deps = vec![
            CellDep { out_point: OutPoint::new([0x22; 32], 0), dep_type: DepType::Code },
            CellDep { out_point: OutPoint::new([0x23; 32], 1), dep_type: DepType::DepGroup },
        ];
        let header_deps = vec![[0x33; 32], [0x34; 32]];
        let outputs = vec![
            CellOutput { lock: sample_script(1, vec![1, 2, 3]), type_: Some(sample_script(2, vec![4, 5])), capacity: 1000 },
            CellOutput { lock: sample_script(3, vec![]), type_: None, capacity: 2000 },
        ];
        let outputs_data = vec![vec![0xAA, 0xBB], vec![]];
        let witnesses = vec![vec![0xCC; 65], vec![0xDD, 0xEE]];
        CellTx::new_with_header_deps(inputs, deps, header_deps, outputs, outputs_data, witnesses).unwrap()
    }

    fn sample_ckb_raw_header() -> CkbRawHeader {
        CkbRawHeader {
            version: 0x0102_0304,
            compact_target: 0x0506_0708,
            timestamp: 0x1112_1314_1516_1718,
            number: 0x2122_2324_2526_2728,
            epoch: 0x3132_3334_3536_3738,
            parent_hash: [0x40; 32],
            transactions_root: [0x50; 32],
            proposals_hash: [0x60; 32],
            extra_hash: [0x70; 32],
            dao: [0x80; 32],
        }
    }

    #[test]
    fn molecule_is_available() {
        assert!(MoleculeSerializer::is_available());
        assert_eq!(MoleculeSerializer::abi_version(), 0x8001);
    }

    #[test]
    fn script_encoding_matches_ckb_molecule_default_layout() {
        let script = Script::new([0; 32], 0, vec![]);
        let bytes = serialize_script_molecule(&script).unwrap();

        assert_eq!(
            bytes,
            vec![
                53, 0, 0, 0, 16, 0, 0, 0, 48, 0, 0, 0, 49, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            ]
        );
        assert_eq!(deserialize_script_molecule(&bytes).unwrap(), script);
    }

    #[test]
    fn ckb_script_hash_uses_blake2b_over_packed_molecule() {
        let script = Script::new([0; 32], 0, vec![]);
        assert_eq!(
            ckb_script_hash_molecule(&script).unwrap(),
            [
                0x77, 0xc9, 0x3b, 0x06, 0x32, 0xb5, 0xb6, 0xc3, 0xef, 0x92, 0x2c, 0x5b, 0x7c, 0xea, 0x20, 0x8f, 0xb0, 0xa7, 0xc4,
                0x27, 0xa1, 0x3d, 0x50, 0xe1, 0x3d, 0x3f, 0xef, 0xad, 0x17, 0xe0, 0xc5, 0x90,
            ]
        );
    }

    #[test]
    fn ckb_blake160_uses_first_20_bytes_of_ckb_blake2b() {
        let pubkey = [0x03; 33];
        assert_eq!(
            ckb_blake160(&pubkey),
            [0xb5, 0x09, 0x50, 0x0e, 0x3a, 0x3c, 0x12, 0x54, 0x13, 0xaf, 0x45, 0xd2, 0x05, 0x03, 0xf9, 0xc0, 0x8a, 0xe8, 0x62, 0x45,]
        );
        assert_eq!(ckb_secp256k1_blake160_pubkey_hash(&pubkey), ckb_blake160(&pubkey));
        assert_ne!(ckb_blake160(&pubkey), crate::celltx::sighash::pubkey_hash(&pubkey));
    }

    #[test]
    fn ckb_standard_lock_config_builds_type_hash_lock_and_dep_group_dep() {
        let type_hash = [0x42; 32];
        let dep_group_out_point = OutPoint::new([0x99; 32], 7);
        let pubkey_hash = [0x11; CKB_SECP256K1_BLAKE160_LOCK_ARG_SIZE];
        let config = CkbSecp256k1Blake160SighashAllLockConfig::with_dep_group(type_hash, dep_group_out_point);

        assert_eq!(config.cell_dep(), CellDep { out_point: dep_group_out_point, dep_type: DepType::DepGroup });

        let lock = config.lock_script(&pubkey_hash);
        assert_eq!(lock, ckb_secp256k1_blake160_sighash_all_lock_script(type_hash, &pubkey_hash));
        assert_eq!(lock.code_hash, type_hash);
        assert_eq!(lock.hash_type, CKB_SCRIPT_HASH_TYPE_TYPE);
        assert_eq!(lock.args, pubkey_hash.to_vec());
    }

    #[test]
    fn ckb_type_id_script_uses_builtin_code_hash_and_type_hash_type() {
        let args = [0xA7; 32];
        let script = ckb_type_id_script(&args);

        assert_eq!(
            CKB_TYPE_ID_CODE_HASH,
            [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, b'T', b'Y', b'P', b'E', b'_', b'I', b'D',]
        );
        assert_eq!(script.code_hash, CKB_TYPE_ID_CODE_HASH);
        assert_eq!(script.hash_type, CKB_SCRIPT_HASH_TYPE_TYPE);
        assert_eq!(script.args, args.to_vec());
    }

    #[test]
    fn ckb_secp256k1_blake160_recoverable_signature_verification_uses_ckb_pubkey_hash() {
        let secp = secp256k1::Secp256k1::new();
        let secret_key = secp256k1::SecretKey::from_slice(&[0x42; 32]).expect("secret key");
        let pubkey = secp256k1::PublicKey::from_secret_key(&secp, &secret_key);
        let message_hash = ckb_blake2b_256(b"ckb signing message");
        let message = secp256k1::Message::from_digest_slice(&message_hash).expect("message");
        let signature = secp.sign_ecdsa_recoverable(&message, &secret_key);
        let (recovery_id, compact) = signature.serialize_compact();
        let mut signature_bytes = [0u8; CKB_SECP256K1_SIGHASH_ALL_SIGNATURE_SIZE];
        signature_bytes[..64].copy_from_slice(&compact);
        signature_bytes[64] = recovery_id.to_i32() as u8;

        let ckb_pubkey_hash = ckb_secp256k1_blake160_pubkey_hash(&pubkey.serialize());
        assert!(ckb_verify_secp256k1_blake160_recoverable_signature(&ckb_pubkey_hash, &signature_bytes, &message_hash));

        let spora_pubkey_hash = crate::celltx::sighash::pubkey_hash(&pubkey.serialize());
        assert!(!ckb_verify_secp256k1_blake160_recoverable_signature(&spora_pubkey_hash, &signature_bytes, &message_hash));

        signature_bytes[64] = 4;
        assert!(!ckb_verify_secp256k1_blake160_recoverable_signature(&ckb_pubkey_hash, &signature_bytes, &message_hash));
    }

    #[test]
    fn ckb_secp256k1_blake160_sighash_all_verifies_zeroed_lock_witness() {
        let tx = sample_tx();
        let secp = secp256k1::Secp256k1::new();
        let secret_key = secp256k1::SecretKey::from_slice(&[0x24; 32]).expect("secret key");
        let pubkey = secp256k1::PublicKey::from_secret_key(&secp, &secret_key);
        let pubkey_hash = ckb_secp256k1_blake160_pubkey_hash(&pubkey.serialize());
        let extra = b"extra-group-witness";

        let unsigned_witness = CkbWitnessArgs::new(Some(vec![0u8; CKB_SECP256K1_SIGHASH_ALL_SIGNATURE_SIZE]), Some(vec![0x10]), None);
        let message_hash = ckb_sighash_all_message_from_witness_args_molecule(&tx, &unsigned_witness, &[extra.as_ref()]).unwrap();
        let message = secp256k1::Message::from_digest_slice(&message_hash).expect("message");
        let signature = secp.sign_ecdsa_recoverable(&message, &secret_key);
        let (recovery_id, compact) = signature.serialize_compact();
        let mut signature_bytes = [0u8; CKB_SECP256K1_SIGHASH_ALL_SIGNATURE_SIZE];
        signature_bytes[..64].copy_from_slice(&compact);
        signature_bytes[64] = recovery_id.to_i32() as u8;

        let signed_witness = CkbWitnessArgs::new(Some(signature_bytes.to_vec()), Some(vec![0x10]), None);
        assert!(
            ckb_verify_secp256k1_blake160_sighash_all_molecule(&pubkey_hash, &tx, &signed_witness, &[extra.as_ref()]).unwrap(),
            "verification must zero the lock field before hashing the signing witness"
        );

        let mut wrong_pubkey_hash = pubkey_hash;
        wrong_pubkey_hash[0] ^= 0xFF;
        assert!(
            !ckb_verify_secp256k1_blake160_sighash_all_molecule(&wrong_pubkey_hash, &tx, &signed_witness, &[extra.as_ref()]).unwrap()
        );

        let mut tampered_witness = signed_witness.clone();
        tampered_witness.lock.as_mut().unwrap()[0] ^= 0xFF;
        assert!(!ckb_verify_secp256k1_blake160_sighash_all_molecule(&pubkey_hash, &tx, &tampered_witness, &[extra.as_ref()]).unwrap());
        assert!(!ckb_verify_secp256k1_blake160_sighash_all_molecule(&pubkey_hash, &tx, &CkbWitnessArgs::default(), &[extra.as_ref()])
            .unwrap());
    }

    #[test]
    fn ckb_secp256k1_blake160_sighash_all_signing_roundtrips_through_verifier() {
        let tx = sample_tx();
        let secp = secp256k1::Secp256k1::new();
        let secret_key = secp256k1::SecretKey::from_slice(&[0x21; 32]).expect("secret key");
        let pubkey = secp256k1::PublicKey::from_secret_key(&secp, &secret_key);
        let pubkey_hash = ckb_secp256k1_blake160_pubkey_hash(&pubkey.serialize());
        let extra = b"extra-group-witness";
        let signing_witness = CkbWitnessArgs::new(Some(vec![0xAA; 65]), Some(vec![0x10, 0x11]), Some(vec![0x20]));

        let signed_witness =
            ckb_sign_secp256k1_blake160_sighash_all_molecule(&tx, &signing_witness, &secret_key, &[extra.as_ref()]).unwrap();

        assert_eq!(signed_witness.lock.as_deref().map(|lock| lock.len()), Some(CKB_SECP256K1_SIGHASH_ALL_SIGNATURE_SIZE));
        assert_eq!(signed_witness.input_type, signing_witness.input_type);
        assert_eq!(signed_witness.output_type, signing_witness.output_type);
        assert!(ckb_verify_secp256k1_blake160_sighash_all_molecule(&pubkey_hash, &tx, &signed_witness, &[extra.as_ref()]).unwrap());

        let zeroed_message = ckb_sighash_all_message_with_zeroed_witness_lock_molecule(
            &tx,
            &signed_witness,
            CKB_SECP256K1_SIGHASH_ALL_SIGNATURE_SIZE,
            &[extra.as_ref()],
        )
        .unwrap();
        let signed_message = ckb_sighash_all_message_from_witness_args_molecule(&tx, &signed_witness, &[extra.as_ref()]).unwrap();
        assert_ne!(signed_message, zeroed_message, "CKB signs the zeroed-lock witness, not the final signature bytes");
    }

    #[test]
    fn ckb_secp256k1_blake160_sighash_all_input_helper_places_molecule_witness() {
        let mut tx = sample_tx();
        let existing_extra_witness = tx.witnesses[1].clone();
        let secp = secp256k1::Secp256k1::new();
        let secret_key = secp256k1::SecretKey::from_slice(&[0x22; 32]).expect("secret key");
        let pubkey = secp256k1::PublicKey::from_secret_key(&secp, &secret_key);
        let pubkey_hash = ckb_secp256k1_blake160_pubkey_hash(&pubkey.serialize());
        let extra = b"group-extra-witness";
        let signing_witness = CkbWitnessArgs::new(None, Some(vec![0x77]), Some(vec![0x88, 0x99]));

        let signed_witness =
            ckb_sign_secp256k1_blake160_sighash_all_input_molecule(&mut tx, 0, &signing_witness, &secret_key, &[extra.as_ref()])
                .unwrap();

        assert_eq!(tx.witnesses[1], existing_extra_witness, "non-signing witness slots must be preserved");
        let placed_witness = deserialize_ckb_witness_args_molecule(&tx.witnesses[0]).unwrap();
        assert_eq!(placed_witness, signed_witness);
        assert_eq!(placed_witness.input_type, Some(vec![0x77]));
        assert_eq!(placed_witness.output_type, Some(vec![0x88, 0x99]));
        assert!(ckb_verify_secp256k1_blake160_sighash_all_molecule(&pubkey_hash, &tx, &placed_witness, &[extra.as_ref()]).unwrap());
        assert!(ckb_sign_secp256k1_blake160_sighash_all_input_molecule(&mut tx, 99, &signing_witness, &secret_key, &[]).is_err());
    }

    #[test]
    fn ckb_secp256k1_blake160_sighash_all_lock_group_helper_discovers_group_witnesses() {
        let target_lock = sample_script(0x61, vec![0x01]);
        let other_lock = sample_script(0x62, vec![0x02]);
        let inputs = vec![
            CellInput::new(OutPoint::new([0x31; 32], 0), 0),
            CellInput::new(OutPoint::new([0x32; 32], 0), 0),
            CellInput::new(OutPoint::new([0x33; 32], 0), 0),
        ];
        let outputs = vec![CellOutput { lock: other_lock.clone(), type_: None, capacity: 1_000 }];
        let outputs_data = vec![vec![]];
        let witnesses = vec![vec![0xA0], vec![0xB1], vec![0xC2, 0xC3]];
        let mut tx = CellTx::new(inputs, vec![], outputs, outputs_data, witnesses).unwrap();
        let resolved_inputs = vec![
            CellOutput { lock: other_lock.clone(), type_: None, capacity: 100 },
            CellOutput { lock: target_lock.clone(), type_: None, capacity: 200 },
            CellOutput { lock: target_lock.clone(), type_: None, capacity: 300 },
        ];
        let second_group_witness = tx.witnesses[2].clone();
        let secp = secp256k1::Secp256k1::new();
        let secret_key = secp256k1::SecretKey::from_slice(&[0x23; 32]).expect("secret key");
        let pubkey = secp256k1::PublicKey::from_secret_key(&secp, &secret_key);
        let pubkey_hash = ckb_secp256k1_blake160_pubkey_hash(&pubkey.serialize());
        let tail_witness = b"tail-witness";
        let signing_witness = CkbWitnessArgs::new(None, Some(vec![0x44]), None);

        let signed_witness = ckb_sign_secp256k1_blake160_sighash_all_lock_group_molecule(
            &mut tx,
            &resolved_inputs,
            &target_lock,
            &signing_witness,
            &secret_key,
            &[tail_witness.as_ref()],
        )
        .unwrap();

        assert_eq!(tx.witnesses[0], vec![0xA0], "non-group witnesses must be preserved");
        assert_eq!(tx.witnesses[2], second_group_witness, "non-signing group witness must be committed as extra, not overwritten");
        assert_eq!(deserialize_ckb_witness_args_molecule(&tx.witnesses[1]).unwrap(), signed_witness);
        assert!(
            ckb_verify_secp256k1_blake160_sighash_all_lock_group_molecule(
                &pubkey_hash,
                &tx,
                &resolved_inputs,
                &target_lock,
                &[tail_witness.as_ref()]
            )
            .unwrap(),
            "lock-group verifier must commit the second group witness and tail witness"
        );
        assert!(
            !ckb_verify_secp256k1_blake160_sighash_all_lock_group_molecule(&pubkey_hash, &tx, &resolved_inputs, &target_lock, &[])
                .unwrap(),
            "omitting the tail witness changes the CKB sighash-all message"
        );
        assert!(ckb_sign_secp256k1_blake160_sighash_all_lock_group_molecule(
            &mut tx,
            &resolved_inputs,
            &sample_script(0x63, vec![]),
            &signing_witness,
            &secret_key,
            &[],
        )
        .is_err());
        assert!(ckb_verify_secp256k1_blake160_sighash_all_lock_group_molecule(
            &pubkey_hash,
            &tx,
            &resolved_inputs[..2],
            &target_lock,
            &[]
        )
        .is_err());
    }

    #[test]
    fn ckb_style_core_types_roundtrip() {
        let outpoint = OutPoint::new([0xAA; 32], 7);
        let input = CellInput::new(outpoint, 0x1122_3344_5566_7788);
        let dep = CellDep { out_point: OutPoint::new([0xBB; 32], 1), dep_type: DepType::DepGroup };
        let lock = sample_script(1, vec![1, 2, 3]);
        let type_ = sample_script(2, vec![4, 5]);
        let output = CellOutput { lock, type_: Some(type_), capacity: 1000 };

        assert_eq!(deserialize_outpoint_molecule(&serialize_outpoint_molecule(&outpoint).unwrap()).unwrap(), outpoint);
        assert_eq!(deserialize_cell_input_molecule(&serialize_cell_input_molecule(&input).unwrap()).unwrap(), input);
        assert_eq!(deserialize_cell_dep_molecule(&serialize_cell_dep_molecule(&dep).unwrap()).unwrap(), dep);
        assert_eq!(deserialize_cell_output_molecule(&serialize_cell_output_molecule(&output).unwrap()).unwrap(), output);
    }

    #[test]
    fn cell_dep_encoding_matches_ckb_molecule_struct_layout() {
        let dep = CellDep { out_point: OutPoint::new([0xAB; 32], 9), dep_type: DepType::DepGroup };
        let bytes = serialize_cell_dep_molecule(&dep).unwrap();

        let mut expected = Vec::new();
        expected.extend_from_slice(&[0xAB; 32]);
        expected.extend_from_slice(&9u32.to_le_bytes());
        expected.push(1);
        assert_eq!(bytes, expected);
        assert!(deserialize_cell_dep_molecule(&[0u8; 36]).is_err());
        let mut invalid_dep_type = expected;
        invalid_dep_type[36] = 2;
        assert!(deserialize_cell_dep_molecule(&invalid_dep_type).is_err());
    }

    #[test]
    fn ckb_outpoint_vec_dep_group_uses_molecule_fixvec_and_rejects_empty() {
        let outpoints = vec![OutPoint::new([0x11; 32], 0), OutPoint::new([0x22; 32], 7)];
        let bytes = serialize_ckb_outpoint_vec_molecule(&outpoints).unwrap();

        let mut expected = Vec::new();
        expected.extend_from_slice(&2u32.to_le_bytes());
        expected.extend_from_slice(&[0x11; 32]);
        expected.extend_from_slice(&0u32.to_le_bytes());
        expected.extend_from_slice(&[0x22; 32]);
        expected.extend_from_slice(&7u32.to_le_bytes());
        assert_eq!(bytes, expected);
        assert_eq!(deserialize_ckb_outpoint_vec_molecule(&bytes).unwrap(), outpoints);
        assert!(serialize_ckb_outpoint_vec_molecule(&[]).is_err());
        assert!(deserialize_ckb_outpoint_vec_molecule(&0u32.to_le_bytes()).is_err());
    }

    #[test]
    fn ckb_type_id_args_match_parent_ckb_hash_material() {
        let input = CellInput::new(OutPoint::new([0xAB; 32], 7), 0x1122_3344_5566_7788);
        let args = ckb_type_id_args(&input, 3).unwrap();

        assert_eq!(
            args,
            [
                0x3e, 0x6a, 0x88, 0x81, 0x6a, 0xe7, 0x5b, 0xd0, 0xce, 0x5f, 0x4f, 0x0f, 0x0e, 0x52, 0xfd, 0xbf, 0x61, 0x9a, 0xc0,
                0xd3, 0x5a, 0xaa, 0xa4, 0xcc, 0xb1, 0x98, 0x3e, 0x92, 0xbf, 0xf7, 0x58, 0x64,
            ]
        );
    }

    #[test]
    fn ckb_type_id_builder_helper_applies_creation_args_to_output_type_script() {
        let mut tx = sample_tx();
        tx.outputs[1].type_ = Some(sample_script(0x55, vec![]));

        let args = ckb_apply_type_id_args_to_output_molecule(&mut tx, 1).unwrap();
        assert_eq!(args, ckb_type_id_args(&tx.inputs[0], 1).unwrap());
        let script = tx.outputs[1].type_.clone().unwrap();
        assert_eq!(script.args, args.to_vec());
        ckb_verify_type_id_script_group_molecule(&tx, &script.args, &[], &[1]).unwrap();

        assert!(ckb_apply_type_id_args_to_output_molecule(&mut tx, 99).is_err());
        let mut missing_type_tx = sample_tx();
        assert!(ckb_apply_type_id_args_to_output_molecule(&mut missing_type_tx, 1).is_err());
        let mut no_input_tx = CellTx::new_with_header_deps(
            vec![],
            vec![],
            vec![],
            vec![CellOutput { lock: sample_script(0x44, vec![]), type_: Some(sample_script(0x55, vec![])), capacity: 1000 }],
            vec![vec![]],
            vec![],
        )
        .unwrap();
        assert!(ckb_apply_type_id_args_to_output_molecule(&mut no_input_tx, 0).is_err());

        let mut type_script_tx = sample_tx();
        type_script_tx.outputs[1].type_ = None;
        let type_script_args = ckb_apply_type_id_script_to_output_molecule(&mut type_script_tx, 1).unwrap();
        assert_eq!(type_script_args, ckb_type_id_args(&type_script_tx.inputs[0], 1).unwrap());
        let type_script = type_script_tx.outputs[1].type_.clone().unwrap();
        assert_eq!(type_script, ckb_type_id_script(&type_script_args));
        ckb_verify_type_id_script_group_molecule(&type_script_tx, &type_script.args, &[], &[1]).unwrap();
    }

    #[test]
    fn ckb_type_id_script_group_verifier_matches_parent_ckb_rules() {
        let tx = sample_tx();
        let creation_args = ckb_type_id_args(&tx.inputs[0], 1).unwrap();

        ckb_verify_type_id_script_group_molecule(&tx, &creation_args, &[], &[1]).unwrap();
        ckb_verify_type_id_script_group_molecule(&tx, &[0xAA; 32], &[0], &[0]).unwrap();
        ckb_verify_type_id_script_group_molecule(&tx, &[0xAA; 32], &[0], &[]).unwrap();

        let mut wrong_creation_args = creation_args;
        wrong_creation_args[0] ^= 0xFF;
        assert!(ckb_verify_type_id_script_group_molecule(&tx, &wrong_creation_args, &[], &[1]).is_err());
        assert!(ckb_verify_type_id_script_group_molecule(&tx, &[0xAA; 31], &[], &[1]).is_err());
        assert!(ckb_verify_type_id_script_group_molecule(&tx, &[0xAA; 32], &[0, 1], &[1]).is_err());
        assert!(ckb_verify_type_id_script_group_molecule(&tx, &[0xAA; 32], &[0], &[0, 1]).is_err());
        assert!(ckb_verify_type_id_script_group_molecule(&tx, &[0xAA; 32], &[], &[]).is_err());
        assert!(ckb_verify_type_id_script_group_molecule(&tx, &[0xAA; 32], &[99], &[]).is_err());
        assert!(ckb_verify_type_id_script_group_molecule(&tx, &[0xAA; 32], &[], &[99]).is_err());

        let no_input_tx = CellTx::new_with_header_deps(
            vec![],
            vec![],
            vec![],
            vec![CellOutput { lock: sample_script(0x44, vec![]), type_: None, capacity: 1000 }],
            vec![vec![]],
            vec![],
        )
        .unwrap();
        assert!(ckb_verify_type_id_script_group_molecule(&no_input_tx, &[0xAA; 32], &[], &[0]).is_err());
    }

    #[test]
    fn ckb_type_id_script_verifier_discovers_groups_from_resolved_cells() {
        let mut creation_tx = sample_tx();
        let creation_args = ckb_type_id_args(&creation_tx.inputs[0], 1).unwrap();
        let creation_script = sample_script(0x55, creation_args.to_vec());
        creation_tx.outputs[1].type_ = Some(creation_script.clone());
        let resolved_inputs = vec![CellOutput { lock: sample_script(0x10, vec![]), type_: None, capacity: 500 }];

        ckb_verify_type_id_script_molecule(&creation_tx, &resolved_inputs, &creation_script).unwrap();

        let continued_script = sample_script(0x56, [0xAA; 32].to_vec());
        let mut continuation_tx = sample_tx();
        continuation_tx.outputs[0].type_ = Some(continued_script.clone());
        let continuation_inputs =
            vec![CellOutput { lock: sample_script(0x11, vec![]), type_: Some(continued_script.clone()), capacity: 600 }];

        ckb_verify_type_id_script_molecule(&continuation_tx, &continuation_inputs, &continued_script).unwrap();

        continuation_tx.outputs[1].type_ = Some(continued_script.clone());
        assert!(ckb_verify_type_id_script_molecule(&continuation_tx, &continuation_inputs, &continued_script).is_err());
        assert!(ckb_verify_type_id_script_molecule(&creation_tx, &[], &creation_script).is_err());
    }

    #[test]
    fn ckb_cell_data_hash_matches_ckb_empty_special_case() {
        assert_eq!(ckb_cell_data_hash(&[]), [0u8; 32]);
        assert_eq!(ckb_cell_data_hash(b"spora"), ckb_blake2b_256(b"spora"));
        assert_ne!(ckb_blake2b_256(&[]), [0u8; 32]);
    }

    #[test]
    fn ckb_transaction_molecule_roundtrip_preserves_vectors() {
        let tx = sample_tx();

        let raw_bytes = serialize_raw_transaction_molecule(&tx).unwrap();
        let decoded_raw = deserialize_raw_transaction_molecule(&raw_bytes).unwrap();
        let mut expected_raw = tx.clone();
        expected_raw.witnesses.clear();
        assert_eq!(decoded_raw, expected_raw);

        let tx_bytes = serialize_transaction_molecule(&tx).unwrap();
        assert_eq!(deserialize_transaction_molecule(&tx_bytes).unwrap(), tx);
    }

    #[test]
    fn ckb_header_struct_layout_and_hashes_match_ckb_molecule() {
        let raw = sample_ckb_raw_header();
        let header = CkbHeader { raw: raw.clone(), nonce: 0x4142_4344_4546_4748_5152_5354_5556_5758 };

        let raw_bytes = serialize_ckb_raw_header_molecule(&raw).unwrap();
        assert_eq!(raw_bytes.len(), 192);
        assert_eq!(&raw_bytes[0..4], &raw.version.to_le_bytes());
        assert_eq!(&raw_bytes[4..8], &raw.compact_target.to_le_bytes());
        assert_eq!(&raw_bytes[8..16], &raw.timestamp.to_le_bytes());
        assert_eq!(&raw_bytes[16..24], &raw.number.to_le_bytes());
        assert_eq!(&raw_bytes[24..32], &raw.epoch.to_le_bytes());
        assert_eq!(&raw_bytes[32..64], &raw.parent_hash);
        assert_eq!(&raw_bytes[64..96], &raw.transactions_root);
        assert_eq!(&raw_bytes[96..128], &raw.proposals_hash);
        assert_eq!(&raw_bytes[128..160], &raw.extra_hash);
        assert_eq!(&raw_bytes[160..192], &raw.dao);
        assert_eq!(deserialize_ckb_raw_header_molecule(&raw_bytes).unwrap(), raw);
        assert!(deserialize_ckb_raw_header_molecule(&raw_bytes[..191]).is_err());

        let header_bytes = serialize_ckb_header_molecule(&header).unwrap();
        assert_eq!(header_bytes.len(), 208);
        assert_eq!(&header_bytes[..192], raw_bytes.as_slice());
        assert_eq!(&header_bytes[192..208], &header.nonce.to_le_bytes());
        assert_eq!(deserialize_ckb_header_molecule(&header_bytes).unwrap(), header);
        assert!(deserialize_ckb_header_molecule(&header_bytes[..207]).is_err());

        assert_eq!(ckb_header_hash_molecule(&header).unwrap(), ckb_blake2b_256(&header_bytes));
    }

    #[test]
    fn ckb_epoch_number_with_fraction_matches_parent_ckb_bit_layout() {
        let full_value = ckb_epoch_number_with_fraction_full_value(1, 40, 1000).unwrap();
        assert_eq!(full_value, (1000u64 << 40) | (40u64 << 24) | 1);

        let epoch = ckb_epoch_number_with_fraction_from_full_value(full_value);
        assert_eq!(epoch.full_value, full_value);
        assert_eq!(epoch.number, 1);
        assert_eq!(epoch.index, 40);
        assert_eq!(epoch.length, 1000);

        let mut raw = sample_ckb_raw_header();
        raw.number = 1234;
        raw.epoch = full_value;
        assert_eq!(ckb_header_epoch_number(&raw), 1);
        assert_eq!(ckb_header_epoch_index(&raw), 40);
        assert_eq!(ckb_header_epoch_length(&raw), 1000);
        assert_eq!(ckb_header_epoch_start_block_number(&raw).unwrap(), 1194);

        raw.number = 39;
        assert!(ckb_header_epoch_start_block_number(&raw).is_err());
        assert!(ckb_epoch_number_with_fraction_full_value(1 << 24, 0, 1).is_err());
        assert!(ckb_epoch_number_with_fraction_full_value(0, 1 << 16, 1).is_err());
        assert!(ckb_epoch_number_with_fraction_full_value(0, 0, 0).is_err());
        assert!(ckb_epoch_number_with_fraction_full_value(0, 0, 1 << 16).is_err());
    }

    #[test]
    fn ckb_raw_tx_hash_ignores_witnesses_but_transaction_witness_hash_commits_them() {
        let tx = sample_tx();
        let mut witness_variant = tx.clone();
        witness_variant.witnesses[0][0] ^= 0xFF;

        assert_eq!(
            ckb_raw_transaction_hash_molecule(&tx).unwrap(),
            ckb_blake2b_256(&serialize_raw_transaction_molecule(&tx).unwrap())
        );
        assert_eq!(ckb_raw_transaction_hash_molecule(&tx).unwrap(), ckb_raw_transaction_hash_molecule(&witness_variant).unwrap());
        assert_ne!(
            ckb_transaction_witness_hash_molecule(&tx).unwrap(),
            ckb_transaction_witness_hash_molecule(&witness_variant).unwrap()
        );
        assert_ne!(
            serialize_raw_transaction_molecule(&tx).unwrap(),
            serialize_transaction_molecule(&tx).unwrap(),
            "RawTransaction and Transaction are distinct CKB Molecule tables"
        );
    }

    #[test]
    fn ckb_sighash_all_message_matches_parent_ckb_update_order() {
        let tx = sample_tx();
        let signing_witness = [0x11u8; 65];
        let extra_a = b"group-witness";
        let extra_b = b"non-signing-witness";

        let raw_hash = ckb_raw_transaction_hash_molecule(&tx).unwrap();
        let signing_len = (signing_witness.len() as u64).to_le_bytes();
        let extra_a_len = (extra_a.len() as u64).to_le_bytes();
        let extra_b_len = (extra_b.len() as u64).to_le_bytes();
        let expected = ckb_blake2b_256_chunks(&[
            raw_hash.as_ref(),
            signing_len.as_ref(),
            signing_witness.as_ref(),
            extra_a_len.as_ref(),
            extra_a.as_ref(),
            extra_b_len.as_ref(),
            extra_b.as_ref(),
        ]);

        assert_eq!(ckb_sighash_all_message_molecule(&tx, &signing_witness, &[extra_a.as_ref(), extra_b.as_ref()]).unwrap(), expected);
    }

    #[test]
    fn ckb_witness_args_matches_ckb_molecule_table_layout() {
        let empty = CkbWitnessArgs::default();
        let empty_bytes = serialize_ckb_witness_args_molecule(&empty).unwrap();
        assert_eq!(empty_bytes, vec![16, 0, 0, 0, 16, 0, 0, 0, 16, 0, 0, 0, 16, 0, 0, 0]);
        assert_eq!(deserialize_ckb_witness_args_molecule(&empty_bytes).unwrap(), empty);

        let witness = CkbWitnessArgs::new(Some(vec![0x11; 65]), None, Some(vec![0x22, 0x33]));
        let bytes = serialize_ckb_witness_args_molecule(&witness).unwrap();
        assert_eq!(deserialize_ckb_witness_args_molecule(&bytes).unwrap(), witness);

        let fields = decode_table(&bytes, 3, "WitnessArgs").unwrap();
        assert_eq!(decode_bytes(fields[0]).unwrap(), vec![0x11; 65]);
        assert!(fields[1].is_empty());
        assert_eq!(decode_bytes(fields[2]).unwrap(), vec![0x22, 0x33]);

        let malformed = encode_table(&[vec![3, 0, 0, 0, 0xAA], vec![], vec![]]);
        assert!(deserialize_ckb_witness_args_molecule(&malformed).is_err());
    }

    #[test]
    fn ckb_sighash_all_message_accepts_typed_witness_args() {
        let tx = sample_tx();
        let witness = CkbWitnessArgs::new(Some(vec![0u8; 65]), Some(vec![0xAA]), None);
        let witness_bytes = serialize_ckb_witness_args_molecule(&witness).unwrap();
        let extra = b"extra-witness";

        assert_eq!(
            ckb_sighash_all_message_from_witness_args_molecule(&tx, &witness, &[extra.as_ref()]).unwrap(),
            ckb_sighash_all_message_molecule(&tx, &witness_bytes, &[extra.as_ref()]).unwrap()
        );
    }

    #[test]
    fn ckb_sighash_all_message_zeroes_witness_lock_field() {
        let tx = sample_tx();
        let witness_a = CkbWitnessArgs::new(Some(vec![0xAA; 65]), Some(vec![0x10]), Some(vec![0x20, 0x21]));
        let witness_b = CkbWitnessArgs::new(Some(vec![0xBB; 65]), Some(vec![0x10]), Some(vec![0x20, 0x21]));
        let extra = b"extra-witness";

        let zeroed =
            CkbWitnessArgs::new(Some(vec![0u8; CKB_SECP256K1_SIGHASH_ALL_SIGNATURE_SIZE]), Some(vec![0x10]), Some(vec![0x20, 0x21]));
        let expected = ckb_sighash_all_message_from_witness_args_molecule(&tx, &zeroed, &[extra.as_ref()]).unwrap();

        assert_eq!(
            ckb_sighash_all_message_with_zeroed_witness_lock_molecule(
                &tx,
                &witness_a,
                CKB_SECP256K1_SIGHASH_ALL_SIGNATURE_SIZE,
                &[extra.as_ref()]
            )
            .unwrap(),
            expected
        );
        assert_eq!(
            ckb_sighash_all_message_with_zeroed_witness_lock_molecule(
                &tx,
                &witness_b,
                CKB_SECP256K1_SIGHASH_ALL_SIGNATURE_SIZE,
                &[extra.as_ref()]
            )
            .unwrap(),
            expected,
            "actual signature bytes must not be committed into the signable witness"
        );

        assert_ne!(
            ckb_sighash_all_message_from_witness_args_molecule(&tx, &witness_a, &[extra.as_ref()]).unwrap(),
            expected,
            "the helper must differ from hashing the signature-filled witness directly"
        );
    }

    #[test]
    fn resolved_cell_roundtrip_preserves_optional_data() {
        let cell = ResolvedCell {
            cell_output: CellOutput { lock: sample_script(3, vec![8, 9]), type_: None, capacity: 42 },
            data: Some(vec![0xAB, 0xCD]),
        };
        let bytes = serialize_resolved_cell_molecule(&cell).unwrap();
        assert_eq!(deserialize_resolved_cell_molecule(&bytes).unwrap(), cell);
    }

    #[test]
    fn resolved_header_roundtrip_preserves_dag_parent_levels() {
        let header = sample_header();
        let bytes = serialize_resolved_header_molecule(&header).unwrap();
        assert_eq!(deserialize_resolved_header_molecule(&bytes).unwrap(), header);
    }

    #[test]
    fn rejects_malformed_table_size() {
        let mut bytes = serialize_script_molecule(&sample_script(1, vec![])).unwrap();
        bytes[0] = bytes[0].wrapping_add(1);
        let err = deserialize_script_molecule(&bytes).unwrap_err();
        assert!(matches!(err, MoleculeError::InvalidFormat { .. }));
    }
}
