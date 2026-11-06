//! One canonical store can host multiple independently fenced
//! sessions. Raw writable store handles never escape to application workers.
use crate::{protocol, AuthorityError, Result};
use myelin_session::{Hash32, SessionGenesis, SessionStore};
use myelin_session_store_rocksdb::RocksSessionStore;
use std::{
    collections::HashSet,
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
};

pub struct CanonicalLedger {
    pub(crate) store: Arc<RocksSessionStore>,
    active: Mutex<HashSet<Hash32>>,
    commits: Mutex<()>,
}
impl CanonicalLedger {
    pub fn open(path: impl AsRef<Path>) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            store: Arc::new(RocksSessionStore::open(path).map_err(protocol::session)?),
            active: Mutex::new(HashSet::new()),
            commits: Mutex::new(()),
        }))
    }
    /// Genesis records are immutable application dependencies, not writable
    /// store capabilities or permission to activate a restored authority.
    pub fn genesis(&self, session_id: Hash32) -> Result<SessionGenesis> {
        self.store.load_genesis(session_id).map_err(protocol::session)
    }
    pub(crate) fn register(self: &Arc<Self>, id: Hash32) -> Result<LedgerSession> {
        let mut active = self.active.lock().map_err(|_| AuthorityError::Uncertain("ledger registration poisoned".into()))?;
        if !active.insert(id) {
            return Err(AuthorityError::Invalid("canonical session already has a live authority".into()));
        }
        Ok(LedgerSession { ledger: self.clone(), id })
    }
    pub(crate) fn commit_lock(&self) -> Result<MutexGuard<'_, ()>> {
        self.commits.lock().map_err(|_| AuthorityError::Uncertain("canonical ledger commit interrupted".into()))
    }
}
pub(crate) struct LedgerSession {
    pub(crate) ledger: Arc<CanonicalLedger>,
    id: Hash32,
}
impl Drop for LedgerSession {
    fn drop(&mut self) {
        if let Ok(mut active) = self.ledger.active.lock() {
            active.remove(&self.id);
        }
    }
}
