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
    entries: Mutex<RegistryState>,
}
#[derive(Default)]
struct RegistryState {
    tickets: HashMap<[u8; 128], Session>,
    current: HashMap<(u32, u64), Session>,
}
impl SessionRegistry {
    pub fn insert(&self, ticket: [u8; 128], session: Session) -> Result<()> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| anyhow::anyhow!("session registry poisoned"))?;
        let now = crate::capture::unix_time_ms() / 1000;
        entries.tickets.retain(|_, s| {
            u128::from(s.expires_at) > now
                && (s.title_id != session.title_id
                    || s.user_id != session.user_id
                    || (s.iv_seed == session.iv_seed && s.key == session.key))
        });
        entries
            .current
            .retain(|_, s| u128::from(s.expires_at) > now);
        ensure!(
            entries.tickets.len() < 128 || entries.tickets.contains_key(&ticket),
            "local session registry is full"
        );
        entries
            .current
            .insert((session.title_id, session.user_id), session.clone());
        entries.tickets.insert(ticket, session);
        Ok(())
    }
    pub fn lookup(&self, ticket: &[u8; 128]) -> Result<Option<Session>> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| anyhow::anyhow!("session registry poisoned"))?;
        Ok(entries
            .tickets
            .get(ticket)
            .filter(|s| u128::from(s.expires_at) > crate::capture::unix_time_ms() / 1000)
            .cloned())
    }
    /// A valid MAC may renew the currently issued identity, but cannot restore
    /// a ticket superseded by a newer login. Ticket expiry still gates handshakes.
    pub(crate) fn confirm_active(&self, session: &Session) -> Result<bool> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| anyhow::anyhow!("session registry poisoned"))?;
        let Some(current) = entries
            .current
            .get_mut(&(session.title_id, session.user_id))
        else {
            return Ok(false);
        };
        if current.iv_seed != session.iv_seed || current.key != session.key {
            return Ok(false);
        }
        current.expires_at = current.expires_at.max(session.expires_at);
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session() -> Session {
        Session {
            title_id: 1002,
            user_id: 42,
            iv_seed: 1,
            expires_at: u32::MAX,
            key: [1; 24],
        }
    }
    #[test]
    fn newer_login_revokes_old_ticket_and_active_connection_without_revoking_other_users() {
        let registry = SessionRegistry::default();
        let old = session();
        let mut other = old.clone();
        other.user_id = 43;
        registry.insert([1; 128], old.clone()).unwrap();
        registry.insert([2; 128], other.clone()).unwrap();
        let mut new = old.clone();
        new.iv_seed += 1;
        registry.insert([3; 128], new.clone()).unwrap();
        assert!(registry.lookup(&[1; 128]).unwrap().is_none());
        assert!(!registry.confirm_active(&old).unwrap());
        assert!(registry.confirm_active(&new).unwrap());
        assert!(registry.confirm_active(&other).unwrap());
        let mut changed = new.clone();
        changed.key[0] += 1;
        registry.insert([4; 128], changed.clone()).unwrap();
        assert!(registry.lookup(&[3; 128]).unwrap().is_none());
        assert!(!registry.confirm_active(&new).unwrap());
        assert!(registry.confirm_active(&changed).unwrap());
    }
}
