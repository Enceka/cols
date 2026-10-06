//! Per-run test identities, bound to the exact issued server ticket.
use anyhow::{Result, ensure};
use std::{collections::HashMap, sync::Mutex};

#[derive(Clone)]
pub struct Session {
    pub title_id: u32,
    pub user_id: u64,
    pub iv_seed: u32,
    pub expires_at: u32,
    pub key: [u8; 24],
}

#[derive(Default)]
pub struct SessionRegistry {
    entries: Mutex<HashMap<[u8; 128], Session>>,
}
impl SessionRegistry {
    pub fn insert(&self, ticket: [u8; 128], session: Session) -> Result<()> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| anyhow::anyhow!("session registry poisoned"))?;
        let now = crate::capture::unix_time_ms() / 1000;
        entries.retain(|_, s| u128::from(s.expires_at) > now);
        ensure!(entries.len() < 128, "local session registry is full");
        entries.insert(ticket, session);
        Ok(())
    }
    pub fn lookup(&self, ticket: &[u8; 128]) -> Result<Option<Session>> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| anyhow::anyhow!("session registry poisoned"))?;
        Ok(entries
            .get(ticket)
            .filter(|s| u128::from(s.expires_at) > crate::capture::unix_time_ms() / 1000)
            .cloned())
    }
}
