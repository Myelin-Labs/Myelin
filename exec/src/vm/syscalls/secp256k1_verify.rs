// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers
//
// Secp256k1 verification syscall (Myelin-specific extension to CKB-VM)

use super::{SECP256K1_VERIFY_SYSCALL_NUMBER, SUCCESS};
use crate::vm::transferred_byte_cycles;
use ckb_vm::{
    registers::{A0, A1, A2, A7},
    Error as VMError, Memory, Register, SupportMachine, Syscalls,
};
use secp256k1::{ecdsa::RecoverableSignature, Message, Secp256k1};

const PUBKEY_HASH_LEN: usize = 20;
const SIGNATURE_LEN: usize = 65;
const MESSAGE_HASH_LEN: usize = 32;
const SECP256K1_VERIFY_FAILED: u8 = 1;
pub const SECP256K1_VERIFY_BASE_CYCLES: u64 = 50_000;

/// Syscall: Secp256k1 recover + pubkey hash verification
///
/// Syscall number: 3002 (Myelin extension, not in CKB)
///
/// Args:
/// - A0: expected pubkey hash pointer (20 bytes, blake3(pubkey)[0..20])
/// - A1: recoverable signature pointer (65 bytes, r||s||v)
/// - A2: message hash pointer (32 bytes)
///
/// Returns:
/// - A0: 0 on success, 1 on verification failure
pub struct Secp256k1Verify;

impl Secp256k1Verify {
    pub fn new() -> Self {
        Self
    }

    fn verify_signature(expected_pubkey_hash: &[u8], signature: &[u8], message_hash: &[u8]) -> bool {
        if expected_pubkey_hash.len() != PUBKEY_HASH_LEN || signature.len() != SIGNATURE_LEN || message_hash.len() != MESSAGE_HASH_LEN
        {
            return false;
        }

        if signature[64] > 3 {
            return false;
        }
        let recovery_id = match secp256k1::ecdsa::RecoveryId::from_i32(signature[64] as i32) {
            Ok(value) => value,
            Err(_) => return false,
        };

        let recoverable_signature = match RecoverableSignature::from_compact(&signature[..64], recovery_id) {
            Ok(value) => value,
            Err(_) => return false,
        };
        let standard_signature = recoverable_signature.to_standard();
        let mut normalized = standard_signature;
        normalized.normalize_s();
        if normalized != standard_signature {
            return false;
        }

        let message = match Message::from_digest_slice(message_hash) {
            Ok(value) => value,
            Err(_) => return false,
        };

        let secp = Secp256k1::new();
        let recovered_pubkey = match secp.recover_ecdsa(&message, &recoverable_signature) {
            Ok(value) => value,
            Err(_) => return false,
        };

        let computed_hash = blake3::hash(&recovered_pubkey.serialize());
        &computed_hash.as_bytes()[..PUBKEY_HASH_LEN] == expected_pubkey_hash
    }
}

impl<M: SupportMachine> Syscalls<M> for Secp256k1Verify {
    fn initialize(&mut self, _machine: &mut M) -> Result<(), VMError> {
        Ok(())
    }

    fn ecall(&mut self, machine: &mut M) -> Result<bool, VMError> {
        let syscall_number = machine.registers()[A7].to_u64();
        if syscall_number != SECP256K1_VERIFY_SYSCALL_NUMBER {
            return Ok(false);
        }

        let expected_pubkey_hash_addr = machine.registers()[A0].to_u64();
        let signature_addr = machine.registers()[A1].to_u64();
        let message_hash_addr = machine.registers()[A2].to_u64();

        let expected_pubkey_hash = machine.memory_mut().load_bytes(expected_pubkey_hash_addr, PUBKEY_HASH_LEN as u64)?;
        let signature = machine.memory_mut().load_bytes(signature_addr, SIGNATURE_LEN as u64)?;
        let message_hash = machine.memory_mut().load_bytes(message_hash_addr, MESSAGE_HASH_LEN as u64)?;

        let verified = Self::verify_signature(expected_pubkey_hash.as_ref(), signature.as_ref(), message_hash.as_ref());
        machine.add_cycles_no_checking(
            SECP256K1_VERIFY_BASE_CYCLES + transferred_byte_cycles(PUBKEY_HASH_LEN + SIGNATURE_LEN + MESSAGE_HASH_LEN),
        )?;
        machine.set_register(A0, M::REG::from_u8(if verified { SUCCESS } else { SECP256K1_VERIFY_FAILED }));

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::ScriptVersion;
    use ckb_vm::{registers::A7, CoreMachine, Memory, Register};
    use secp256k1::{ecdsa::RecoverableSignature, PublicKey, SecretKey};

    const PUBKEY_HASH_ADDR: u64 = 0x1000;
    const SIGNATURE_ADDR: u64 = 0x2000;
    const MESSAGE_HASH_ADDR: u64 = 0x3000;

    fn sample_signature_inputs() -> ([u8; 20], [u8; 65], [u8; 32]) {
        let secret_key = SecretKey::from_slice(&[0x11; 32]).expect("secret key");
        let message_hash = [0x22; 32];
        let message = Message::from_digest_slice(&message_hash).expect("message");
        let secp = Secp256k1::new();
        let signature = secp.sign_ecdsa_recoverable(&message, &secret_key);
        let (recovery_id, compact_sig) = signature.serialize_compact();

        let mut signature_bytes = [0u8; 65];
        signature_bytes[..64].copy_from_slice(&compact_sig);
        signature_bytes[64] = recovery_id.to_i32() as u8;

        let pubkey = PublicKey::from_secret_key(&secp, &secret_key).serialize();
        let pubkey_hash = blake3::hash(&pubkey);
        let mut pubkey_hash20 = [0u8; 20];
        pubkey_hash20.copy_from_slice(&pubkey_hash.as_bytes()[..20]);

        (pubkey_hash20, signature_bytes, message_hash)
    }

    fn sub_be(lhs: &[u8; 32], rhs: &[u8; 32]) -> [u8; 32] {
        let mut out = [0u8; 32];
        let mut borrow = 0i16;
        for i in (0..32).rev() {
            let diff = lhs[i] as i16 - rhs[i] as i16 - borrow;
            if diff < 0 {
                out[i] = (diff + 256) as u8;
                borrow = 1;
            } else {
                out[i] = diff as u8;
                borrow = 0;
            }
        }
        out
    }

    fn high_s_variant(signature: [u8; 65]) -> [u8; 65] {
        // secp256k1 curve order (big-endian)
        const CURVE_ORDER: [u8; 32] = [
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE, 0xBA, 0xAE, 0xDC, 0xE6,
            0xAF, 0x48, 0xA0, 0x3B, 0xBF, 0xD2, 0x5E, 0x8C, 0xD0, 0x36, 0x41, 0x41,
        ];

        let mut high_s = signature;
        let mut s = [0u8; 32];
        s.copy_from_slice(&signature[32..64]);
        let high_s_component = sub_be(&CURVE_ORDER, &s);
        high_s[32..64].copy_from_slice(&high_s_component);
        // Flip y-parity bit in recovery id to preserve recoverability.
        high_s[64] ^= 1;
        high_s
    }

    #[test]
    fn test_secp256k1_verify_accepts_valid_signature() {
        let (pubkey_hash, signature, message_hash) = sample_signature_inputs();
        let mut machine = ScriptVersion::V2.init_core_machine(200_000);
        machine.memory_mut().store_bytes(PUBKEY_HASH_ADDR, &pubkey_hash).unwrap();
        machine.memory_mut().store_bytes(SIGNATURE_ADDR, &signature).unwrap();
        machine.memory_mut().store_bytes(MESSAGE_HASH_ADDR, &message_hash).unwrap();
        machine.set_register(A0, PUBKEY_HASH_ADDR);
        machine.set_register(A1, SIGNATURE_ADDR);
        machine.set_register(A2, MESSAGE_HASH_ADDR);
        machine.set_register(A7, SECP256K1_VERIFY_SYSCALL_NUMBER);

        let mut syscall = Secp256k1Verify::new();
        let handled = syscall.ecall(&mut machine).expect("syscall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SUCCESS as u64);
        assert!(machine.cycles() >= SECP256K1_VERIFY_BASE_CYCLES);
    }

    #[test]
    fn test_secp256k1_verify_rejects_tampered_signature() {
        let (pubkey_hash, mut signature, message_hash) = sample_signature_inputs();
        signature[0] ^= 0xFF;

        let mut machine = ScriptVersion::V2.init_core_machine(200_000);
        machine.memory_mut().store_bytes(PUBKEY_HASH_ADDR, &pubkey_hash).unwrap();
        machine.memory_mut().store_bytes(SIGNATURE_ADDR, &signature).unwrap();
        machine.memory_mut().store_bytes(MESSAGE_HASH_ADDR, &message_hash).unwrap();
        machine.set_register(A0, PUBKEY_HASH_ADDR);
        machine.set_register(A1, SIGNATURE_ADDR);
        machine.set_register(A2, MESSAGE_HASH_ADDR);
        machine.set_register(A7, SECP256K1_VERIFY_SYSCALL_NUMBER);

        let mut syscall = Secp256k1Verify::new();
        let handled = syscall.ecall(&mut machine).expect("syscall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SECP256K1_VERIFY_FAILED as u64);
    }

    #[test]
    fn test_secp256k1_verify_rejects_non_canonical_recovery_id() {
        let (pubkey_hash, mut signature, message_hash) = sample_signature_inputs();
        signature[64] = signature[64].saturating_add(4);

        let mut machine = ScriptVersion::V2.init_core_machine(200_000);
        machine.memory_mut().store_bytes(PUBKEY_HASH_ADDR, &pubkey_hash).unwrap();
        machine.memory_mut().store_bytes(SIGNATURE_ADDR, &signature).unwrap();
        machine.memory_mut().store_bytes(MESSAGE_HASH_ADDR, &message_hash).unwrap();
        machine.set_register(A0, PUBKEY_HASH_ADDR);
        machine.set_register(A1, SIGNATURE_ADDR);
        machine.set_register(A2, MESSAGE_HASH_ADDR);
        machine.set_register(A7, SECP256K1_VERIFY_SYSCALL_NUMBER);

        let mut syscall = Secp256k1Verify::new();
        let handled = syscall.ecall(&mut machine).expect("syscall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SECP256K1_VERIFY_FAILED as u64);
    }

    #[test]
    fn test_secp256k1_verify_rejects_high_s_signature() {
        let (pubkey_hash, signature, message_hash) = sample_signature_inputs();
        let high_s_signature = high_s_variant(signature);

        // Ensure the transformed signature still recovers to the same pubkey hash.
        let recovery_id = secp256k1::ecdsa::RecoveryId::from_i32(high_s_signature[64] as i32).expect("recovery id");
        let recoverable_signature =
            RecoverableSignature::from_compact(&high_s_signature[..64], recovery_id).expect("recoverable signature");
        let message = Message::from_digest_slice(&message_hash).expect("message");
        let recovered_pubkey = Secp256k1::new().recover_ecdsa(&message, &recoverable_signature).expect("recovered pubkey");
        let recovered_hash = blake3::hash(&recovered_pubkey.serialize());
        assert_eq!(&recovered_hash.as_bytes()[..20], &pubkey_hash);

        let mut machine = ScriptVersion::V2.init_core_machine(200_000);
        machine.memory_mut().store_bytes(PUBKEY_HASH_ADDR, &pubkey_hash).unwrap();
        machine.memory_mut().store_bytes(SIGNATURE_ADDR, &high_s_signature).unwrap();
        machine.memory_mut().store_bytes(MESSAGE_HASH_ADDR, &message_hash).unwrap();
        machine.set_register(A0, PUBKEY_HASH_ADDR);
        machine.set_register(A1, SIGNATURE_ADDR);
        machine.set_register(A2, MESSAGE_HASH_ADDR);
        machine.set_register(A7, SECP256K1_VERIFY_SYSCALL_NUMBER);

        let mut syscall = Secp256k1Verify::new();
        let handled = syscall.ecall(&mut machine).expect("syscall should succeed");

        assert!(handled);
        assert_eq!(machine.registers()[A0].to_u64(), SECP256K1_VERIFY_FAILED as u64);
    }

    #[test]
    fn test_secp256k1_verify_ignores_other_syscalls() {
        let mut machine = ScriptVersion::V2.init_core_machine(200_000);
        machine.set_register(A7, 1);

        let mut syscall = Secp256k1Verify::new();
        let handled = syscall.ecall(&mut machine).expect("non-matching syscall should not error");

        assert!(!handled);
    }
}
