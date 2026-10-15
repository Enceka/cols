//! Persistent public and private player profiles.
use crate::contracts::account::AccountId;
use crate::infra::storage::accounts::AccountStore;
use crate::infra::storage::atomic_private_write;
use crate::infra::storage::read_bounded;
use anyhow::Result;
use codol_protocol::lobby::{profile, task};
use std::sync::Arc;

pub(crate) struct ProfileService {
    store: Arc<AccountStore>,
}
impl ProfileService {
    pub(crate) fn new(store: Arc<AccountStore>) -> Self {
        Self { store }
    }
    pub(crate) async fn read(
        &self,
        session: impl Into<AccountId>,
        private: bool,
    ) -> Result<Option<Vec<u8>>> {
        let session = session.into();
        if private
            && let Some(committed) =
                crate::backend::progression::service::committed_private(&self.store, session)
                    .await?
        {
            return Ok(Some(committed));
        }
        let path = self.store.profile_path(session, private).await?;
        read_bounded(&path, task::MAX_FILE_BYTES).await
    }
    pub(crate) async fn save_public(
        &self,
        session: impl Into<AccountId>,
        data: &[u8],
    ) -> Result<()> {
        let session = session.into();
        profile::validate_public(data)?;
        let _lock = self.store.writes.lock().await;
        let path = self.store.profile_path(session, false).await?;
        atomic_private_write(&path, data).await
    }
}
