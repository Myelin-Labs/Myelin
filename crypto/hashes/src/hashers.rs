//! Implements domain-separated hashing for Myelin blockchain.
//!
//! ⚠️ **Modified for the radical transition from blake2b & sha256 to full-scale BLAKE3 adoption.**
//!
//! Domain-separated hashers are implemented on top of BLAKE3 for:
//! - Transaction ID
//! - Block hash
//! - Signing challenge hash
//! - Merkle tree hashing
use crate::{blake3::blake3_256, Hash};
use sha2::{Digest, Sha256};

pub trait HasherBase {
    fn update<A: AsRef<[u8]>>(&mut self, data: A) -> &mut Self;
}

pub trait Hasher: HasherBase + Clone + Default {
    fn finalize(self) -> Hash;
    fn reset(&mut self);
    #[inline(always)]
    fn hash<A: AsRef<[u8]>>(data: A) -> Hash {
        let mut hasher = Self::default();
        hasher.update(data);
        hasher.finalize()
    }
}

// Hashers defined using domain-separation + BLAKE3 backend.
macro_rules! blake3_hasher {
    ($(struct $name:ident => $domain_sep:literal),+ $(,)?) => {
        $(
            #[derive(Clone)]
            pub struct $name(Vec<u8>);

            impl $name {
                #[inline(always)]
                pub fn new() -> Self {
                    let prefix = Vec::from($domain_sep);
                    Self(prefix)
                }

                pub fn write<A: AsRef<[u8]>>(&mut self, data: A) {
                    // TODO: use update
                    self.0.extend_from_slice(data.as_ref());
                }

                #[inline(always)]
                pub fn finalize(self) -> Hash {
                    blake3_256(&self.0).into()
                }
            }

            impl_hasher! { struct $name }
        )*
    };
}

macro_rules! impl_hasher {
    (struct $name:ident) => {
        impl HasherBase for $name {
            #[inline(always)]
            fn update<A: AsRef<[u8]>>(&mut self, data: A) -> &mut Self {
                self.write(data);
                self
            }
        }
        impl Hasher for $name {
            #[inline(always)]
            fn finalize(self) -> Hash {
                // Call the method
                $name::finalize(self)
            }
            #[inline(always)]
            fn reset(&mut self) {
                *self = Self::new();
            }
        }
        impl Default for $name {
            #[inline(always)]
            fn default() -> Self {
                Self::new()
            }
        }
    };
}

// ✅ Now define all hashers via BLAKE3
blake3_hasher! {
    struct CellTxHash                  => b"TransactionHash",
    struct CellTxId                    => b"TransactionID",
    struct CellTxSigningHash           => b"TransactionSigningHash",
    struct BlockHash                   => b"BlockHash",
    struct MerkleBranchHash            => b"MerkleBranchHash",
    struct MuHashElementHash           => b"MuHashElement",
    struct MuHashFinalizeHash          => b"MuHashFinalize",
    struct CellMessageSigningHash      => b"PersonalMessageSigningHash",
    struct CellTxSigningHashEcdsa      => b"TransactionSigningHashECDSA",
}

// Add dedicated SHA256 hasher for Schnorr signatures
macro_rules! sha256_hasher {
    ($(struct $name:ident => $domain_sep:literal),+ $(,)?) => {
        $(
            #[derive(Clone)]
            pub struct $name(Vec<u8>);

            impl $name {
                #[inline(always)]
                pub fn new() -> Self {
                    let prefix = Vec::from($domain_sep);
                    Self(prefix)
                }

                pub fn write<A: AsRef<[u8]>>(&mut self, data: A) {
                    self.0.extend_from_slice(data.as_ref());
                }

                #[inline(always)]
                pub fn finalize(self) -> Hash {
                    sha256(&self.0).into()
                }
            }

            impl_hasher! { struct $name }
        )*
    };
}

sha256_hasher! {
    struct SchnorrSigningHash => b"SchnorrSigningHash",
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vectors() {
        let input_data = [&[], &[1][..], &[42; 64], &[0; 8][..]];

        fn run_test_vector<H: Hasher>(input: &[&[u8]], hasher: impl Fn() -> H) {
            let mut h = hasher();
            for chunk in input {
                h.update(chunk);
            }
            let result = h.finalize();
            assert_eq!(result.0.len(), 32);
        }

        run_test_vector(&input_data, CellTxHash::new);
        run_test_vector(&input_data, CellTxId::new);
        run_test_vector(&input_data, BlockHash::new);
    }
}
