//! Permanent-item purchases with atomic debit and grant persistence.
use super::marketplace::{LocalShop, LocalWallet, Payment};
use crate::contracts::account::AccountId;
use crate::infra::storage::accounts::AccountStore;
use crate::infra::storage::atomic_private_write;
use crate::infra::storage::read_bounded;
use anyhow::Result;
use std::sync::Arc;

pub(crate) struct CommerceService {
    store: Arc<AccountStore>,
}
impl CommerceService {
    pub(crate) fn new(store: Arc<AccountStore>) -> Self {
        Self { store }
    }
    pub(crate) async fn wallet(
        &self,
        session: impl Into<AccountId>,
    ) -> Result<Option<LocalWallet>> {
        let session = session.into();
        let path = self
            .store
            .profile_directory(session.title_id, session.user_id)
            .await?
            .join("wallet.json");
        let Some(bytes) = read_bounded(&path, 32768).await? else {
            return Ok(None);
        };
        let wallet: LocalWallet = serde_json::from_slice(&bytes)?;
        wallet.validate(session.title_id, session.user_id)?;
        Ok(Some(wallet))
    }
    /// Funds and grants commit together under the shared account write lock.
    pub(crate) async fn purchase(
        &self,
        session: impl Into<AccountId>,
        inventory: &crate::backend::inventory::InventoryService,
        sku: u32,
        payment: Payment,
    ) -> Result<Result<(), &'static str>> {
        let session = session.into();
        let _lock = self.store.writes.lock().await;
        let Some(bytes) = read_bounded(&self.store.root.join("shop.json"), 4 * 1024 * 1024).await?
        else {
            return Ok(Err("local_shop_missing"));
        };
        let shop: LocalShop = serde_json::from_slice(&bytes)?;
        shop.validate(session.title_id)?;
        let Some(mut wallet) = self.wallet(session).await? else {
            return Ok(Err("local_wallet_missing"));
        };
        let inventory = inventory.items(session).await?;
        if let Err(reason) = wallet.purchase(&shop, &inventory, sku, payment) {
            tracing::info!(sku, ?payment, %reason, "local purchase rejected");
            return Ok(Err("local_purchase_rejected"));
        }
        // Debit and grant commit together. A write failure never acknowledges success.
        let path = self
            .store
            .profile_directory(session.title_id, session.user_id)
            .await?
            .join("wallet.json");
        atomic_private_write(&path, &serde_json::to_vec_pretty(&wallet)?).await?;
        Ok(Ok(()))
    }
}
