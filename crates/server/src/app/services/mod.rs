//! Application composition for persistent services and live room delivery.
use crate::backend::commerce::service::CommerceService;
use crate::backend::files::FileService;
use crate::backend::identity::session::Session;
use crate::backend::inventory::InventoryService;
use crate::backend::profile::ProfileService;
use crate::backend::publisher::PublisherService;
use crate::backend::rooms::Rooms;
use crate::contracts::account::AccountId;
use crate::contracts::lobby::DirectedPush;
use crate::contracts::lobby::TaskResult;
use crate::infra::storage::accounts::AccountStore;
use crate::infra::storage::read_bounded;
use crate::transport::lobby::delivery::DeliveryHub;
use crate::transport::lobby::delivery::Subscription;
use crate::transport::lobby::delivery::publish_with_cleanup;
use crate::transport::lobby::router::TaskRouter;
use anyhow::{Context, Result, ensure};
use codol_protocol::lobby::{
    profile,
    task::{self, Request},
};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::Mutex;

pub struct LocalServices {
    accounts: Arc<AccountStore>,
    profiles: ProfileService,
    inventory: InventoryService,
    commerce: CommerceService,
    files: FileService,
    publisher: PublisherService,
    rooms: Arc<Mutex<Rooms>>,
    delivery: Arc<DeliveryHub>,
}
pub(crate) struct GameProfile {
    pub public: Vec<u8>,
    pub owned: Vec<u32>,
    pub rank: (u8, u8),
}
impl LocalServices {
    pub async fn open(root: PathBuf) -> Result<Self> {
        let accounts = Arc::new(AccountStore::open(root).await?);
        Ok(Self {
            profiles: ProfileService::new(accounts.clone()),
            inventory: InventoryService::new(accounts.clone()),
            commerce: CommerceService::new(accounts.clone()),
            files: FileService::new(accounts.clone()),
            publisher: PublisherService::new(accounts.clone()),
            accounts,
            rooms: Arc::new(Mutex::new(Rooms::default())),
            delivery: Arc::new(DeliveryHub::default()),
        })
    }
    fn router(&self) -> TaskRouter<'_> {
        TaskRouter {
            rooms: &self.rooms,
            profiles: &self.profiles,
            inventory: &self.inventory,
            commerce: &self.commerce,
            files: &self.files,
            publisher: &self.publisher,
        }
    }
    pub async fn handle(&self, body: &[u8], session: &Session, now: u32) -> Result<TaskResult> {
        self.router().handle(body, session, now).await
    }
    pub async fn publisher_playlist_version(&self) -> Result<u32> {
        self.publisher.playlist_version().await
    }
    pub(crate) async fn player_nickname(&self, account: AccountId) -> Result<Option<String>> {
        crate::backend::players::nickname(&self.accounts.root, account.title_id, account.user_id)
            .await
    }
    pub(crate) async fn subscribe_lobby(&self, session: &Session) -> Subscription {
        let (subscription, replaced) = self.delivery.subscribe(session.into(), self.rooms.clone());
        let mut pushes = Vec::new();
        {
            let mut rooms = self.rooms.lock().await;
            pushes.extend(rooms.discard_changed_credentials(session));
            for identity in replaced {
                pushes.extend(rooms.disconnect(identity));
            }
        }
        publish_with_cleanup(&self.delivery, &self.rooms, pushes).await;
        subscription
    }
    pub(crate) async fn publish_lobby(&self, pushes: Vec<DirectedPush>) {
        publish_with_cleanup(&self.delivery, &self.rooms, pushes).await;
    }
    pub(crate) async fn reap_rooms(&self, now: u32) {
        let pushes = self.rooms.lock().await.reap(now);
        self.publish_lobby(pushes).await;
    }
    pub(crate) async fn handle_connected(
        &self,
        body: &[u8],
        session: &Session,
        now: u32,
        connection: &Subscription,
    ) -> Result<TaskResult> {
        ensure!(connection.is_current(), "lobby connection was replaced");
        let request = Request::parse(body)?;
        if matches!(request.service, 11 | 21) || (request.service, request.task) == (6, 14) {
            let mut rooms = self.rooms.lock().await;
            ensure!(connection.is_current(), "lobby connection was replaced");
            return rooms.handle(&request, session, now);
        }
        self.handle(body, session, now).await
    }
    pub(crate) fn permits_nuke_diagnostics(&self) -> bool {
        self.accounts.root.join(".codol-nuke-acceptance").is_file()
    }
    pub(crate) async fn renew_play_session(&self, session: &Session, now: u32) -> Result<()> {
        self.rooms.lock().await.renew_session(session, now)
    }
    pub async fn with_game_endpoint(
        self,
        endpoint: std::sync::Arc<crate::backend::rooms::admission::GameAdmissions>,
    ) -> Self {
        *self.rooms.lock().await = crate::backend::rooms::Rooms::with_game_endpoint(endpoint);
        self
    }
    pub(crate) async fn game_profile(&self, title_id: u32, user_id: u64) -> Result<GameProfile> {
        let _lock = self.accounts.writes.lock().await;
        let account = AccountId { title_id, user_id };
        let path = self.accounts.profile_path(account, false).await?;
        let public = read_bounded(&path, task::MAX_FILE_BYTES)
            .await?
            .context("saved player loadout is missing")?;
        profile::validate_public(&public)?;
        let rank = match read_bounded(
            &self.accounts.profile_path(account, true).await?,
            task::MAX_FILE_BYTES,
        )
        .await?
        {
            Some(private) => profile::player_rank(&private, user_id)?,
            None => (0, 0),
        };
        let mut owned = self.inventory.items(account).await?;
        if let Some(wallet) = self.commerce.wallet(account).await? {
            owned.extend(wallet.purchases.iter().map(|p| p.item_id));
        }
        owned.sort_unstable();
        owned.dedup();
        Ok(GameProfile {
            public,
            owned,
            rank,
        })
    }
    /// Offline account maintenance; callers must stop other writers first.
    pub async fn boost_test_account(
        &self,
        title_id: u32,
        user_id: u64,
        cp: u32,
        gp: u32,
    ) -> Result<PathBuf> {
        crate::diagnostics::account::boost_test_account(&self.accounts, title_id, user_id, cp, gp)
            .await
    }
}
#[cfg(test)]
mod tests;
