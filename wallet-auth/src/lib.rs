// SPDX-License-Identifier: MIT
// Copyright (C) 2026 Myelin developers

//! CKB-compatible wallet identities and challenge signatures.
//!
//! This crate deliberately implements only the cryptographic identity used by
//! the standard CKB `secp256k1_blake160_sighash_all` lock: compressed
//! secp256k1 public keys, CKB-personalized Blake2b hashing, the first 20 hash
//! bytes as lock args, and compact recoverable ECDSA signatures. It does not
//! claim that an arbitrary message signature is a CKB transaction witness.

use base64::{engine::general_purpose, Engine as _};
use bech32::{Bech32m, Hrp};
use blake2b_simd::Params;
use hmac::{Hmac, Mac};
use ring::signature::{RsaPublicKeyComponents, UnparsedPublicKey, ECDSA_P256_SHA256_ASN1, RSA_PKCS1_2048_8192_SHA256};
use secp256k1::{
    ecdsa::{RecoverableSignature, RecoveryId},
    Message, PublicKey, Scalar, Secp256k1, SecretKey,
};
use sha2::Sha512;
use zeroize::Zeroize;

/// CKB's hash personalization, used by `ckb_hash::new_blake2b`.
pub const CKB_HASH_PERSONALIZATION: &[u8; 16] = b"ckb-default-hash";
/// Stable wallet-login signature domain.
pub const LOGIN_DOMAIN: &[u8] = b"veloren:myelin:ckb-wallet-login";
/// Stable JoyID login challenge domain. JoyID signs the printable digest of
/// this domain-separated payload through WebAuthn.
pub const JOYID_LOGIN_DOMAIN: &[u8] = b"veloren:myelin:joyid-login";
/// Stable CKB-compatible PoA seal domain.
pub const POA_DOMAIN: &[u8] = b"myelin:ckb-proof-of-authority-seal";
/// First external address in CKB's registered BIP-44 coin namespace.
pub const CKB_FIRST_ACCOUNT_DERIVATION_PATH: &str = "m/44'/309'/0'/0/0";

const CKB_FIRST_ACCOUNT_DERIVATION: [u32; 5] = [44 | (1 << 31), 309 | (1 << 31), 1 << 31, 0, 0];

const CKB_WALLET_UUID_DOMAIN: &[u8] = b"veloren:myelin:ckb-wallet-uuid";
const JOYID_WALLET_UUID_DOMAIN: &[u8] = b"veloren:myelin:joyid-wallet-uuid";
const WALLET_ALIAS_PREFIX: &str = "w_";

const CKB_SIGHASH_TYPE_HASH: [u8; 32] = [
    0x9b, 0xd7, 0xe0, 0x6f, 0x3e, 0xcf, 0x4b, 0xe0, 0xf2, 0xfc, 0xd2, 0x18, 0x8b, 0x23, 0xf1, 0xb9, 0xfc, 0xc8, 0x8e, 0x5d, 0x4b,
    0x65, 0xa8, 0x63, 0x7b, 0x17, 0x72, 0x3b, 0xbd, 0xa3, 0xcc, 0xe8,
];
const JOYID_MAINNET_TYPE_HASH: [u8; 32] = [
    0xd0, 0x0c, 0x84, 0xf0, 0xec, 0x8f, 0xd4, 0x41, 0xc3, 0x8b, 0xc3, 0xf8, 0x7a, 0x37, 0x1f, 0x54, 0x71, 0x90, 0xf2, 0xfc, 0xff,
    0x88, 0xe6, 0x42, 0xbc, 0x5b, 0xf5, 0x4b, 0x9e, 0x31, 0x83, 0x23,
];
const JOYID_TESTNET_TYPE_HASH: [u8; 32] = [
    0xd2, 0x37, 0x61, 0xb3, 0x64, 0x21, 0x07, 0x35, 0xc1, 0x9c, 0x60, 0x56, 0x1d, 0x21, 0x3f, 0xb3, 0xbe, 0xae, 0x2f, 0xd6, 0x17,
    0x27, 0x43, 0x71, 0x9e, 0xff, 0x69, 0x20, 0xe0, 0x20, 0xba, 0xac,
];

/// A compressed secp256k1 public key, matching the input to CKB Blake160.
pub type CompressedPublicKey = [u8; 33];
/// The 20-byte args of the standard CKB secp256k1 lock.
pub type CkbLockArg = [u8; 20];
/// CKB-compatible compact recoverable ECDSA (`r || s || recovery_id`).
pub type RecoverableSignature65 = [u8; 65];

/// JoyID credential kind returned by the official browser SDK.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoyIdKeyType {
    MainKey,
    SubKey,
    MainSessionKey,
    SubSessionKey,
}

impl JoyIdKeyType {
    /// Parse the exact SDK wire name.
    pub fn parse(value: &str) -> Result<Self, WalletAuthError> {
        match value {
            "main_key" => Ok(Self::MainKey),
            "sub_key" => Ok(Self::SubKey),
            "main_session_key" => Ok(Self::MainSessionKey),
            "sub_session_key" => Ok(Self::SubSessionKey),
            _ => Err(WalletAuthError::InvalidJoyIdKeyType),
        }
    }

    /// Return the exact SDK wire name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MainKey => "main_key",
            Self::SubKey => "sub_key",
            Self::MainSessionKey => "main_session_key",
            Self::SubSessionKey => "sub_session_key",
        }
    }

    fn is_native(self) -> bool {
        matches!(self, Self::MainKey | Self::SubKey)
    }
}

/// Borrowed JoyID message-signature proof returned by the official SDK.
#[derive(Clone, Copy, Debug)]
pub struct JoyIdSignatureProof<'a> {
    pub signature: &'a str,
    pub message: &'a str,
    pub public_key: &'a str,
    pub key_type: JoyIdKeyType,
    /// WebAuthn COSE algorithm: `-7` (ES256) or `-257` (RS256).
    pub algorithm: i32,
}

/// Exact server challenge fields committed by a wallet login signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoginChallenge {
    /// Myelin deployment commitment advertised by the server.
    pub deployment_commitment: [u8; 32],
    /// Cryptographically random, connection-specific server nonce.
    pub nonce: [u8; 32],
    /// Server issuance time in Unix milliseconds.
    pub issued_at_ms: u64,
    /// Server expiry time in Unix milliseconds.
    pub expires_at_ms: u64,
    /// Exact network identifier (`ckb`, `ckb_testnet`, or a private chain id).
    pub network: u8,
}

/// Wallet-authentication failures.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum WalletAuthError {
    /// A BIP-39 recovery phrase or its checksum is invalid.
    #[error("invalid BIP-39 recovery phrase: {0}")]
    InvalidMnemonic(String),
    /// BIP-32 could not derive the standard CKB account key.
    #[error("invalid CKB BIP-32 derivation: {0}")]
    InvalidDerivation(String),
    /// Secret key bytes are not a valid secp256k1 scalar.
    #[error("invalid secp256k1 secret key: {0}")]
    InvalidSecretKey(String),
    /// Public key encoding is not a compressed secp256k1 key.
    #[error("invalid compressed secp256k1 public key: {0}")]
    InvalidPublicKey(String),
    /// Compact signature or recovery id is malformed.
    #[error("invalid recoverable secp256k1 signature: {0}")]
    InvalidSignature(String),
    /// Signature recovery produced a different CKB lock arg.
    #[error("signature does not recover the claimed CKB lock arg")]
    LockArgMismatch,
    /// A textual fixed-width value had the wrong encoding or length.
    #[error("invalid {field}: {reason}")]
    InvalidHex { field: &'static str, reason: String },
    /// A deterministic Veloren wallet alias was malformed or non-canonical.
    #[error("invalid wallet alias: {0}")]
    InvalidWalletAlias(String),
    /// A CKB address is malformed, belongs to another network, or does not use
    /// the expected standard/JoyID lock.
    #[error("invalid CKB address: {0}")]
    InvalidAddress(String),
    /// JoyID returned an unsupported credential kind.
    #[error("invalid JoyID key type")]
    InvalidJoyIdKeyType,
    /// JoyID proof encoding or WebAuthn client data is malformed.
    #[error("invalid JoyID proof: {0}")]
    InvalidJoyIdProof(String),
    /// The proof was validly encoded but did not authenticate the expected
    /// challenge, JoyID origin, or relying party.
    #[error("JoyID proof binding mismatch: {0}")]
    JoyIdBindingMismatch(&'static str),
    /// The JoyID P-256 or RSA signature did not verify.
    #[error("invalid JoyID signature")]
    InvalidJoyIdSignature,
}

/// Encode caller-supplied entropy as an English BIP-39 recovery phrase. The
/// wallet creation UI supplies 128 bits, producing the standard 12 words. The
/// phrase is not persisted by this crate.
pub fn ckb_mnemonic_from_entropy(entropy: &[u8]) -> Result<String, WalletAuthError> {
    bip39::Mnemonic::from_entropy_in(bip39::Language::English, entropy)
        .map(|mnemonic| mnemonic.to_string())
        .map_err(|error| WalletAuthError::InvalidMnemonic(error.to_string()))
}

/// Recover the first standard CKB account key from an English BIP-39 phrase.
///
/// This follows BIP-39 with an empty passphrase, then BIP-32/BIP-44 at
/// `m/44'/309'/0'/0/0`. Callers should persist only an appropriately protected
/// signer key and show the recovery phrase once for an offline backup.
pub fn ckb_secret_from_mnemonic(recovery_phrase: &str) -> Result<[u8; 32], WalletAuthError> {
    let mnemonic = bip39::Mnemonic::parse_in(bip39::Language::English, recovery_phrase)
        .map_err(|error| WalletAuthError::InvalidMnemonic(error.to_string()))?;
    let mut seed = mnemonic.to_seed("");
    let result = derive_bip32_secret(&seed, &CKB_FIRST_ACCOUNT_DERIVATION);
    seed.zeroize();
    result
}

fn derive_bip32_secret(seed: &[u8], path: &[u32]) -> Result<[u8; 32], WalletAuthError> {
    let mut digest = hmac_sha512(b"Bitcoin seed", seed)?;
    let mut secret = SecretKey::from_slice(&digest[..32])
        .map_err(|error| WalletAuthError::InvalidDerivation(format!("invalid master key: {error}")))?;
    let mut chain_code = [0u8; 32];
    chain_code.copy_from_slice(&digest[32..]);
    digest.zeroize();

    let secp = Secp256k1::new();
    for &child_number in path {
        let mut data = Vec::with_capacity(37);
        if child_number & (1 << 31) != 0 {
            data.push(0);
            data.extend_from_slice(&secret.secret_bytes());
        } else {
            data.extend_from_slice(&PublicKey::from_secret_key(&secp, &secret).serialize());
        }
        data.extend_from_slice(&child_number.to_be_bytes());

        digest = hmac_sha512(&chain_code, &data)?;
        data.zeroize();
        let mut left = [0u8; 32];
        left.copy_from_slice(&digest[..32]);
        if left == [0; 32] {
            left.zeroize();
            digest.zeroize();
            chain_code.zeroize();
            return Err(WalletAuthError::InvalidDerivation("BIP-32 produced a zero child tweak".to_owned()));
        }
        let tweak = Scalar::from_be_bytes(left)
            .map_err(|error| WalletAuthError::InvalidDerivation(format!("BIP-32 child tweak is out of range: {error}")))?;
        left.zeroize();
        secret = secret
            .add_tweak(&tweak)
            .map_err(|error| WalletAuthError::InvalidDerivation(format!("invalid BIP-32 child key: {error}")))?;
        chain_code.copy_from_slice(&digest[32..]);
        digest.zeroize();
    }
    chain_code.zeroize();
    Ok(secret.secret_bytes())
}

fn hmac_sha512(key: &[u8], data: &[u8]) -> Result<[u8; 64], WalletAuthError> {
    let mut mac = Hmac::<Sha512>::new_from_slice(key)
        .map_err(|error| WalletAuthError::InvalidDerivation(format!("invalid HMAC key: {error}")))?;
    mac.update(data);
    Ok(mac.finalize().into_bytes().into())
}

/// Compute CKB's personalized Blake2b-256 digest.
pub fn ckb_blake2b_256(data: &[u8]) -> [u8; 32] {
    let hash = Params::new().hash_length(32).personal(CKB_HASH_PERSONALIZATION).hash(data);
    let mut output = [0u8; 32];
    output.copy_from_slice(hash.as_bytes());
    output
}

/// Derive the exact UUID bytes historically used by Veloren for a standard
/// CKB wallet. This encoding is stable so removing the user-selected login
/// alias cannot move characters, bans, or administrator records to a new
/// account.
pub fn ckb_wallet_uuid_bytes(lock_arg: CkbLockArg) -> [u8; 16] {
    wallet_uuid_bytes(CKB_WALLET_UUID_DOMAIN, &lock_arg)
}

/// Derive the exact UUID bytes historically used by Veloren for a canonical
/// JoyID address.
pub fn joyid_wallet_uuid_bytes(address: &str) -> [u8; 16] {
    wallet_uuid_bytes(JOYID_WALLET_UUID_DOMAIN, address.as_bytes())
}

/// Encode a wallet UUID as a compact, reversible Veloren protocol alias.
/// This is an internal compatibility identifier, not a user-chosen login name.
pub fn wallet_alias(uuid_bytes: [u8; 16]) -> String {
    format!("{WALLET_ALIAS_PREFIX}{}", general_purpose::URL_SAFE_NO_PAD.encode(uuid_bytes))
}

/// Decode the canonical internal wallet alias back to its exact UUID bytes.
pub fn wallet_uuid_bytes_from_alias(alias: &str) -> Result<[u8; 16], WalletAuthError> {
    let encoded = alias
        .strip_prefix(WALLET_ALIAS_PREFIX)
        .ok_or_else(|| WalletAuthError::InvalidWalletAlias("missing wallet prefix".to_owned()))?;
    let decoded =
        general_purpose::URL_SAFE_NO_PAD.decode(encoded).map_err(|error| WalletAuthError::InvalidWalletAlias(error.to_string()))?;
    let uuid_bytes: [u8; 16] = decoded
        .try_into()
        .map_err(|bytes: Vec<u8>| WalletAuthError::InvalidWalletAlias(format!("expected 16 UUID bytes, got {}", bytes.len())))?;
    if wallet_alias(uuid_bytes) != alias {
        return Err(WalletAuthError::InvalidWalletAlias("alias is not canonically encoded".to_owned()));
    }
    Ok(uuid_bytes)
}

fn wallet_uuid_bytes(domain: &[u8], identity: &[u8]) -> [u8; 16] {
    let mut input = Vec::with_capacity(domain.len() + identity.len());
    input.extend_from_slice(domain);
    input.extend_from_slice(identity);
    let hash = ckb_blake2b_256(&input);
    let mut historical_little_endian = [0u8; 16];
    historical_little_endian.copy_from_slice(&hash[..16]);
    u128::from_le_bytes(historical_little_endian).to_be_bytes()
}

/// Derive a compressed public key from secret bytes.
pub fn compressed_public_key(secret: [u8; 32]) -> Result<CompressedPublicKey, WalletAuthError> {
    let secret = SecretKey::from_slice(&secret).map_err(|error| WalletAuthError::InvalidSecretKey(error.to_string()))?;
    Ok(PublicKey::from_secret_key(&Secp256k1::new(), &secret).serialize())
}

/// Derive standard CKB secp256k1 lock args from a compressed public key.
pub fn lock_arg_from_public_key(public_key: CompressedPublicKey) -> Result<CkbLockArg, WalletAuthError> {
    let public_key = PublicKey::from_slice(&public_key).map_err(|error| WalletAuthError::InvalidPublicKey(error.to_string()))?;
    let hash = ckb_blake2b_256(&public_key.serialize());
    let mut lock_arg = [0u8; 20];
    lock_arg.copy_from_slice(&hash[..20]);
    Ok(lock_arg)
}

/// Derive standard CKB secp256k1 lock args directly from secret bytes.
pub fn lock_arg_from_secret(secret: [u8; 32]) -> Result<CkbLockArg, WalletAuthError> {
    lock_arg_from_public_key(compressed_public_key(secret)?)
}

/// Canonical wallet-login digest signed as a prehashed CKB message.
pub fn login_digest(challenge: LoginChallenge, alias: &str, lock_arg: CkbLockArg) -> [u8; 32] {
    let mut bytes = Vec::with_capacity(LOGIN_DOMAIN.len() + 32 + 32 + 8 + 8 + 1 + 4 + alias.len() + 20);
    put_bytes(&mut bytes, LOGIN_DOMAIN);
    bytes.extend_from_slice(&challenge.deployment_commitment);
    bytes.extend_from_slice(&challenge.nonce);
    bytes.extend_from_slice(&challenge.issued_at_ms.to_le_bytes());
    bytes.extend_from_slice(&challenge.expires_at_ms.to_le_bytes());
    bytes.push(challenge.network);
    put_bytes(&mut bytes, alias.as_bytes());
    bytes.extend_from_slice(&lock_arg);
    ckb_blake2b_256(&bytes)
}

/// Canonical printable challenge for JoyID message signing. The signed string
/// is intentionally short, while its CKB hash commits to every login field,
/// alias, and exact normalized JoyID address.
pub fn joyid_login_challenge(challenge: LoginChallenge, alias: &str, address: &str) -> String {
    let mut bytes = Vec::with_capacity(JOYID_LOGIN_DOMAIN.len() + 32 + 32 + 8 + 8 + 1 + 8 + alias.len() + address.len());
    put_bytes(&mut bytes, JOYID_LOGIN_DOMAIN);
    bytes.extend_from_slice(&challenge.deployment_commitment);
    bytes.extend_from_slice(&challenge.nonce);
    bytes.extend_from_slice(&challenge.issued_at_ms.to_le_bytes());
    bytes.extend_from_slice(&challenge.expires_at_ms.to_le_bytes());
    bytes.push(challenge.network);
    put_bytes(&mut bytes, alias.as_bytes());
    put_bytes(&mut bytes, address.as_bytes());
    format!("Veloren Myelin JoyID login v1: {}", hex::encode(ckb_blake2b_256(&bytes)))
}

/// Encode a standard CKB secp256k1-blake160 address using the current CKB2021
/// full-address Bech32m format. Network `0` is mainnet; `1` and `2` use the
/// test/private `ckt` prefix.
pub fn ckb_sighash_address(lock_arg: CkbLockArg, network: u8) -> Result<String, WalletAuthError> {
    let hrp = ckb_hrp(network)?;
    let mut payload = Vec::with_capacity(54);
    payload.push(0); // CKB2021 full payload.
    payload.extend_from_slice(&CKB_SIGHASH_TYPE_HASH);
    payload.push(1); // ScriptHashType::Type.
    payload.extend_from_slice(&lock_arg);
    bech32::encode::<Bech32m>(hrp, &payload).map_err(|error| WalletAuthError::InvalidAddress(error.to_string()))
}

/// Validate a standard CKB secp256k1-blake160 address and return its canonical
/// CKB2021 spelling plus exact 20-byte lock argument. Legacy short/full-type
/// spellings are accepted but normalized before identity binding.
pub fn normalize_ckb_sighash_address(address: &str, network: u8) -> Result<(String, CkbLockArg), WalletAuthError> {
    if address.len() > 160 {
        return Err(WalletAuthError::InvalidAddress("address is too long".to_owned()));
    }
    let (hrp, payload) = bech32::decode(address).map_err(|error| WalletAuthError::InvalidAddress(error.to_string()))?;
    if hrp != ckb_hrp(network)? {
        return Err(WalletAuthError::InvalidAddress("network prefix mismatch".to_owned()));
    }
    let args = match payload.first().copied() {
        // Deprecated short address: 0x01 | sighash code-hash index 0x00 | args.
        Some(1) if payload.len() == 22 && payload[1] == 0 => &payload[2..],
        // CKB2021: 0x00 | code_hash | hash_type | args.
        Some(0) if payload.len() == 54 => {
            if payload[1..33] != CKB_SIGHASH_TYPE_HASH || payload[33] != 1 {
                return Err(WalletAuthError::InvalidAddress("address does not use the standard CKB sighash lock".to_owned()));
            }
            &payload[34..]
        }
        // Deprecated full type: 0x04 | code_hash | args.
        Some(4) if payload.len() == 53 => {
            if payload[1..33] != CKB_SIGHASH_TYPE_HASH {
                return Err(WalletAuthError::InvalidAddress("address does not use the standard CKB sighash lock".to_owned()));
            }
            &payload[33..]
        }
        _ => {
            return Err(WalletAuthError::InvalidAddress("unsupported standard CKB address payload".to_owned()));
        }
    };
    let lock_arg: CkbLockArg =
        args.try_into().map_err(|_| WalletAuthError::InvalidAddress("sighash lock args must be 20 bytes".to_owned()))?;
    Ok((ckb_sighash_address(lock_arg, network)?, lock_arg))
}

/// Validate and normalize a current JoyID CKB address. This verifies its
/// network, type hash, type hash mode, and 22-byte JoyID lock arguments; it
/// does not by itself prove that a supplied credential belongs to the address.
pub fn normalize_joyid_address(address: &str, network: u8) -> Result<String, WalletAuthError> {
    if address.len() > 160 {
        return Err(WalletAuthError::InvalidAddress("address is too long".to_owned()));
    }
    let (hrp, payload) = bech32::decode(address).map_err(|error| WalletAuthError::InvalidAddress(error.to_string()))?;
    if hrp != ckb_hrp(network)? {
        return Err(WalletAuthError::InvalidAddress("network prefix mismatch".to_owned()));
    }
    let expected_hash = match network {
        0 => JOYID_MAINNET_TYPE_HASH,
        1 => JOYID_TESTNET_TYPE_HASH,
        _ => {
            return Err(WalletAuthError::InvalidAddress("JoyID is unavailable on private CKB networks".to_owned()));
        }
    };
    let args = match payload.first().copied() {
        // CKB2021: 0x00 | code_hash | hash_type | args.
        Some(0) if payload.len() == 56 => {
            if payload[1..33] != expected_hash || payload[33] != 1 {
                return Err(WalletAuthError::InvalidAddress("address does not use the configured JoyID type lock".to_owned()));
            }
            &payload[34..]
        }
        // Legacy full type: 0x04 | code_hash | args.
        Some(4) if payload.len() == 55 => {
            if payload[1..33] != expected_hash {
                return Err(WalletAuthError::InvalidAddress("address does not use the configured JoyID type lock".to_owned()));
            }
            &payload[33..]
        }
        _ => {
            return Err(WalletAuthError::InvalidAddress("unsupported JoyID address payload".to_owned()));
        }
    };
    if args.len() != 22 || args[0] != 0 || !matches!(args[1], 1 | 2) {
        return Err(WalletAuthError::InvalidAddress("invalid JoyID lock arguments".to_owned()));
    }
    let mut canonical = Vec::with_capacity(56);
    canonical.push(0);
    canonical.extend_from_slice(&expected_hash);
    canonical.push(1);
    canonical.extend_from_slice(args);
    bech32::encode::<Bech32m>(hrp, &canonical).map_err(|error| WalletAuthError::InvalidAddress(error.to_string()))
}

/// Verify the cryptographic half of a JoyID login proof. Callers must also
/// verify that the credential is registered to the claimed JoyID CKB address
/// (locally from CKB/COTA evidence or through the official credential API).
pub fn verify_joyid_signature(
    proof: JoyIdSignatureProof<'_>,
    expected_challenge: &str,
    expected_origin: &str,
    expected_rp_id: &str,
) -> Result<(), WalletAuthError> {
    let public_key = parse_unprefixed_hex(proof.public_key, "JoyID public key")?;
    let signature = decode_base64url(proof.signature, "JoyID signature")?;
    let message = decode_base64url(proof.message, "JoyID message")?;

    if proof.key_type.is_native() {
        verify_joyid_native(proof.algorithm, &public_key, &signature, &message, expected_challenge, expected_origin, expected_rp_id)
    } else {
        if message != expected_challenge.as_bytes() {
            return Err(WalletAuthError::JoyIdBindingMismatch("session-key challenge"));
        }
        verify_joyid_rsa(&public_key, &signature, &message)
    }
}

fn verify_joyid_native(
    algorithm: i32,
    public_key: &[u8],
    signature: &[u8],
    message: &[u8],
    expected_challenge: &str,
    expected_origin: &str,
    expected_rp_id: &str,
) -> Result<(), WalletAuthError> {
    if message.len() < 38 {
        return Err(WalletAuthError::InvalidJoyIdProof("WebAuthn message is shorter than authenticator data".to_owned()));
    }
    let (authenticator_data, client_data) = message.split_at(37);
    let rp_hash = ring::digest::digest(&ring::digest::SHA256, expected_rp_id.as_bytes());
    if authenticator_data[..32] != *rp_hash.as_ref() {
        return Err(WalletAuthError::JoyIdBindingMismatch("relying-party id"));
    }
    let flags = authenticator_data[32];
    if flags & 0x01 == 0 || flags & 0x04 == 0 {
        return Err(WalletAuthError::JoyIdBindingMismatch("WebAuthn user-presence/user-verification flags"));
    }
    let client_data: serde_json::Value =
        serde_json::from_slice(client_data).map_err(|error| WalletAuthError::InvalidJoyIdProof(error.to_string()))?;
    if client_data.get("type").and_then(serde_json::Value::as_str) != Some("webauthn.get") {
        return Err(WalletAuthError::JoyIdBindingMismatch("WebAuthn ceremony type"));
    }
    if client_data.get("origin").and_then(serde_json::Value::as_str) != Some(expected_origin) {
        return Err(WalletAuthError::JoyIdBindingMismatch("JoyID origin"));
    }
    let expected = general_purpose::URL_SAFE_NO_PAD.encode(expected_challenge.as_bytes());
    if client_data.get("challenge").and_then(serde_json::Value::as_str) != Some(expected.as_str()) {
        return Err(WalletAuthError::JoyIdBindingMismatch("login challenge"));
    }
    // The signature covers the exact received clientDataJSON bytes, not its
    // parsed/serialized representation.
    let client_data_bytes = &message[37..];
    let client_hash = ring::digest::digest(&ring::digest::SHA256, client_data_bytes);
    let mut signed = Vec::with_capacity(69);
    signed.extend_from_slice(authenticator_data);
    signed.extend_from_slice(client_hash.as_ref());
    match algorithm {
        -7 => {
            if public_key.len() != 64 {
                return Err(WalletAuthError::InvalidJoyIdProof("ES256 public key must contain 64 bytes".to_owned()));
            }
            let mut sec1 = Vec::with_capacity(65);
            sec1.push(4);
            sec1.extend_from_slice(public_key);
            UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, sec1)
                .verify(&signed, signature)
                .map_err(|_| WalletAuthError::InvalidJoyIdSignature)
        }
        -257 => verify_joyid_rsa(public_key, signature, &signed),
        _ => Err(WalletAuthError::InvalidJoyIdProof("unsupported WebAuthn algorithm".to_owned())),
    }
}

fn verify_joyid_rsa(public_key: &[u8], signature: &[u8], message: &[u8]) -> Result<(), WalletAuthError> {
    if public_key.len() < 260 || public_key[3] != 0 {
        return Err(WalletAuthError::InvalidJoyIdProof("invalid JoyID RSA public key".to_owned()));
    }
    let mut exponent = public_key[..3].to_vec();
    exponent.reverse();
    let mut modulus = public_key[4..].to_vec();
    modulus.reverse();
    RsaPublicKeyComponents { n: &modulus, e: &exponent }
        .verify(&RSA_PKCS1_2048_8192_SHA256, message, signature)
        .map_err(|_| WalletAuthError::InvalidJoyIdSignature)
}

fn decode_base64url(value: &str, field: &'static str) -> Result<Vec<u8>, WalletAuthError> {
    general_purpose::URL_SAFE_NO_PAD
        .decode(value)
        .or_else(|_| general_purpose::URL_SAFE.decode(value))
        .map_err(|error| WalletAuthError::InvalidJoyIdProof(format!("invalid {field}: {error}")))
}

fn parse_unprefixed_hex(value: &str, field: &'static str) -> Result<Vec<u8>, WalletAuthError> {
    let value = value.strip_prefix("0x").unwrap_or(value);
    hex::decode(value).map_err(|error| WalletAuthError::InvalidJoyIdProof(format!("invalid {field}: {error}")))
}

fn ckb_hrp(network: u8) -> Result<Hrp, WalletAuthError> {
    let value = match network {
        0 => "ckb",
        1 | 2 => "ckt",
        _ => return Err(WalletAuthError::InvalidAddress("unknown CKB network".to_owned())),
    };
    Hrp::parse(value).map_err(|error| WalletAuthError::InvalidAddress(error.to_string()))
}

/// Canonical PoA seal digest using CKB's hashing function.
pub fn poa_seal_digest(authority_id: &str, block_hash: [u8; 32], height: u64) -> [u8; 32] {
    let mut bytes = Vec::with_capacity(POA_DOMAIN.len() + authority_id.len() + 32 + 12);
    put_bytes(&mut bytes, POA_DOMAIN);
    bytes.extend_from_slice(&height.to_le_bytes());
    put_bytes(&mut bytes, authority_id.as_bytes());
    bytes.extend_from_slice(&block_hash);
    ckb_blake2b_256(&bytes)
}

/// Sign an already-hashed 32-byte CKB message.
pub fn sign_recoverable(secret: [u8; 32], digest: [u8; 32]) -> Result<RecoverableSignature65, WalletAuthError> {
    let secret = SecretKey::from_slice(&secret).map_err(|error| WalletAuthError::InvalidSecretKey(error.to_string()))?;
    let signature = Secp256k1::new().sign_ecdsa_recoverable(&Message::from_digest(digest), &secret);
    let (recovery_id, compact) = signature.serialize_compact();
    let mut output = [0u8; 65];
    output[..64].copy_from_slice(&compact);
    output[64] = recovery_id.to_i32() as u8;
    Ok(output)
}

/// Recover the compressed public key from a CKB-compatible signature.
pub fn recover_public_key(digest: [u8; 32], signature: RecoverableSignature65) -> Result<CompressedPublicKey, WalletAuthError> {
    let recovery_id =
        RecoveryId::from_i32(i32::from(signature[64])).map_err(|error| WalletAuthError::InvalidSignature(error.to_string()))?;
    let signature = RecoverableSignature::from_compact(&signature[..64], recovery_id)
        .map_err(|error| WalletAuthError::InvalidSignature(error.to_string()))?;
    Secp256k1::new()
        .recover_ecdsa(&Message::from_digest(digest), &signature)
        .map(|public_key| public_key.serialize())
        .map_err(|error| WalletAuthError::InvalidSignature(error.to_string()))
}

/// Verify by public-key recovery and exact CKB Blake160 lock-arg comparison.
pub fn verify_lock_arg(
    digest: [u8; 32],
    signature: RecoverableSignature65,
    expected_lock_arg: CkbLockArg,
) -> Result<CompressedPublicKey, WalletAuthError> {
    let public_key = recover_public_key(digest, signature)?;
    if lock_arg_from_public_key(public_key)? != expected_lock_arg {
        return Err(WalletAuthError::LockArgMismatch);
    }
    Ok(public_key)
}

/// Parse exactly 32 bytes of optional-`0x` hex.
pub fn parse_secret_hex(value: &str) -> Result<[u8; 32], WalletAuthError> {
    parse_fixed_hex(value, "secret key")
}

/// Parse exactly 32 bytes of optional-`0x` hex as a generic digest.
pub fn parse_hash_hex(value: &str) -> Result<[u8; 32], WalletAuthError> {
    parse_fixed_hex(value, "hash")
}

/// Parse exactly 20 bytes of optional-`0x` hex.
pub fn parse_lock_arg_hex(value: &str) -> Result<CkbLockArg, WalletAuthError> {
    parse_fixed_hex(value, "CKB lock arg")
}

/// Parse exactly 65 bytes of optional-`0x` hex.
pub fn parse_signature_hex(value: &str) -> Result<RecoverableSignature65, WalletAuthError> {
    parse_fixed_hex(value, "recoverable signature")
}

fn parse_fixed_hex<const N: usize>(value: &str, field: &'static str) -> Result<[u8; N], WalletAuthError> {
    let value = value.strip_prefix("0x").unwrap_or(value);
    let decoded = hex::decode(value).map_err(|error| WalletAuthError::InvalidHex { field, reason: error.to_string() })?;
    decoded
        .try_into()
        .map_err(|bytes: Vec<u8>| WalletAuthError::InvalidHex { field, reason: format!("expected {N} bytes, got {}", bytes.len()) })
}

fn put_bytes(output: &mut Vec<u8>, value: &[u8]) {
    output.extend_from_slice(&(value.len() as u32).to_le_bytes());
    output.extend_from_slice(value);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn challenge() -> LoginChallenge {
        LoginChallenge { deployment_commitment: [1; 32], nonce: [2; 32], issued_at_ms: 1_000, expires_at_ms: 61_000, network: 1 }
    }

    #[test]
    fn ckb_lock_arg_and_recoverable_signature_round_trip() {
        let secret = [7; 32];
        let lock_arg = lock_arg_from_secret(secret).unwrap();
        let digest = login_digest(challenge(), "alice", lock_arg);
        let signature = sign_recoverable(secret, digest).unwrap();
        let recovered = verify_lock_arg(digest, signature, lock_arg).unwrap();
        assert_eq!(recovered, compressed_public_key(secret).unwrap());
    }

    #[test]
    fn every_login_binding_rejects_replay_or_substitution() {
        let secret = [8; 32];
        let lock_arg = lock_arg_from_secret(secret).unwrap();
        let original = login_digest(challenge(), "alice", lock_arg);
        let signature = sign_recoverable(secret, original).unwrap();
        let mut mutations = Vec::new();
        let mut changed = challenge();
        changed.deployment_commitment[0] ^= 1;
        mutations.push(login_digest(changed, "alice", lock_arg));
        changed = challenge();
        changed.nonce[0] ^= 1;
        mutations.push(login_digest(changed, "alice", lock_arg));
        changed = challenge();
        changed.issued_at_ms += 1;
        mutations.push(login_digest(changed, "alice", lock_arg));
        changed = challenge();
        changed.expires_at_ms += 1;
        mutations.push(login_digest(changed, "alice", lock_arg));
        changed = challenge();
        changed.network ^= 1;
        mutations.push(login_digest(changed, "alice", lock_arg));
        mutations.push(login_digest(challenge(), "bob", lock_arg));
        let mut other_lock = lock_arg;
        other_lock[0] ^= 1;
        mutations.push(login_digest(challenge(), "alice", other_lock));
        for digest in mutations {
            assert!(verify_lock_arg(digest, signature, lock_arg).is_err());
        }
    }

    #[test]
    fn deterministic_wallet_alias_is_reversible_and_identity_scoped() {
        let lock_arg = lock_arg_from_secret([9; 32]).unwrap();
        let ckb_uuid = ckb_wallet_uuid_bytes(lock_arg);
        let ckb_alias = wallet_alias(ckb_uuid);
        assert!(ckb_alias.starts_with("w_"));
        assert!(ckb_alias.len() <= 32);
        assert_eq!(wallet_uuid_bytes_from_alias(&ckb_alias).unwrap(), ckb_uuid);
        assert!(wallet_uuid_bytes_from_alias(&ckb_alias.replacen("w_", "W_", 1)).is_err());

        let joyid_uuid = joyid_wallet_uuid_bytes("ckt1-canonical-joyid-fixture");
        assert_ne!(joyid_uuid, ckb_uuid);
        assert_ne!(wallet_alias(joyid_uuid), ckb_alias);
    }

    #[test]
    fn matches_known_ckb_blake160_vector() {
        // Generated independently with ckb-cli/secp256k1 for secret 0x01..01.
        let secret = [1; 32];
        assert_eq!(
            hex::encode(compressed_public_key(secret).unwrap()),
            "031b84c5567b126440995d3ed5aaba0565d71e1834604819ff9c17f5e9d5dd078f"
        );
        assert_eq!(hex::encode(lock_arg_from_secret(secret).unwrap()), "b6ac779881b4fe05a167e413ff534469b6b5f6c0");
    }

    #[test]
    fn derives_the_first_standard_ckb_account_from_bip39() {
        // 128-bit all-zero entropy from the official BIP-39 vectors. The BIP-32
        // result was independently checked at m/44'/309'/0'/0/0.
        let phrase = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        assert_eq!(ckb_mnemonic_from_entropy(&[0; 16]).unwrap(), phrase);
        let secret = ckb_secret_from_mnemonic(phrase).unwrap();
        assert_eq!(hex::encode(secret), "b217d9a18ff657c99872cc11a2fa2aa3e970cef8c6faa7d6e424bf057cb3707b");
        assert_eq!(hex::encode(lock_arg_from_secret(secret).unwrap()), "196f6c1f21f7dbf0df814539b840059facbafc24");
        assert!(ckb_secret_from_mnemonic("abandon abandon").is_err());
    }

    #[test]
    fn accepts_ckb_cli_recoverable_signature_vector() {
        // Produced by ckb-cli 2.0.0 `util sign-message --recoverable` for
        // digest 0x42..42. This guards the external-wallet wire format.
        let digest = [0x42; 32];
        let signature = parse_signature_hex(
            "9ce2d5bc07a3552b468dc60953a1f3a7d2cabc9e28def9bbc514f212d11b4154\
             0da4aec4bf2f483620c1ac990486b87e48579c779d37778c0eac7b7851c71e2201",
        )
        .unwrap();
        let expected_lock_arg = parse_lock_arg_hex("696bc2fe08aaee2ebb0d230e12ce593d5e4c61b3").unwrap();
        let recovered = verify_lock_arg(digest, signature, expected_lock_arg).unwrap();
        assert_eq!(hex::encode(recovered), "02a5a11946b0fa6c47a5b1560a081dddb2a9befe55ad79809e3e7f3dac91bcd979");
    }

    #[test]
    fn verifies_official_joyid_es256_fixture_and_all_bindings() {
        // Fixture published by @joyid/ckb 1.1.2. The SDK fixture stores the
        // WebAuthn values as hex; the live popup returns base64url.
        let signature = general_purpose::URL_SAFE_NO_PAD.encode(
            hex::decode(
                "30450220132cdbb56e034dfae1659dc2c27269d0e8fbde5ac3aaa1cfeab1ada7ca55629b02210099c3a1300ef6216ec9ee7f6d81585940983a44d13f0c5b113f204fe7ae4545b2",
            )
            .unwrap(),
        );
        let message = general_purpose::URL_SAFE_NO_PAD.encode(
            hex::decode(
                "b6c062a17d8a430d9413ffc10a1e1d3389943ceadd8a5c5fed23804ebf1308ca1d000000007b2274797065223a22776562617574686e2e676574222c226368616c6c656e6765223a2255326c6e6269423061476c7a49475a76636942745a51222c226f726967696e223a2268747470733a2f2f6a6f7969642d6170702d6769742d666561742d72656d6f76652d6e616d652d6e657276696e612e76657263656c2e617070227d",
            )
            .unwrap(),
        );
        let proof = JoyIdSignatureProof {
            signature: &signature,
            message: &message,
            public_key: "225644b369b4814011963d6f60624099eca92c17a0d48599e6c60d32caf178e002068e7e033cd203f1e31372a9f1fe0fcd416de1624778cccaf6a8de92478327",
            key_type: JoyIdKeyType::MainKey,
            algorithm: -7,
        };
        let origin = "https://joyid-app-git-feat-remove-name-nervina.vercel.app";
        let rp_id = "joyid-app-git-feat-remove-name-nervina.vercel.app";
        verify_joyid_signature(proof, "Sign this for me", origin, rp_id).unwrap();
        assert!(verify_joyid_signature(proof, "Sign this for you", origin, rp_id).is_err());
        assert!(verify_joyid_signature(proof, "Sign this for me", "https://evil.example", rp_id).is_err());
        assert!(verify_joyid_signature(proof, "Sign this for me", origin, "evil.example").is_err());
    }

    #[test]
    fn ckb2021_addresses_are_network_and_script_bound() {
        let lock_arg = lock_arg_from_secret([1; 32]).unwrap();
        let address = ckb_sighash_address(lock_arg, 1).unwrap();
        assert_eq!(normalize_ckb_sighash_address(&address, 1).unwrap(), (address.clone(), lock_arg));
        let (hrp, payload) = bech32::decode(&address).unwrap();
        assert_eq!(hrp, Hrp::parse("ckt").unwrap());
        assert_eq!(payload[0], 0);
        assert_eq!(&payload[1..33], &CKB_SIGHASH_TYPE_HASH);
        assert_eq!(payload[33], 1);
        assert_eq!(&payload[34..], &lock_arg);
        assert!(normalize_ckb_sighash_address(&address, 0).is_err());

        let joyid = "ckt1qrfrwcdnvssswdwpn3s9v8fp87emat306ctjwsm3nmlkjg8qyza2cqgqq9mxjf0qnyfusww65kapv2rc0qdm6sjpvvadd4hp";
        assert_eq!(normalize_joyid_address(joyid, 1).unwrap(), joyid);
        assert!(normalize_joyid_address(joyid, 0).is_err());
        assert!(normalize_joyid_address(joyid, 2).is_err());
    }
}
