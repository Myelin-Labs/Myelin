//! Portable snapshot codec shared by the native adapter and browser checkers.
//! Borrowed state decoding never allocates an attacker-declared buffer.
use serde::{Deserialize, Serialize};

const MAGIC: &[u8; 5] = b"MCAS\x01";

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot<'a> {
    pub profile: [u8; 32],
    pub cursor: u64,
    pub logical_time: u64,
    #[serde(borrow)]
    pub state: &'a [u8],
}
impl Snapshot<'_> {
    pub fn encode(&self, limit: u64) -> Result<Vec<u8>, String> {
        if self.state.len() as u64 > limit {
            return Err("application snapshot byte limit".into());
        }
        let size = postcard::experimental::serialized_size(self).map_err(|e| e.to_string())?;
        if (size as u64).checked_add(MAGIC.len() as u64).is_none_or(|total| total > limit) {
            return Err("application snapshot byte limit".into());
        }
        let mut bytes = MAGIC.to_vec();
        bytes.extend(postcard::to_allocvec(self).map_err(|e| e.to_string())?);
        Ok(bytes)
    }
}
pub fn decode(bytes: &[u8], profile: [u8; 32], limit: u64) -> Result<Snapshot<'_>, String> {
    if bytes.len() as u64 > limit || !bytes.starts_with(MAGIC) {
        return Err("unknown or oversized application snapshot".into());
    }
    let (value, rest): (Snapshot<'_>, _) = postcard::take_from_bytes(&bytes[MAGIC.len()..]).map_err(|e| e.to_string())?;
    if !rest.is_empty() || value.profile != profile || value.encode(limit)? != bytes {
        return Err("noncanonical application snapshot or profile mismatch".into());
    }
    Ok(value)
}
pub fn state_root(bytes: &[u8]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(b"myelin:command-application-state:v1");
    h.update(bytes);
    *h.finalize().as_bytes()
}
