use crate::{Error, Result};
use myelin_session::bounded_codec;
use serde::{de::DeserializeOwned, Serialize};

const HEADER: usize = 46;
fn version(magic: &[u8; 4]) -> u16 {
    if magic == b"MTRS" {
        2
    } else {
        1
    }
}

pub(crate) fn encode<T: Serialize>(magic: &[u8; 4], value: &T, cap: usize) -> Result<Vec<u8>> {
    let cap = cap.min(bounded_codec::MAX_RECORD_BYTES);
    let body_cap = cap.checked_sub(HEADER).ok_or(Error::Bounds("record cap"))?;
    bounded_codec::size_with_limit(value, "transfer_record", body_cap as u64).map_err(|e| Error::Codec(e.to_string()))?;
    let body = postcard::to_allocvec(value).map_err(|e| Error::Codec(e.to_string()))?;
    let mut bytes = Vec::with_capacity(HEADER + body.len());
    bytes.extend(magic);
    bytes.extend(version(magic).to_le_bytes());
    bytes.extend((body.len() as u64).to_le_bytes());
    bytes.extend(blake3::hash(&body).as_bytes());
    bytes.extend(body);
    Ok(bytes)
}

pub(crate) fn decode<T: DeserializeOwned + Serialize>(magic: &[u8; 4], bytes: &[u8], cap: usize) -> Result<T> {
    if bytes.len() < HEADER
        || bytes.len() > cap.min(bounded_codec::MAX_RECORD_BYTES)
        || !bytes.starts_with(magic)
        || bytes[4..6] != version(magic).to_le_bytes()
    {
        return Err(Error::Codec("invalid transfer record envelope/version/size".into()));
    }
    let body = &bytes[HEADER..];
    if u64::from_le_bytes(bytes[6..14].try_into().expect("header checked")) != body.len() as u64
        || blake3::hash(body).as_bytes() != &bytes[14..46]
    {
        return Err(Error::Codec("transfer record length/checksum mismatch".into()));
    }
    let (value, rest): (T, &[u8]) = postcard::take_from_bytes(body).map_err(|e| Error::Codec(e.to_string()))?;
    if !rest.is_empty() || encode(magic, &value, cap)? != bytes {
        return Err(Error::Codec("noncanonical or trailing transfer record bytes".into()));
    }
    Ok(value)
}

pub(crate) fn hash<T: Serialize>(domain: &[u8], value: &T) -> Result<[u8; 32]> {
    bounded_codec::size_with_limit(value, "transfer_hash", bounded_codec::MAX_RECORD_BYTES as u64)
        .map_err(|e| Error::Codec(e.to_string()))?;
    let bytes = postcard::to_allocvec(value).map_err(|e| Error::Codec(e.to_string()))?;
    let mut h = blake3::Hasher::new();
    h.update(domain);
    h.update(&bytes);
    Ok(*h.finalize().as_bytes())
}
