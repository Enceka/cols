//! Base item ownership loaded for an authenticated account.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalInventory {
    schema_version: u32,
    client_sha256: String,
    title_id: u32,
    user_id: String,
    items: Vec<u32>,
}

// A pretty-printed complete target catalogue exceeds the old 32 KiB starter
// file bound. This also bounds hostile JSON whitespace before deserialization.
const MAX_INVENTORY_FILE_BYTES: usize = 256 * 1024;

use crate::contracts::account::AccountId;
use crate::infra::storage::accounts::AccountStore;
use crate::infra::storage::read_bounded;
use anyhow::{Result, ensure};
use codol_protocol::lobby::task;
use serde::Deserialize;
use std::sync::Arc;

pub(crate) struct InventoryService {
    store: Arc<AccountStore>,
}
impl InventoryService {
    pub(crate) fn new(store: Arc<AccountStore>) -> Self {
        Self { store }
    }
    pub(crate) async fn with_progress(
        &self,
        account: impl Into<AccountId>,
        owned: &[u32],
    ) -> Result<Vec<task::InventoryItem>> {
        crate::backend::progression::service::inventory_progress(&self.store, account.into(), owned)
            .await
    }
    pub(crate) async fn items(&self, session: impl Into<AccountId>) -> Result<Vec<u32>> {
        let session = session.into();
        let path = self
            .store
            .profile_path(session, false)
            .await?
            .with_file_name("inventory.json");
        let Some(bytes) = read_bounded(&path, MAX_INVENTORY_FILE_BYTES).await? else {
            return Ok(Vec::new());
        };
        let inventory: LocalInventory = serde_json::from_slice(&bytes)?;
        ensure!(
            inventory.schema_version == 1
                && inventory.client_sha256 == codol_protocol::CLIENT_SHA256
                && inventory.title_id == session.title_id
                && inventory.user_id.parse::<u64>()? == session.user_id,
            "inventory identity/version mismatch"
        );
        task::player_data(session.user_id, &inventory.items)?;
        Ok(inventory.items)
    }
}
