//! A reference economic-effect boundary: canonical authentication, permanent
//! deduplication and application state commit in the same synchronous DB write.
use crate::{protocol, AuthorityError, CanonicalEffect, FencingPolicy, Result};
use myelin_session::{Hash32, OutboxMessage};
use rocksdb::{Options, WriteOptions, DB};
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::Mutex};

const KEY: &[u8] = b"myelin-canonical-effect-store";
const MAGIC: &[u8] = b"MEFX\x01";
const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_STATE_BYTES: usize = 1024 * 1024;
const MAX_EFFECTS: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectDisposition {
    Applied,
    Duplicate,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Seen {
    id: Hash32,
    message: Hash32,
    block: Hash32,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    policy: FencingPolicy,
    state: Vec<u8>,
    seen: Vec<Seen>,
}
struct Inner {
    snapshot: Snapshot,
    uncertain: bool,
}
pub struct CanonicalEffectStore {
    db: DB,
    inner: Mutex<Inner>,
}
impl CanonicalEffectStore {
    pub fn open(path: impl AsRef<Path>, policy: FencingPolicy, initial: Vec<u8>) -> Result<Self> {
        if initial.len() > MAX_STATE_BYTES {
            return Err(AuthorityError::Invalid("effect state limit exceeded".into()));
        }
        let mut options = Options::default();
        options.create_if_missing(true);
        let db = DB::open(&options, path).map_err(super::storage)?;
        let snapshot = match db.get(KEY).map_err(super::storage)? {
            Some(bytes) => decode(&bytes)?,
            None => {
                if db.iterator(rocksdb::IteratorMode::Start).next().transpose().map_err(super::storage)?.is_some() {
                    return Err(AuthorityError::Invalid("nonempty database has no canonical effect record".into()));
                }
                let value = Snapshot { policy: policy.clone(), state: initial, seen: vec![] };
                persist(&db, &value)?;
                value
            }
        };
        if snapshot.policy != policy {
            return Err(AuthorityError::Invalid("effect store trust policy differs".into()));
        }
        Ok(Self { db, inner: Mutex::new(Inner { snapshot, uncertain: false }) })
    }
    fn locked(&self) -> Result<std::sync::MutexGuard<'_, Inner>> {
        let inner = self.inner.lock().map_err(|_| AuthorityError::Uncertain("effect mutex poisoned".into()))?;
        if inner.uncertain {
            return Err(AuthorityError::Uncertain("effect write requires reopen".into()));
        }
        Ok(inner)
    }
    pub fn state(&self) -> Result<Vec<u8>> {
        Ok(self.locked()?.snapshot.state.clone())
    }
    /// The callback mutates only this store's application state. Network calls
    /// or other databases inside it do not become atomic; use another outbox.
    pub fn apply(
        &self,
        certificate: &CanonicalEffect,
        message: &OutboxMessage,
        apply: impl FnOnce(&[u8], &OutboxMessage) -> std::result::Result<Vec<u8>, String>,
    ) -> Result<EffectDisposition> {
        let mut inner = self.locked()?;
        certificate.verify(&inner.snapshot.policy, message)?;
        if let Some(seen) = inner.snapshot.seen.iter().find(|seen| seen.id == message.id) {
            if seen.message != certificate.message_commitment || seen.block != certificate.receipt.block_hash {
                return Err(AuthorityError::Invalid("canonical message ID conflicts with retained effect".into()));
            }
            return Ok(EffectDisposition::Duplicate);
        }
        if inner.snapshot.seen.len() >= MAX_EFFECTS {
            return Err(AuthorityError::Invalid("effect retention capacity reached".into()));
        }
        let state = apply(&inner.snapshot.state, message).map_err(AuthorityError::Session)?;
        if state.len() > MAX_STATE_BYTES {
            return Err(AuthorityError::Invalid("effect state limit exceeded".into()));
        }
        let mut next = inner.snapshot.clone();
        next.state = state;
        next.seen.push(Seen {
            id: message.id,
            message: protocol::message_commitment(message)?,
            block: certificate.receipt.block_hash,
        });
        if let Err(error) = persist(&self.db, &next) {
            inner.uncertain = true;
            return Err(AuthorityError::Uncertain(error.to_string()));
        }
        inner.snapshot = next;
        Ok(EffectDisposition::Applied)
    }
}
fn persist(db: &DB, value: &Snapshot) -> Result<()> {
    let body = serde_json::to_vec(value).map_err(|e| AuthorityError::Invalid(e.to_string()))?;
    if body.len() + MAGIC.len() + 32 > MAX_BYTES {
        return Err(AuthorityError::Invalid("effect record size limit exceeded".into()));
    }
    let mut bytes = MAGIC.to_vec();
    bytes.extend_from_slice(blake3::hash(&body).as_bytes());
    bytes.extend_from_slice(&body);
    let mut options = WriteOptions::default();
    options.set_sync(true);
    db.put_opt(KEY, bytes, &options).map_err(super::storage)
}
fn decode(bytes: &[u8]) -> Result<Snapshot> {
    if bytes.len() < MAGIC.len() + 32 || bytes.len() > MAX_BYTES || &bytes[..MAGIC.len()] != MAGIC {
        return Err(AuthorityError::Invalid("effect format/size invalid".into()));
    }
    let body = &bytes[MAGIC.len() + 32..];
    if &bytes[MAGIC.len()..MAGIC.len() + 32] != blake3::hash(body).as_bytes() {
        return Err(AuthorityError::Invalid("effect checksum mismatch".into()));
    }
    let value: Snapshot = serde_json::from_slice(body).map_err(|e| AuthorityError::Invalid(e.to_string()))?;
    let ids: std::collections::HashSet<_> = value.seen.iter().map(|s| s.id).collect();
    if value.state.len() > MAX_STATE_BYTES || value.seen.len() > MAX_EFFECTS || ids.len() != value.seen.len() {
        return Err(AuthorityError::Invalid("effect state/retention invalid".into()));
    }
    Ok(value)
}
