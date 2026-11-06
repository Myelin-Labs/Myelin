//! Allocation limits for bounded record imports. These local codec ceilings are
//! distinct from a session's genesis-bound validity limits.
use crate::{Result, SessionError};
use serde::{
    de::{Deserialize, Deserializer, Error, SeqAccess, Visitor},
    Serialize,
};
use std::{fmt, marker::PhantomData};

pub const MAX_RECORD_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_SEQUENCE_ITEMS: usize = 65_536;

pub fn bounded_bytes<'de, D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Vec<u8>, D::Error> {
    struct Bytes;
    impl<'de> Visitor<'de> for Bytes {
        type Value = Vec<u8>;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            write!(f, "bounded bytes")
        }
        fn visit_bytes<E: Error>(self, bytes: &[u8]) -> std::result::Result<Vec<u8>, E> {
            if bytes.len() > MAX_RECORD_BYTES {
                return Err(E::custom("byte length exceeds decoder ceiling"));
            }
            Ok(bytes.to_vec())
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<Vec<u8>, A::Error> {
            if seq.size_hint().is_some_and(|len| len > MAX_RECORD_BYTES) {
                return Err(A::Error::custom("byte length exceeds decoder ceiling"));
            }
            let mut bytes = Vec::new();
            while let Some(byte) = seq.next_element()? {
                if bytes.len() == MAX_RECORD_BYTES {
                    return Err(A::Error::custom("byte length exceeds decoder ceiling"));
                }
                bytes.push(byte);
            }
            Ok(bytes)
        }
    }
    deserializer.deserialize_bytes(Bytes)
}

pub fn bounded_string<'de, D: Deserializer<'de>>(deserializer: D) -> std::result::Result<String, D::Error> {
    struct Text;
    impl<'de> Visitor<'de> for Text {
        type Value = String;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            write!(f, "bounded text")
        }
        fn visit_str<E: Error>(self, value: &str) -> std::result::Result<String, E> {
            if value.len() > 4096 {
                return Err(E::custom("string length exceeds decoder ceiling"));
            }
            Ok(value.to_owned())
        }
    }
    deserializer.deserialize_str(Text)
}

pub fn bounded_vec<'de, D, T>(deserializer: D) -> std::result::Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Bounded<T>(PhantomData<T>);
    impl<'de, T: Deserialize<'de>> Visitor<'de> for Bounded<T> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            write!(f, "a bounded sequence")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<Vec<T>, A::Error> {
            let limit = MAX_SEQUENCE_ITEMS;
            if seq.size_hint().is_some_and(|len| len > limit) {
                return Err(A::Error::custom("sequence length exceeds decoder ceiling"));
            }
            let mut result = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(4096));
            while let Some(value) = seq.next_element()? {
                if result.len() >= limit {
                    return Err(A::Error::custom("sequence length exceeds decoder ceiling"));
                }
                result.push(value);
            }
            Ok(result)
        }
    }
    deserializer.deserialize_seq(Bounded(PhantomData))
}

pub fn transactions<'de, D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Vec<Vec<u8>>, D::Error> {
    #[derive(serde::Deserialize)]
    struct Bytes(#[serde(deserialize_with = "bounded_bytes")] Vec<u8>);
    let values: Vec<Bytes> = bounded_vec(deserializer)?;
    Ok(values.into_iter().map(|value| value.0).collect())
}

/// Count without allocating an encoded copy; stop as soon as the cap is exceeded.
pub fn size_with_limit<T: Serialize + ?Sized>(value: &T, resource: &'static str, limit: u64) -> Result<u64> {
    struct Counter {
        used: u64,
        limit: u64,
    }
    impl postcard::ser_flavors::Flavor for Counter {
        type Output = u64;
        fn try_push(&mut self, _: u8) -> postcard::Result<()> {
            self.used = self.used.checked_add(1).ok_or(postcard::Error::SerializeBufferFull)?;
            if self.used > self.limit {
                return Err(postcard::Error::SerializeBufferFull);
            }
            Ok(())
        }
        fn try_extend(&mut self, bytes: &[u8]) -> postcard::Result<()> {
            self.used = self.used.checked_add(bytes.len() as u64).ok_or(postcard::Error::SerializeBufferFull)?;
            if self.used > self.limit {
                return Err(postcard::Error::SerializeBufferFull);
            }
            Ok(())
        }
        fn finalize(self) -> postcard::Result<u64> {
            Ok(self.used)
        }
    }
    postcard::serialize_with_flavor(value, Counter { used: 0, limit }).map_err(|error| {
        if error == postcard::Error::SerializeBufferFull {
            SessionError::ResourceLimit { resource, used: limit.saturating_add(1), limit }
        } else {
            SessionError::Codec(error.to_string())
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize, serde::Serialize, Debug)]
    struct Nested(#[serde(deserialize_with = "transactions")] Vec<Vec<u8>>);

    #[test]
    fn forged_container_lengths_reject_before_parsing_or_reserving_elements() {
        let mut outer = postcard::to_allocvec(&(MAX_SEQUENCE_ITEMS as u64 + 1)).unwrap();
        assert!(postcard::from_bytes::<Nested>(&outer).is_err());
        // Postcard withholds size_hint when fewer bytes remain than the
        // declared count. Supply enough bytes to test its explicit length gate.
        outer.extend(std::iter::repeat_n(0, MAX_SEQUENCE_ITEMS + 1));
        let error = postcard::from_bytes::<Nested>(&outer).unwrap_err();
        assert_eq!(error, postcard::Error::SerdeDeCustom);
        let mut inner = vec![1];
        inner.extend(postcard::to_allocvec(&(MAX_RECORD_BYTES as u64 + 1)).unwrap());
        // Postcard's borrowed byte parser checks actual input length without
        // reserving attacker-declared storage, even before our byte visitor.
        assert!(postcard::from_bytes::<Nested>(&inner).is_err());
        let canonical = postcard::to_allocvec(&Nested(vec![vec![1, 2], vec![]])).unwrap();
        assert_eq!(postcard::from_bytes::<Nested>(&canonical).unwrap().0, vec![vec![1, 2], vec![]]);
    }

    #[test]
    fn size_counter_is_inclusive_and_checks_overflow() {
        let data = vec![7u8; 128];
        let size = postcard::to_allocvec(&data).unwrap().len() as u64;
        assert_eq!(size_with_limit(&data, "test", size).unwrap(), size);
        assert!(matches!(size_with_limit(&data, "test", size - 1), Err(SessionError::ResourceLimit { .. })));
    }
}
