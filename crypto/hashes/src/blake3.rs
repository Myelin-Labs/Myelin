use blake3::Hasher;

use crate::HASH_SIZE;

pub type Hash256 = [u8; HASH_SIZE];

/// Computes a one-shot BLAKE3-256 hash of the input.
/// Returns a 32-byte array.
///
/// 📌 Used for:
/// - Transaction ID (TXID) generation
/// - Block header hashing
/// - Public key hashing (e.g., address encoding)
/// - Schnorr signing challenge hash (H(R‖m))
pub fn blake3_256(data: &[u8]) -> Hash256 {
    blake3::hash(data).into()
}

/// Computes a double BLAKE3 hash (i.e., BLAKE3d),
/// Returns a 32-byte array.
///
/// 📌 Used for:
/// - Commitment ID hashing
/// - Cell ID or contract state anchors
#[allow(dead_code)]
pub fn blake3d(data: &[u8]) -> Hash256 {
    blake3_256(&blake3_256(data))
}

/// Computes a streaming BLAKE3 hash from multiple input slices,
/// useful when serializing structured data piecewise.
/// Returns a 32-byte array.
///
/// 📌 Used for:
/// - Merkle tree hash construction
/// - Incremental digest (e.g., block serialization)
/// - Multi-part message signing
#[allow(dead_code)]
pub fn blake3_stream(data: &[&[u8]]) -> Hash256 {
    let mut hasher = Hasher::new();
    for chunk in data {
        hasher.update(chunk);
    }
    hasher.finalize().into()
}

#[cfg(test)]
mod test {
    use super::*;

    /// Unit test for `blake3_256()`. Verifies hash length is 32 bytes.
    /// Replace "myelin" with a test vector if needed.
    #[test]
    fn test_blake3_256() {
        let hash = blake3_256(b"myelin");
        assert_eq!(hash.len(), 32);
    }

    #[test]
    fn test_empty_blake3() {
        let hash = blake3_256(b"");
        insta::assert_snapshot!(format!("{hash:?}"), @"[175, 19, 73, 185, 245, 249, 161, 166, 160, 64, 77, 234, 54, 220, 201, 73, 155, 203, 37, 201, 173, 193, 18, 183, 204, 154, 147, 202, 228, 31, 50, 98]");
    }
}
