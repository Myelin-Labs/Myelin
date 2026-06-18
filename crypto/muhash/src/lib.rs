// Make u3072 public if we're fuzzing
#[cfg(fuzzing)]
pub mod u3072;
#[cfg(not(fuzzing))]
mod u3072;

use crate::u3072::U3072;
use rand_chacha::rand_core::{RngCore, SeedableRng};
use rand_chacha::ChaCha20Rng;
use serde::{Deserialize, Serialize};
use spora_hashes::{Hash, Hasher, HasherBase, MuHashElementHash, MuHashFinalizeHash};
use spora_math::Uint3072;
use std::error::Error;
use std::fmt::Display;

pub const HASH_SIZE: usize = 32;
pub const SERIALIZED_MUHASH_SIZE: usize = ELEMENT_BYTE_SIZE;
// The hash of `NewMuHash().Finalize()`
pub const EMPTY_MUHASH: Hash = Hash::from_bytes([
    0xa6, 0x21, 0xf2, 0xef, 0x4d, 0x16, 0xcb, 0xea, 0x35, 0xf2, 0x5f, 0xad, 0x40, 0x31, 0xe4, 0x42, 0xd3, 0x32, 0x2c, 0xbe, 0x15,
    0x69, 0xaa, 0x90, 0x30, 0x0a, 0xff, 0x47, 0xb6, 0x61, 0x2c, 0x51,
]);

pub(crate) const ELEMENT_BIT_SIZE: usize = 3072;
pub(crate) const ELEMENT_BYTE_SIZE: usize = ELEMENT_BIT_SIZE / 8;

/// MuHash is a type used to create a Multiplicative Hash
/// which is a rolling(homomorphic) hash that you can add and remove elements from
/// and receive the same resulting hash as-if you never hashed them.
/// Because of that the order of adding and removing elements doesn't matter.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MuHash {
    numerator: U3072,
    denominator: U3072,
}

#[derive(Debug, PartialEq, Eq)]
pub struct OverflowError;

impl Display for OverflowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Overflow in the MuHash field")
    }
}

impl Error for OverflowError {}

impl MuHash {
    #[inline]
    /// return an empty initialized set.
    /// when finalized it should be equal to a finalized set with all elements removed.
    pub fn new() -> Self {
        Self { numerator: U3072::one(), denominator: U3072::one() }
    }

    #[inline]
    // hashes the data and adds it to the muhash.
    // Supports arbitrary length data (subject to the underlying hash function(Blake2b) limits)
    pub fn add_element(&mut self, data: &[u8]) {
        let element = data_to_element(data);
        self.numerator *= element;
    }

    #[inline]
    // hashes the data and removes it from the muhash.
    // Supports arbitrary length data (subject to the underlying hash function(Blake2b) limits)
    pub fn remove_element(&mut self, data: &[u8]) {
        let element = data_to_element(data);
        self.denominator *= element;
    }

    #[inline]
    // returns a hasher for hashing data which on `finalize` adds the finalized hash to the muhash.
    pub fn add_element_builder(&mut self) -> MuHashElementBuilder<'_> {
        MuHashElementBuilder::new(&mut self.numerator)
    }

    #[inline]
    // returns a hasher for hashing data which on `finalize` removes the finalized hash from the muhash.
    pub fn remove_element_builder(&mut self) -> MuHashElementBuilder<'_> {
        MuHashElementBuilder::new(&mut self.denominator)
    }

    #[inline]
    // will add the MuHash together. Equivalent to manually adding all the data elements
    // from one set to the other.
    pub fn combine(&mut self, other: &Self) {
        self.numerator *= other.numerator;
        self.denominator *= other.denominator;
    }

    #[inline]
    pub fn finalize(&mut self) -> Hash {
        let serialized = self.serialize();
        MuHashFinalizeHash::hash(serialized)
    }

    #[inline]
    fn normalize(&mut self) {
        self.numerator /= self.denominator;
        self.denominator = U3072::one();
    }

    #[inline]
    pub fn serialize(&mut self) -> [u8; SERIALIZED_MUHASH_SIZE] {
        self.normalize();
        self.numerator.to_le_bytes()
    }

    #[inline]
    pub fn deserialize(data: [u8; SERIALIZED_MUHASH_SIZE]) -> Result<Self, OverflowError> {
        let numerator = U3072::from_le_bytes(data);
        if numerator.is_overflow() {
            Err(OverflowError)
        } else {
            Ok(Self { numerator, denominator: U3072::one() })
        }
    }
}

#[derive(Debug)]
pub enum MuHashError {
    NonNormalizedValue,
}

impl TryFrom<MuHash> for Uint3072 {
    type Error = MuHashError;

    fn try_from(value: MuHash) -> Result<Self, Self::Error> {
        if value.denominator == U3072::one() {
            Ok(value.numerator.into())
        } else {
            Err(MuHashError::NonNormalizedValue)
        }
    }
}

impl From<Uint3072> for MuHash {
    fn from(u: Uint3072) -> Self {
        MuHash { numerator: u.into(), denominator: U3072::one() }
    }
}

pub struct MuHashElementBuilder<'a> {
    muhash_field: &'a mut U3072,
    element_hasher: MuHashElementHash,
}

impl HasherBase for MuHashElementBuilder<'_> {
    fn update<A: AsRef<[u8]>>(&mut self, data: A) -> &mut Self {
        self.element_hasher.write(data);
        self
    }
}

impl<'a> MuHashElementBuilder<'a> {
    pub fn new(muhash_field: &'a mut U3072) -> Self {
        Self { muhash_field, element_hasher: MuHashElementHash::new() }
    }

    pub fn finalize(self) {
        let hash = self.element_hasher.finalize();
        let mut stream = ChaCha20Rng::from_seed(hash.as_bytes());
        let mut bytes = [0u8; ELEMENT_BYTE_SIZE];
        stream.fill_bytes(&mut bytes);
        *self.muhash_field *= U3072::from_le_bytes(bytes);
    }
}

#[inline]
fn data_to_element(data: &[u8]) -> U3072 {
    let hash = MuHashElementHash::hash(data);
    let mut stream = ChaCha20Rng::from_seed(hash.as_bytes());
    let mut bytes = [0u8; ELEMENT_BYTE_SIZE];
    stream.fill_bytes(&mut bytes);
    U3072::from_le_bytes(bytes)
}

impl Default for MuHash {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use crate::OverflowError;
    use crate::{MuHash, EMPTY_MUHASH, U3072};
    use rand::{Rng, SeedableRng};
    use rand_chacha::ChaCha8Rng;
    use spora_hashes::Hash;

    struct TestVector {
        data: &'static [u8],
        multiset_hash: Hash,
        cumulative_hash: Hash,
    }

    const TEST_VECTORS: [TestVector; 3] = [
        TestVector {
            data: &[
                152, 32, 81, 253, 30, 75, 167, 68, 187, 190, 104, 14, 31, 238, 20, 103, 123, 161, 163, 195, 84, 11, 247, 177, 205,
                182, 6, 232, 87, 35, 62, 14, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 242, 5, 42, 1, 0, 0, 0, 67, 65, 4, 150, 181, 56, 232, 83,
                81, 156, 114, 106, 44, 145, 230, 30, 193, 22, 0, 174, 19, 144, 129, 58, 98, 124, 102, 251, 139, 231, 148, 123, 230,
                60, 82, 218, 117, 137, 55, 149, 21, 212, 224, 166, 4, 248, 20, 23, 129, 230, 34, 148, 114, 17, 102, 191, 98, 30, 115,
                168, 44, 191, 35, 66, 200, 88, 238, 172,
            ],
            multiset_hash: Hash::from_bytes([
                0x94, 0x5d, 0xa7, 0xb7, 0x01, 0xd1, 0x75, 0xbf, 0xb7, 0xf0, 0x56, 0x0f, 0x52, 0xab, 0xe1, 0x09, 0x25, 0x81, 0x3b,
                0x84, 0x5c, 0xf1, 0xa9, 0xd7, 0xac, 0x5a, 0x65, 0x59, 0x07, 0xca, 0x09, 0xbd,
            ]),
            cumulative_hash: Hash::from_bytes([
                0x94, 0x5d, 0xa7, 0xb7, 0x01, 0xd1, 0x75, 0xbf, 0xb7, 0xf0, 0x56, 0x0f, 0x52, 0xab, 0xe1, 0x09, 0x25, 0x81, 0x3b,
                0x84, 0x5c, 0xf1, 0xa9, 0xd7, 0xac, 0x5a, 0x65, 0x59, 0x07, 0xca, 0x09, 0xbd,
            ]),
        },
        TestVector {
            data: &[
                213, 253, 204, 84, 30, 37, 222, 28, 122, 90, 221, 237, 242, 72, 88, 184, 187, 102, 92, 159, 54, 239, 116, 78, 228, 44,
                49, 96, 34, 201, 15, 155, 0, 0, 0, 0, 2, 0, 0, 0, 1, 0, 242, 5, 42, 1, 0, 0, 0, 67, 65, 4, 114, 17, 168, 36, 245, 91,
                80, 82, 40, 228, 195, 213, 25, 76, 31, 207, 170, 21, 164, 86, 171, 223, 55, 249, 185, 217, 122, 64, 64, 175, 192, 115,
                222, 230, 200, 144, 100, 152, 79, 3, 56, 82, 55, 217, 33, 103, 193, 62, 35, 100, 70, 180, 23, 171, 121, 160, 252, 174,
                65, 42, 227, 49, 107, 119, 172,
            ],
            multiset_hash: Hash::from_bytes([
                0x28, 0xb5, 0xa2, 0x3b, 0xbf, 0x07, 0x1c, 0x54, 0x8e, 0x2b, 0xc3, 0x66, 0xce, 0xe8, 0xee, 0xf6, 0x7e, 0x4b, 0x99,
                0x37, 0x3a, 0x9a, 0x27, 0x58, 0x56, 0x6d, 0x3c, 0x16, 0x63, 0xa3, 0x7b, 0xfe,
            ]),
            cumulative_hash: Hash::from_bytes([
                0x7c, 0x33, 0xb9, 0x47, 0xe8, 0xb1, 0xeb, 0x57, 0x61, 0xaf, 0x4d, 0xb5, 0x14, 0x8b, 0x9d, 0x81, 0x5e, 0x92, 0xa8,
                0x11, 0x22, 0xb6, 0x9c, 0x8d, 0x98, 0xe7, 0x0d, 0x78, 0x95, 0xd9, 0x4c, 0x93,
            ]),
        },
        TestVector {
            data: &[
                68, 246, 114, 34, 96, 144, 216, 93, 185, 169, 242, 251, 254, 95, 15, 150, 9, 179, 135, 175, 123, 229, 183, 251, 183,
                161, 118, 124, 131, 28, 158, 153, 0, 0, 0, 0, 3, 0, 0, 0, 1, 0, 242, 5, 42, 1, 0, 0, 0, 67, 65, 4, 148, 185, 211, 231,
                108, 91, 22, 41, 236, 249, 127, 255, 149, 215, 164, 187, 218, 200, 124, 194, 96, 153, 173, 162, 128, 102, 198, 255,
                30, 185, 25, 18, 35, 205, 137, 113, 148, 160, 141, 12, 39, 38, 197, 116, 127, 29, 180, 158, 140, 249, 14, 117, 220,
                62, 53, 80, 174, 155, 48, 8, 111, 60, 213, 170, 172,
            ],
            multiset_hash: Hash::from_bytes([
                0xee, 0x9c, 0xe8, 0x12, 0x78, 0x9d, 0x30, 0x10, 0x7e, 0x46, 0x1e, 0x25, 0x84, 0xf4, 0xfc, 0xff, 0x68, 0x3f, 0x62,
                0x09, 0x5e, 0x2d, 0x83, 0xc8, 0xe9, 0xd2, 0x1a, 0x75, 0xe3, 0xe7, 0xe7, 0x1a,
            ]),
            cumulative_hash: Hash::from_bytes([
                0x58, 0x71, 0x46, 0x37, 0xd4, 0x05, 0x56, 0xc3, 0xfa, 0xdf, 0xfe, 0x7e, 0x74, 0x9f, 0x95, 0xb8, 0x6a, 0x22, 0xe0,
                0x71, 0xff, 0x96, 0x02, 0xd3, 0x41, 0x56, 0x35, 0x18, 0x82, 0x5a, 0x6f, 0xa0,
            ]),
        },
    ];

    fn element_from_byte(b: u8) -> [u8; 32] {
        let mut out = [0u8; 32];
        out[0] = b;
        out
    }

    #[test]
    fn test_random_muhash_arithmetic() {
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        for _ in 0..10 {
            let mut res = Hash::default();
            let mut table = [0u8; 4];
            rng.fill(&mut table[..]);

            for order in 0..4 {
                let mut acc = MuHash::new();
                for i in 0..4 {
                    let t = table[i ^ order];
                    if (t & 4) != 0 {
                        acc.remove_element(&element_from_byte(t & 3));
                    } else {
                        acc.add_element(&element_from_byte(t & 3));
                    }
                }
                let out = acc.finalize();
                if order == 0 {
                    res = out;
                } else {
                    assert_eq!(res, out);
                }
            }
            let x = element_from_byte(rng.gen()); // x=X
            let y = element_from_byte(rng.gen()); // x=X, y=Y
            let mut z = MuHash::new(); // x=X, y=X, z=1
            let mut yx = MuHash::new(); // x=X, y=Y, z=1 yx=1
            yx.add_element(&y); // x=X, y=X, z=1, yx=Y
            yx.add_element(&x); // x=X, y=X, z=1, yx=Y*X
            z.add_element(&x); // x=X, y=Y, z=X, yx=Y*X
            z.add_element(&y); // x=X, y=Y, z=X*Y, yx = Y*X
            z.denominator *= yx.numerator; // x=X, y=Y, z=1, yx=Y*X
            assert_eq!(z.finalize(), EMPTY_MUHASH);
        }
    }

    #[test]
    fn test_empty_hash_hex() {
        let hash = MuHash::new().finalize();
        let hexs: String = hash.as_bytes().into_iter().map(|b| format!("{b:#04x}, ")).collect();
        insta::assert_snapshot!(hexs, @"0xa6, 0x21, 0xf2, 0xef, 0x4d, 0x16, 0xcb, 0xea, 0x35, 0xf2, 0x5f, 0xad, 0x40, 0x31, 0xe4, 0x42, 0xd3, 0x32, 0x2c, 0xbe, 0x15, 0x69, 0xaa, 0x90, 0x30, 0x0a, 0xff, 0x47, 0xb6, 0x61, 0x2c, 0x51,");
    }

    #[test]
    fn test_empty_hash() {
        let mut empty = MuHash::new();
        assert_eq!(empty.finalize(), EMPTY_MUHASH);
    }

    #[test]
    fn test_new_pre_computed() {
        let expected = "dd06d968546bc5f9aa344a0915fbf2688652df50091e6e4014d1bd33e77f478a";
        let mut acc = MuHash::new();
        acc.add_element(&element_from_byte(0));
        acc.add_element(&element_from_byte(1));
        acc.remove_element(&element_from_byte(2));
        assert_eq!(acc.finalize().to_string(), expected);
    }

    #[test]
    fn test_serialize() {
        let expected = [
            95, 236, 157, 218, 194, 9, 144, 65, 166, 21, 22, 166, 177, 171, 228, 157, 149, 144, 201, 140, 140, 30, 64, 189, 172, 221,
            202, 145, 32, 138, 8, 92, 182, 171, 6, 42, 202, 160, 49, 90, 45, 197, 53, 242, 192, 190, 102, 225, 90, 204, 160, 224, 92,
            92, 28, 56, 200, 174, 47, 195, 55, 96, 172, 151, 68, 213, 71, 20, 63, 86, 62, 142, 41, 97, 178, 15, 1, 131, 64, 184, 156,
            65, 95, 30, 183, 72, 40, 249, 233, 156, 45, 115, 184, 139, 217, 162, 4, 153, 132, 72, 0, 11, 96, 235, 44, 241, 195, 120,
            86, 30, 73, 177, 225, 110, 160, 94, 207, 79, 121, 222, 163, 59, 83, 24, 33, 63, 100, 164, 105, 213, 25, 90, 236, 163, 38,
            190, 141, 164, 82, 43, 179, 30, 224, 226, 89, 244, 237, 102, 69, 9, 171, 45, 182, 109, 120, 29, 187, 248, 38, 183, 105,
            55, 24, 202, 255, 20, 237, 39, 123, 78, 221, 130, 165, 18, 35, 119, 81, 242, 61, 31, 135, 32, 203, 64, 188, 248, 207, 197,
            204, 188, 124, 229, 228, 119, 215, 185, 177, 181, 68, 5, 47, 4, 184, 148, 165, 138, 227, 109, 166, 56, 160, 126, 34, 196,
            84, 11, 230, 144, 12, 10, 150, 0, 125, 241, 132, 229, 61, 247, 212, 50, 38, 23, 249, 74, 184, 170, 54, 92, 64, 119, 42,
            77, 87, 203, 159, 42, 145, 207, 245, 112, 93, 33, 86, 14, 91, 37, 212, 149, 148, 149, 60, 13, 24, 86, 6, 73, 72, 159, 206,
            186, 224, 132, 2, 61, 89, 239, 4, 145, 69, 54, 64, 202, 84, 60, 50, 33, 41, 97, 222, 124, 159, 139, 54, 58, 119, 232, 31,
            207, 218, 218, 228, 255, 42, 203, 247, 134, 103, 24, 238, 26, 190, 22, 107, 52, 144, 164, 46, 253, 179, 137, 89, 197, 10,
            68, 209, 106, 35, 44, 222, 33, 72, 197, 186, 14, 195, 245, 160, 156, 172, 8, 8, 255, 25, 151, 22, 198, 155, 6, 78, 26,
            215, 110, 120, 12, 66, 97, 226, 90, 76, 60, 41, 17, 26, 8, 237, 33, 82, 67, 29, 86, 212, 104, 214, 9, 89, 28, 111, 94, 94,
            9, 140, 103, 152, 59,
        ];

        let mut check = MuHash::new();
        check.add_element(&element_from_byte(1));
        check.add_element(&element_from_byte(2));
        let ser = check.serialize();
        assert_eq!(&ser[..], &expected[..]);

        let mut deserialized = MuHash::deserialize(ser).unwrap();
        assert_eq!(deserialized.finalize(), check.finalize());
        let overflow = [255; 384];
        assert_eq!(MuHash::deserialize(overflow).unwrap_err(), OverflowError);

        let mut zeroed = MuHash::new();
        zeroed.numerator *= U3072::zero();
        assert_eq!(zeroed.serialize(), [0u8; 384]);

        let mut deserialized = MuHash::deserialize(zeroed.serialize()).unwrap();
        assert_eq!(zeroed.finalize(), deserialized.finalize());
    }

    #[test]
    fn test_vectors_hash() {
        for test in TEST_VECTORS {
            let mut m = MuHash::new();
            m.add_element(test.data);
            let hash = m.finalize();
            println!("test_vectors_hash: left: {:?}, right: {:?}", hash, test.multiset_hash);
            assert_eq!(hash, test.multiset_hash);
        }
    }
    #[test]
    fn test_vectors_add_remove() {
        let mut m = MuHash::new();

        for test in TEST_VECTORS {
            m.add_element(test.data);
            let hash = m.finalize();
            println!("test_vectors_add_remove: left: {:?}, right: {:?}", hash, test.cumulative_hash);
            assert_eq!(hash, test.cumulative_hash);
        }

        for (i, test) in TEST_VECTORS.iter().enumerate().rev() {
            m.remove_element(test.data);
            if i != 0 {
                assert_eq!(m.finalize(), TEST_VECTORS[i - 1].cumulative_hash);
            }
        }
        assert_eq!(m.finalize(), EMPTY_MUHASH);
    }

    #[test]
    fn test_vectors_combine_subtract() {
        let mut m1 = MuHash::new();
        let mut m2 = MuHash::new();
        for test in TEST_VECTORS {
            m1.add_element(test.data);
            m2.remove_element(test.data);
        }
        let m1_orig = m1.clone();
        m1.combine(&m2);
        m2.combine(&m1_orig);
        assert_eq!(m1.finalize(), m2.finalize());
        assert_eq!(m1.finalize(), EMPTY_MUHASH);
    }

    #[test]
    fn test_vectors_commutativity() {
        // Here we first remove an element from an empty multiset, and then add some other
        // elements, and then we create a new empty multiset, then we add the same elements
        // we added to the previous multiset, and then we remove the same element we remove
        // the same element we removed from the previous multiset. According to commutativity
        // laws, the result should be the same.
        for (remove_index, _) in TEST_VECTORS.iter().enumerate() {
            let remove_data = TEST_VECTORS[remove_index].data;
            let mut m1 = MuHash::new();
            let mut m2 = MuHash::new();
            m1.remove_element(remove_data);
            for (i, test) in TEST_VECTORS.iter().enumerate() {
                if i != remove_index {
                    m1.add_element(test.data);
                    m2.add_element(test.data);
                }
            }
            m2.remove_element(remove_data);
            assert_eq!(m1.finalize(), m2.finalize());
        }
    }

    #[test]
    fn test_parse_muhash_fail() {
        let mut serialized = [255; 384];
        serialized[0..3].copy_from_slice(&[155, 40, 239]);

        assert_eq!(MuHash::deserialize(serialized).unwrap_err(), OverflowError);

        serialized[0] = 0;
        let _ = MuHash::deserialize(serialized).unwrap();
    }

    #[test]
    fn test_muhash_add_remove() {
        const LOOPS: usize = 1024;
        let mut rng = ChaCha8Rng::seed_from_u64(42);
        let mut set = MuHash::new();
        let list: Vec<_> = (0..LOOPS)
            .map(|_| {
                let mut data = [0u8; 100];
                rng.fill(&mut data[..]);
                set.add_element(&data);
                data
            })
            .collect();

        assert_ne!(set.finalize(), EMPTY_MUHASH);

        for elem in list.iter() {
            set.remove_element(elem);
        }

        assert_eq!(set.finalize(), EMPTY_MUHASH);
    }
}
