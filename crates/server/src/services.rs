//! Opt-in local services. Opaque user files are isolated by authenticated
//! title/UID; no old-build profile or catalogue data is manufactured.
use crate::marketplace::{LocalShop, LocalWallet, Payment};
use crate::{auth_server::write_private, session::Session};
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use codol_protocol::{
    lobby, profile,
    task::{self, Request, StorageRequest, Writer},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};
use tokio::{io::AsyncReadExt, sync::Mutex};

pub struct LocalServices {
    root: PathBuf,
    writes: Mutex<()>,
    rooms: Mutex<crate::rooms::Rooms>,
}
pub(crate) struct GameProfile {
    pub public: Vec<u8>,
    pub owned: Vec<u32>,
    pub rank: (u8, u8),
}
pub struct TaskResult {
    pub label: &'static str,
    pub result: Result<Vec<Vec<u8>>, u32>,
    /// Unsolicited lobby kind-2 bodies, sent after the task acknowledgement.
    pub pushes: Vec<Vec<u8>>,
}
impl TaskResult {
    pub(crate) fn success(label: &'static str, records: Vec<Vec<u8>>) -> Self {
        Self {
            label,
            result: Ok(records),
            pushes: Vec::new(),
        }
    }
    pub(crate) fn failure(label: &'static str, code: u32) -> Self {
        Self {
            label,
            result: Err(code),
            pushes: Vec::new(),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredFile {
    schema: u32,
    created: u32,
    modified: u32,
    visibility: bool,
    data_base64: String,
}
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

impl LocalServices {
    pub(crate) fn permits_nuke_diagnostics(&self) -> bool {
        self.root.join(".codol-nuke-acceptance").is_file()
    }
    pub(crate) async fn renew_play_session(&self, session: &Session, now: u32) -> Result<()> {
        self.rooms.lock().await.renew_session(session, now)
    }
    pub async fn publisher_playlist_version(&self) -> Result<u32> {
        let bytes = tokio::fs::read(
            self.root
                .join("publisher/mp_playlists_shipRetail_release.info"),
        )
        .await?;
        ensure!(
            bytes.len() <= task::MAX_FILE_BYTES,
            "publisher playlist exceeds budget"
        );
        for line in bytes.split(|&b| b == b'\n') {
            if let Some(version) = line.strip_prefix(b"version ") {
                let value = std::str::from_utf8(version)?.trim().parse::<u32>()?;
                ensure!(value > 0, "invalid publisher playlist version");
                return Ok(value);
            }
        }
        anyhow::bail!("publisher playlist version is missing")
    }
    pub async fn with_game_endpoint(
        self,
        endpoint: std::sync::Arc<crate::game_server::GameAdmissions>,
    ) -> Self {
        *self.rooms.lock().await = crate::rooms::Rooms::with_game_endpoint(endpoint);
        self
    }
    pub async fn open(root: PathBuf) -> Result<Self> {
        create_directory(&root).await?;
        create_directory(&root.join("publisher")).await?;
        create_directory(&root.join("users")).await?;
        create_directory(&root.join("profiles")).await?;
        Ok(Self {
            root: tokio::fs::canonicalize(root).await?,
            writes: Mutex::new(()),
            rooms: Mutex::new(crate::rooms::Rooms::default()),
        })
    }

    pub async fn handle(&self, body: &[u8], session: &Session, now: u32) -> Result<TaskResult> {
        let request = Request::parse(body)?;
        match (request.service, request.task) {
            (11 | 21, _) | (6, 14) => self.rooms.lock().await.handle(&request, session, now),
            (71, 2) => {
                let mut reader = task::Reader::new(request.arguments);
                let name = reader.string(1024)?;
                reader.finish()?;
                if name.is_empty() || name.chars().any(char::is_control) {
                    return Ok(TaskResult::failure("invalid_local_text", task::TASK_FAILED));
                }
                // Target 0xa52180 reads Bool + String. Callback 0x6aa540 names
                // the Boolean "sanitized", so unchanged local text uses false.
                let mut record = Writer::default();
                record.boolean(false);
                record.string(name);
                Ok(TaskResult::success("local_text_checked", vec![record.0]))
            }
            (71, 4) => {
                lobby::registration_build(body)?;
                Ok(TaskResult::success("registration", vec![]))
            }
            (12, 6) => {
                task::Reader::new(request.arguments).finish()?;
                let mut record = Writer::default();
                record.u32(now);
                Ok(TaskResult::success("server_time", vec![record.0]))
            }
            (10, 10 | 12 | 21) => self.storage(task::storage(&request)?, session, now).await,
            (10, 20) => self.list_publisher(request.arguments).await,
            (80, 70 | 107) => {
                let mut reader = task::Reader::new(request.arguments);
                let context = reader.string(16)?;
                let page = reader.u32()?;
                let page_size = reader.u32()?;
                if request.task == 70 {
                    let _show_all = reader.boolean()?;
                    let sku_filters = reader.u32()?;
                    if sku_filters != 0 {
                        return Ok(TaskResult::failure(
                            "unsupported_catalogue_filter",
                            task::TASK_FAILED,
                        ));
                    }
                    let type_filters = reader.u32()?;
                    if type_filters != 0 {
                        return Ok(TaskResult::failure(
                            "unsupported_catalogue_filter",
                            task::TASK_FAILED,
                        ));
                    }
                }
                let _client_version = reader.string(32)?;
                reader.finish()?;
                if !context.is_empty() || page != 1 || !(1..=2000).contains(&page_size) {
                    return Ok(TaskResult::failure(
                        "unsupported_catalogue_query",
                        task::TASK_FAILED,
                    ));
                }
                // VA 0x4a6150 stops pagination when resultCount < pageSize,
                // including an empty first page. The lab has no saleable SKUs;
                // owned inventory and starter grants remain separate work.
                Ok(TaskResult::success(
                    if request.task == 70 {
                        "local_empty_catalogue"
                    } else {
                        "local_empty_catalogue_links"
                    },
                    vec![],
                ))
            }
            (80, 230) => {
                let mut reader = task::Reader::new(request.arguments);
                let context = reader.string(16)?;
                reader.finish()?;
                if !context.is_empty() {
                    return Ok(TaskResult::failure(
                        "unsupported_marketplace_namespace",
                        task::TASK_FAILED,
                    ));
                }
                let mut items = self.inventory(session).await?;
                let wallet = self.wallet(session).await?;
                if let Some(w) = &wallet {
                    for grant in &w.purchases {
                        if !items.contains(&grant.item_id) {
                            items.push(grant.item_id);
                        }
                    }
                }
                let balances = wallet
                    .as_ref()
                    // VA 0x4a5680 copies ONLY entry 0 to the marketplace wallet.
                    // It must be GP; the separate 93/1 result supplies CP.
                    .map(|w| vec![(Payment::Gp.wire_currency(), w.gp)])
                    .unwrap_or_default();
                Ok(TaskResult::success(
                    if wallet.is_some() {
                        "local_player_wallet_inventory"
                    } else if items.is_empty() {
                        "local_empty_player_data"
                    } else {
                        "local_player_inventory"
                    },
                    vec![task::player_data_with_balances(
                        session.user_id,
                        &items,
                        &balances,
                    )?],
                ))
            }
            (80, 173) => {
                let mut reader = task::Reader::new(request.arguments);
                let context = reader.string(16)?;
                let maximum = reader.u16()?;
                reader.finish()?;
                if !context.is_empty() || !(1..=700).contains(&maximum) {
                    return Ok(TaskResult::failure(
                        "unsupported_coupon_query",
                        task::TASK_FAILED,
                    ));
                }
                // VA 0xa46fb0 binds an array; VA 0x4a3470 consumes its count.
                // This local account has no issued coupons/mail attachments.
                Ok(TaskResult::success("empty_coupon_inventory", vec![]))
            }
            (93, 1) | (80, 130) => {
                let mut reader = task::Reader::new(request.arguments);
                let maximum = reader.u32()?;
                reader.finish()?;
                if maximum != 1 {
                    return Ok(TaskResult::failure(
                        "unsupported_balance_query",
                        task::TASK_FAILED,
                    ));
                }
                // VA 0xa4c4a0 reads UChar + UInt32. Payment route and UI wire
                // currency are distinct; the target UI uses GP=1 and CP=2.
                let cp = request.service == 93;
                let amount = self
                    .wallet(session)
                    .await?
                    .map_or(0, |w| if cp { w.cp } else { w.gp });
                let mut record = Writer::default();
                record.u8(if cp { Payment::Cp } else { Payment::Gp }.wire_currency());
                record.u32(amount);
                Ok(TaskResult::success(
                    if cp {
                        "local_cp_balance"
                    } else {
                        "local_gp_balance"
                    },
                    vec![record.0],
                ))
            }
            (80, 203) => {
                let mut r = task::Reader::new(request.arguments);
                let context = r.string(16)?;
                let maximum = r.u16()?;
                r.finish()?;
                if !context.is_empty() || !(1..=20000).contains(&maximum) {
                    return Ok(TaskResult::failure(
                        "unsupported_inventory_query",
                        task::TASK_FAILED,
                    ));
                }
                let mut items = self.inventory(session).await?;
                if let Some(wallet) = self.wallet(session).await? {
                    for p in wallet.purchases {
                        if !items.contains(&p.item_id) {
                            items.push(p.item_id);
                        }
                    }
                }
                let mut records = task::inventory_records(session.user_id, &items)?;
                records.truncate(usize::from(maximum));
                Ok(TaskResult::success("local_inventory_refresh", records))
            }
            (80, 154) | (93, 131) => {
                let sku = codol_protocol::purchase::single_sku(request.service, request.arguments)?;
                let _lock = self.writes.lock().await;
                let Some(bytes) =
                    read_bounded(&self.root.join("shop.json"), 4 * 1024 * 1024).await?
                else {
                    return Ok(TaskResult::failure("local_shop_missing", task::TASK_FAILED));
                };
                let shop: LocalShop = serde_json::from_slice(&bytes)?;
                shop.validate(session.title_id)?;
                let Some(mut wallet) = self.wallet(session).await? else {
                    return Ok(TaskResult::failure(
                        "local_wallet_missing",
                        task::TASK_FAILED,
                    ));
                };
                let inventory = self.inventory(session).await?;
                let payment = if request.service == 93 {
                    Payment::Cp
                } else {
                    Payment::Gp
                };
                if let Err(reason) = wallet.purchase(&shop, &inventory, sku, payment) {
                    tracing::info!(sku, ?payment, %reason, "local purchase rejected");
                    return Ok(TaskResult::failure(
                        "local_purchase_rejected",
                        task::TASK_FAILED,
                    ));
                }
                // Debit and grant commit together. A write failure never acknowledges success.
                let path = self
                    .profile_directory(session.title_id, session.user_id)
                    .await?
                    .join("wallet.json");
                atomic_private_write(&path, &serde_json::to_vec_pretty(&wallet)?).await?;
                Ok(TaskResult::success(
                    "local_permanent_item_purchased",
                    vec![],
                ))
            }
            (8, 1 | 2) => {
                let mut reader = task::Reader::new(request.arguments);
                if request.task == 1 && reader.u64()? != session.user_id {
                    return Ok(TaskResult::failure(
                        "profile_owner_mismatch",
                        task::TASK_FAILED,
                    ));
                }
                reader.finish()?;
                let private = request.task == 2;
                let path = self.profile_path(session, private).await?;
                if let Some(data) = read_bounded(&path, task::MAX_FILE_BYTES).await? {
                    let record = if private {
                        profile::private_record(session.user_id, &data)?
                    } else {
                        profile::public_record(session.user_id, &data)?
                    };
                    return Ok(TaskResult::success(
                        if private {
                            "private_profile"
                        } else {
                            "public_profile"
                        },
                        vec![record],
                    ));
                }
                // Private profile RPC writes are not supported. A precise
                // missing result lets the target initialize its own defaults.
                Ok(TaskResult::failure("profile_missing", task::NO_PROFILE))
            }
            (8, 3) => {
                profile::validate_public(request.arguments)?;
                let _lock = self.writes.lock().await;
                let path = self.profile_path(session, false).await?;
                atomic_private_write(&path, request.arguments).await?;
                Ok(TaskResult::success("public_profile_saved", vec![]))
            }
            _ => Ok(TaskResult::failure("unsupported_task", task::TASK_FAILED)),
        }
    }

    async fn profile_path(&self, session: &Session, private: bool) -> Result<PathBuf> {
        let user = self
            .profile_directory(session.title_id, session.user_id)
            .await?;
        let path = user.join(if private { "private.bin" } else { "public.bin" });
        if let Ok(metadata) = tokio::fs::symlink_metadata(&path).await {
            ensure!(
                metadata.is_file() && !metadata.is_symlink(),
                "profile path must be a regular file"
            );
        }
        Ok(path)
    }

    async fn profile_directory(&self, title_id: u32, user_id: u64) -> Result<PathBuf> {
        let title = self.root.join("profiles").join(title_id.to_string());
        create_directory(&title).await?;
        let user = title.join(user_id.to_string());
        create_directory(&user).await?;
        Ok(user)
    }

    async fn wallet(&self, session: &Session) -> Result<Option<LocalWallet>> {
        let path = self
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

    async fn inventory(&self, session: &Session) -> Result<Vec<u32>> {
        let path = self
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

    /// Freeze the latest saved backpack and ownership for one admitted player.
    /// Uses the same write lock and grant sources as lobby saves/purchases.
    pub(crate) async fn game_profile(&self, title_id: u32, user_id: u64) -> Result<GameProfile> {
        let _lock = self.writes.lock().await;
        let session = Session {
            title_id,
            user_id,
            iv_seed: 0,
            expires_at: 0,
            key: [0; 24],
        };
        let path = self.profile_path(&session, false).await?;
        let public = read_bounded(&path, task::MAX_FILE_BYTES)
            .await?
            .context("saved player loadout is missing")?;
        profile::validate_public(&public)?;
        let rank = match read_bounded(
            &self.profile_path(&session, true).await?,
            task::MAX_FILE_BYTES,
        )
        .await?
        {
            Some(private) => profile::player_rank(&private, user_id)?,
            None => (0, 0),
        };
        let mut owned = self.inventory(&session).await?;
        if let Some(wallet) = self.wallet(&session).await? {
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

    /// Offline administration of one existing lab account. Stop all services
    /// first: this instance's mutex cannot coordinate another server process.
    /// Each replacement is atomic; the two-file update is recoverable from the
    /// returned backup directory, not a cross-file transaction.
    pub async fn boost_test_account(
        &self,
        title_id: u32,
        user_id: u64,
        cp: u32,
        gp: u32,
    ) -> Result<PathBuf> {
        let _lock = self.writes.lock().await;
        let directory = self.profile_directory(title_id, user_id).await?;
        let private_path = directory.join("private.bin");
        let old_private = read_bounded(&private_path, task::MAX_FILE_BYTES)
            .await?
            .context("existing private profile required; initialize the account first")?;
        let new_private = profile::maximize_test_rank(&old_private, user_id)?;
        let wallet_path = directory.join("wallet.json");
        let old_wallet = read_bounded(&wallet_path, 32768).await?;
        let purchases = if let Some(bytes) = &old_wallet {
            let wallet = serde_json::from_slice::<LocalWallet>(bytes)?;
            wallet.validate(title_id, user_id)?;
            wallet.purchases
        } else {
            Vec::new()
        };
        let wallet = LocalWallet {
            schema_version: 1,
            client_sha256: codol_protocol::CLIENT_SHA256.to_owned(),
            title_id,
            user_id: user_id.to_string(),
            cp,
            gp,
            purchases,
        };
        let new_wallet = serde_json::to_vec_pretty(&wallet)?;
        let mut nonce = [0u8; 8];
        openssl::rand::rand_bytes(&mut nonce)?;
        let backup = directory.join(format!(
            "backup-test-account-{:016x}",
            u64::from_le_bytes(nonce)
        ));
        tokio::fs::create_dir(&backup).await?;
        atomic_private_write(&backup.join("private.bin"), &old_private).await?;
        if let Some(bytes) = &old_wallet {
            atomic_private_write(&backup.join("wallet.json"), bytes).await?;
        }
        atomic_private_write(
            &backup.join("manifest.json"),
            &serde_json::to_vec_pretty(&serde_json::json!({
                "title_id": title_id, "user_id": user_id.to_string(),
                "wallet_existed": old_wallet.is_some(), "level": 70, "cp": cp, "gp": gp,
            }))?,
        )
        .await?;
        atomic_private_write(&private_path, &new_private)
            .await
            .with_context(|| format!("private update failed; backup: {}", backup.display()))?;
        atomic_private_write(&wallet_path, &new_wallet)
            .await
            .with_context(|| {
                format!(
                    "wallet update failed; restore both files from {}",
                    backup.display()
                )
            })?;
        Ok(backup)
    }

    async fn list_publisher(&self, arguments: &[u8]) -> Result<TaskResult> {
        let mut reader = task::Reader::new(arguments);
        let context = reader.string(16)?;
        let filter = reader.u32()?;
        let limit = reader.u16()?;
        let offset = reader.u16()?;
        reader.finish()?;
        // Only the observed unfiltered query is supported; filter/offset
        // semantics need more evidence before implementing pagination.
        if !context.is_empty() || filter != 0 || offset != 0 || limit != 32 {
            return Ok(TaskResult::failure(
                "unsupported_publisher_query",
                task::TASK_FAILED,
            ));
        }
        let mut entries = tokio::fs::read_dir(self.root.join("publisher")).await?;
        let mut names = Vec::new();
        let mut scanned = 0;
        while let Some(entry) = entries.next_entry().await? {
            scanned += 1;
            ensure!(scanned <= 128, "publisher directory exceeds local limit");
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("non-UTF8 publisher name"))?;
            if valid_name(&name) && (name.ends_with(".info") || name.ends_with(".ff")) {
                names.push(name);
            }
        }
        names.sort();
        ensure!(
            names.len() <= limit as usize,
            "publisher pagination is not implemented"
        );
        let mut records = Vec::new();
        for name in names.into_iter().take(limit as usize) {
            let path = self.root.join("publisher").join(&name);
            let Some(data) = read_bounded(&path, task::MAX_FILE_BYTES).await? else {
                continue;
            };
            let metadata = tokio::fs::metadata(&path).await?;
            let modified: u32 = metadata
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs()
                .try_into()?;
            ensure!(
                modified != 0 && modified != u32::MAX,
                "invalid publisher modification time"
            );
            let digest = Sha256::digest(name.as_bytes());
            let file_id = u64::from_le_bytes(digest[..8].try_into()?).max(1);
            // VA 0xa5b230: this result type differs from upload FileInfo.
            let mut record = Writer::default();
            record.u32(data.len() as u32);
            record.u64(file_id);
            record.u32(modified);
            record.u32(modified);
            record.boolean(true);
            record.u64(0);
            record.string(&name);
            record.string(""); // Required String(cap=50), locally unset; semantics still unknown.
            records.push(record.0);
        }
        Ok(TaskResult::success("publisher_file_list", records))
    }

    async fn storage(
        &self,
        request: StorageRequest<'_>,
        session: &Session,
        now: u32,
    ) -> Result<TaskResult> {
        let (context, name) = match &request {
            StorageRequest::Read { context, name, .. }
            | StorageRequest::Publisher { context, name }
            | StorageRequest::Upload { context, name, .. } => (*context, *name),
        };
        if !context.is_empty() || !valid_name(name) {
            return Ok(TaskResult::failure(
                "unsupported_storage_namespace",
                task::TASK_FAILED,
            ));
        }
        match request {
            StorageRequest::Publisher { .. } => {
                // Only operator-provided files, never generated empty config files.
                let path = self.root.join("publisher").join(name);
                match read_bounded(&path, task::MAX_FILE_BYTES).await? {
                    None => Ok(TaskResult::failure("publisher_file_missing", task::NO_FILE)),
                    Some(data) => blob_result("publisher_file", &data),
                }
            }
            StorageRequest::Read {
                owner,
                account_type,
                ..
            } => {
                if owner != session.user_id || !account_type.is_empty() {
                    return Ok(TaskResult::failure(
                        "storage_owner_mismatch",
                        task::TASK_FAILED,
                    ));
                }
                let path = self.user_path(session, name).await?;
                match read_stored(&path).await? {
                    None => Ok(TaskResult::failure("user_file_missing", task::NO_FILE)),
                    Some(record) => blob_result("user_file", &STANDARD.decode(record.data_base64)?),
                }
            }
            StorageRequest::Upload {
                visibility,
                data,
                owner,
                account_type,
                version,
                ..
            } => {
                if (owner != 0 && owner != session.user_id)
                    || !account_type.is_empty()
                    || version != 0
                {
                    return Ok(TaskResult::failure(
                        "unsupported_upload_parameters",
                        task::TASK_FAILED,
                    ));
                }
                let _lock = self.writes.lock().await;
                let path = self.user_path(session, name).await?;
                let previous = read_stored(&path).await?;
                if previous.is_none() {
                    let mut entries =
                        tokio::fs::read_dir(path.parent().context("user folder")?).await?;
                    let mut count = 0;
                    while entries.next_entry().await?.is_some() {
                        count += 1;
                    }
                    ensure!(count < 128, "local user file quota exceeded");
                }
                let record = StoredFile {
                    schema: 1,
                    created: previous.map_or(now, |p| p.created),
                    modified: now,
                    visibility,
                    data_base64: STANDARD.encode(data),
                };
                let bytes = serde_json::to_vec(&record)?;
                atomic_private_write(&path, &bytes).await?;
                // Target bdFileInfo::deserialize VA 0xa4eab0. No fileID field.
                let mut info = Writer::default();
                info.u32(data.len() as u32);
                info.u32(record.created);
                info.u32(now);
                info.boolean(visibility);
                info.u64(session.user_id);
                info.string("");
                info.string(name);
                Ok(TaskResult::success("user_file_uploaded", vec![info.0]))
            }
        }
    }

    async fn user_path(&self, session: &Session, name: &str) -> Result<PathBuf> {
        let title = self.root.join("users").join(session.title_id.to_string());
        create_directory(&title).await?;
        let user = title.join(session.user_id.to_string());
        create_directory(&user).await?;
        Ok(user.join(format!("{name}.json")))
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() < 128
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}
async fn atomic_private_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut random = [0; 8];
    openssl::rand::rand_bytes(&mut random)?;
    let temporary = path.with_extension(format!("{:016x}.tmp", u64::from_le_bytes(random)));
    let result = async {
        write_private(&temporary, bytes).await?;
        tokio::fs::File::open(&temporary).await?.sync_all().await?;
        tokio::fs::rename(&temporary, path).await?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(temporary).await;
    }
    result
}
async fn create_directory(path: &Path) -> Result<()> {
    tokio::fs::create_dir_all(path).await?;
    let metadata = tokio::fs::symlink_metadata(path).await?;
    ensure!(
        metadata.is_dir() && !metadata.is_symlink(),
        "local data directory must be a real directory"
    );
    Ok(())
}
async fn read_bounded(path: &Path, limit: usize) -> Result<Option<Vec<u8>>> {
    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(m) => m,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    ensure!(
        metadata.is_file() && !metadata.is_symlink(),
        "local file must be a regular file"
    );
    ensure!(
        metadata.len() <= limit as u64,
        "local file exceeds byte limit"
    );
    let mut bytes = Vec::new();
    tokio::fs::File::open(path)
        .await?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .await?;
    ensure!(bytes.len() <= limit, "local file grew beyond byte limit");
    Ok(Some(bytes))
}
async fn read_stored(path: &Path) -> Result<Option<StoredFile>> {
    let Some(bytes) = read_bounded(path, task::MAX_FILE_BYTES * 2).await? else {
        return Ok(None);
    };
    let record: StoredFile = serde_json::from_slice(&bytes)?;
    ensure!(record.schema == 1, "unsupported local storage schema");
    ensure!(
        STANDARD.decode(&record.data_base64)?.len() <= task::MAX_FILE_BYTES,
        "stored blob exceeds byte limit"
    );
    Ok(Some(record))
}
fn blob_result(label: &'static str, data: &[u8]) -> Result<TaskResult> {
    let mut record = Writer::default();
    record.blob(data)?;
    Ok(TaskResult::success(label, vec![record.0]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn game_profile_reads_latest_save_and_combines_starter_and_purchase_ownership() {
        let temp = tempfile::tempdir().unwrap();
        let services = LocalServices::open(temp.path().join("data")).await.unwrap();
        let s = session(1);
        let directory = services
            .profile_directory(s.title_id, s.user_id)
            .await
            .unwrap();
        let inventory = serde_json::json!({"schema_version":1,"client_sha256":codol_protocol::CLIENT_SHA256,
            "title_id":s.title_id,"user_id":s.user_id.to_string(),"items":[10001]});
        atomic_private_write(
            &directory.join("inventory.json"),
            &serde_json::to_vec(&inventory).unwrap(),
        )
        .await
        .unwrap();
        let wallet = LocalWallet {
            schema_version: 1,
            client_sha256: codol_protocol::CLIENT_SHA256.into(),
            title_id: s.title_id,
            user_id: s.user_id.to_string(),
            cp: 0,
            gp: 0,
            purchases: vec![crate::marketplace::Grant {
                sku_id: 14629,
                item_id: 20659,
                payment: Payment::Cp,
                price: 280,
            }],
        };
        atomic_private_write(
            &directory.join("wallet.json"),
            &serde_json::to_vec(&wallet).unwrap(),
        )
        .await
        .unwrap();
        for item in [0u32, 20659] {
            let mut plain = vec![0u8; 32768];
            plain[1000..1004].copy_from_slice(&item.to_le_bytes());
            let crc = crc32fast::hash(&plain[4..]);
            plain[..4].copy_from_slice(&crc.to_le_bytes());
            let mut public = Writer::default();
            public.u32(0);
            public.u32(1);
            public.u32(crc);
            public.blob(&plain).unwrap();
            for _ in 0..125 {
                public.i32(0);
            }
            public.0.push(0);
            let mut upload = vec![8, 3, 3];
            upload.extend_from_slice(&public.0);
            services
                .handle(&upload, &s, 1)
                .await
                .unwrap()
                .result
                .unwrap();
            let GameProfile {
                public: saved,
                owned,
                rank,
            } = services.game_profile(s.title_id, s.user_id).await.unwrap();
            assert_eq!(rank, (0, 0));
            assert_eq!(saved, public.0);
            assert_eq!(owned, [10001, 20659]);
            assert!(services.game_profile(s.title_id, 2).await.is_err());
            assert!(
                services
                    .game_profile(s.title_id + 1, s.user_id)
                    .await
                    .is_err()
            );
        }
    }
    #[tokio::test]
    async fn room_name_check_preserves_utf8_and_reports_unchanged_text() {
        let temp = tempfile::tempdir().unwrap();
        let server = LocalServices::open(temp.path().join("data")).await.unwrap();
        let s = session(1);
        let mut query = Writer(vec![71, 3, 2]);
        query.string("本地房间");
        query.0.push(0);
        let response = server
            .handle(&query.0, &s, 1)
            .await
            .unwrap()
            .result
            .unwrap();
        assert_eq!(&response[0][..3], &[1, 0, 16]);
        assert_eq!(&response[0][3..], "本地房间\0".as_bytes());
        for n in 0..query.0.len() {
            assert!(server.handle(&query.0[..n], &s, 1).await.is_err());
        }
        for text in ["", "room\nname"] {
            let mut q = Writer(vec![71, 3, 2]);
            q.string(text);
            q.0.push(0);
            assert_eq!(
                server.handle(&q.0, &s, 1).await.unwrap().result,
                Err(task::TASK_FAILED)
            );
        }
    }
    fn buy(service: u8, sku: u32) -> Vec<u8> {
        let mut w = Writer(vec![service, 3, if service == 80 { 154 } else { 131 }]);
        w.string("");
        w.u32(1);
        w.u32(sku);
        w.u32(1);
        w.u32(1);
        w.boolean(false);
        w.u32(1);
        w.u64(0);
        w.string("");
        w.u32(0);
        w.u32(0);
        w.u8(0);
        w.u32(u32::from(service == 80));
        w.0.push(0);
        w.0
    }
    #[tokio::test]
    async fn purchases_commit_debit_and_item_together_and_reject_duplicates_after_restart() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("data");
        let server = LocalServices::open(root.clone()).await.unwrap();
        let s = session(0x89abcdef01234567);
        let wallet_path = server
            .profile_directory(s.title_id, s.user_id)
            .await
            .unwrap()
            .join("wallet.json");
        let initial = serde_json::json!({"schema_version":1,"client_sha256":codol_protocol::CLIENT_SHA256,
            "title_id":s.title_id,"user_id":s.user_id.to_string(),"cp":1000,"gp":3000});
        atomic_private_write(&wallet_path, &serde_json::to_vec(&initial).unwrap())
            .await
            .unwrap();
        let shop = serde_json::json!({"schema_version":1,"client_sha256":codol_protocol::CLIENT_SHA256,
            "title_id":s.title_id,"source_sha256":"a".repeat(64),"offers":[
            {"sku_id":10073,"item_id":10183,"currency":2,"price":2600},
            {"sku_id":14629,"item_id":20659,"currency":1,"price":280},
            {"sku_id":15,"item_id":10183,"currency":1,"price":100},
            {"sku_id":16,"item_id":5,"currency":2,"price":9999}]});
        atomic_private_write(&root.join("shop.json"), &serde_json::to_vec(&shop).unwrap())
            .await
            .unwrap();
        let request = buy(80, 10073);
        let (a, b) = tokio::join!(
            server.handle(&request, &s, 1),
            server.handle(&request, &s, 1)
        );
        assert_eq!(
            usize::from(a.unwrap().result.is_ok()) + usize::from(b.unwrap().result.is_ok()),
            1
        );
        assert!(
            server
                .handle(&buy(93, 14629), &s, 2)
                .await
                .unwrap()
                .result
                .unwrap()
                .is_empty()
        );
        drop(server);
        let server = LocalServices::open(root).await.unwrap();
        let wallet = server.wallet(&s).await.unwrap().unwrap();
        assert_eq!((wallet.cp, wallet.gp), (720, 400));
        assert_eq!(
            wallet
                .purchases
                .iter()
                .map(|p| p.item_id)
                .collect::<Vec<_>>(),
            [10183, 20659]
        );
        let records = server
            .handle(&[80, 3, 230, 16, 0, 0], &s, 3)
            .await
            .unwrap()
            .result
            .unwrap();
        assert_eq!(
            records,
            [task::player_data_with_balances(s.user_id, &[10183, 20659], &[(1, 400)]).unwrap()]
        );
        let refresh = [80, 3, 203, 16, 0, 6, 32, 78, 0];
        let refreshed = server
            .handle(&refresh, &s, 3)
            .await
            .unwrap()
            .result
            .unwrap();
        assert_eq!(refreshed.len(), 2);
        for (record, id) in refreshed.iter().zip([10183, 20659]) {
            let mut r = task::Reader::new(record);
            assert_eq!(r.u64().unwrap(), s.user_id);
            assert_eq!(r.string(16).unwrap(), "");
            assert_eq!(r.u32().unwrap(), id);
            assert_eq!(r.u32().unwrap(), 1);
        }
        assert_eq!(
            server
                .handle(&[80, 3, 130, 8, 1, 0, 0, 0, 0], &s, 3)
                .await
                .unwrap()
                .result,
            Ok(vec![vec![3, 1, 8, 144, 1, 0, 0]])
        );
        assert_eq!(
            server
                .handle(&refresh, &session(2), 3)
                .await
                .unwrap()
                .result,
            Ok(vec![])
        );
        for n in 0..refresh.len() {
            assert!(server.handle(&refresh[..n], &s, 3).await.is_err());
        }
        let before = tokio::fs::read(&wallet_path).await.unwrap();
        // Replays, alternative SKU for owned item, wrong currency, unknown SKU, overdraft.
        for request in [
            buy(80, 10073),
            buy(93, 14629),
            buy(93, 15),
            buy(93, 10073),
            buy(80, 999),
            buy(80, 16),
        ] {
            assert_eq!(
                server.handle(&request, &s, 4).await.unwrap().result,
                Err(task::TASK_FAILED)
            );
            assert_eq!(tokio::fs::read(&wallet_path).await.unwrap(), before);
        }
        assert_eq!(
            server
                .handle(&buy(80, 10073), &session(2), 4)
                .await
                .unwrap()
                .result,
            Err(task::TASK_FAILED)
        );
        assert!(server.wallet(&session(2)).await.unwrap().is_none());
    }
    #[tokio::test]
    async fn test_account_boost_backs_up_progress_and_persists_consistent_isolated_balances() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("data");
        let server = LocalServices::open(root.clone()).await.unwrap();
        let s = session(0x89abcdef01234567);
        let mut blank = vec![0; profile::SNAPSHOT_BYTES];
        // CRC32 of a 32764-byte zero default, computed independently.
        blank[..4].copy_from_slice(&0xb52d2b04u32.to_le_bytes());
        let seeded = profile::prepare_private(&blank, s.user_id, true).unwrap();
        let private_path = server.profile_path(&s, true).await.unwrap();
        atomic_private_write(&private_path, &seeded).await.unwrap();
        let public_path = server.profile_path(&s, false).await.unwrap();
        atomic_private_write(&public_path, b"preserve public")
            .await
            .unwrap();
        let inventory_path = private_path.with_file_name("inventory.json");
        let inventory = serde_json::to_vec(&serde_json::json!({
            "schema_version":1,"client_sha256":codol_protocol::CLIENT_SHA256,
            "title_id":s.title_id,"user_id":s.user_id.to_string(),"items":[792]
        }))
        .unwrap();
        atomic_private_write(&inventory_path, &inventory)
            .await
            .unwrap();
        let backup = server
            .boost_test_account(s.title_id, s.user_id, 100_000, 1_000_000)
            .await
            .unwrap();
        assert_eq!(
            tokio::fs::read(backup.join("private.bin")).await.unwrap(),
            seeded
        );
        assert!(!backup.join("wallet.json").exists());
        assert_eq!(
            tokio::fs::read(&public_path).await.unwrap(),
            b"preserve public"
        );
        assert_eq!(tokio::fs::read(&inventory_path).await.unwrap(), inventory);
        drop(server);
        let server = LocalServices::open(root).await.unwrap();
        let player = server
            .handle(&[80, 3, 230, 16, 0, 0], &s, 1)
            .await
            .unwrap()
            .result
            .unwrap();
        assert_eq!(
            &player[0][11..28],
            &[8, 1, 0, 0, 0, 3, 1, 8, 64, 66, 15, 0, 8, 1, 0, 0, 0]
        );
        let external = [93, 3, 1, 8, 1, 0, 0, 0, 0];
        assert_eq!(
            server.handle(&external, &s, 1).await.unwrap().result,
            Ok(vec![vec![3, 2, 8, 160, 134, 1, 0]])
        );
        let mut other = s.clone();
        for change_title in [true, false] {
            if change_title {
                other.title_id = 999;
            } else {
                other = session(2);
            }
            assert_eq!(
                server.handle(&external, &other, 1).await.unwrap().result,
                Ok(vec![vec![3, 2, 8, 0, 0, 0, 0]])
            );
            let misplaced = server
                .profile_directory(other.title_id, other.user_id)
                .await
                .unwrap()
                .join("wallet.json");
            tokio::fs::copy(private_path.with_file_name("wallet.json"), misplaced)
                .await
                .unwrap();
            assert!(server.handle(&external, &other, 1).await.is_err());
        }
        let backup2 = server
            .boost_test_account(s.title_id, s.user_id, 7, 8)
            .await
            .unwrap();
        let previous: LocalWallet =
            serde_json::from_slice(&tokio::fs::read(backup2.join("wallet.json")).await.unwrap())
                .unwrap();
        assert_eq!((previous.cp, previous.gp), (100_000, 1_000_000));
        let before = tokio::fs::read(&private_path).await.unwrap();
        tokio::fs::write(private_path.with_file_name("wallet.json"), b"{}")
            .await
            .unwrap();
        assert!(
            server
                .boost_test_account(s.title_id, s.user_id, 1, 1)
                .await
                .is_err()
        );
        assert_eq!(tokio::fs::read(private_path).await.unwrap(), before);
    }
    #[tokio::test]
    async fn initial_coupon_and_balance_queries_have_distinct_verified_result_shapes() {
        let temp = tempfile::tempdir().unwrap();
        let server = LocalServices::open(temp.path().join("data")).await.unwrap();
        let s = session(1);
        let coupons = [80, 3, 173, 16, 0, 6, 188, 2, 0];
        let balance = [93, 3, 1, 8, 1, 0, 0, 0, 0];
        assert_eq!(
            server.handle(&coupons, &s, 1).await.unwrap().result,
            Ok(vec![])
        );
        assert_eq!(
            server.handle(&balance, &s, 1).await.unwrap().result,
            Ok(vec![vec![3, 2, 8, 0, 0, 0, 0]])
        );
        for body in [&coupons[..], &balance[..]] {
            for size in 0..body.len() {
                assert!(server.handle(&body[..size], &s, 1).await.is_err());
            }
            let mut extra = body.to_vec();
            extra.push(0);
            assert!(server.handle(&extra, &s, 1).await.is_err());
        }
        let mut zero = coupons;
        zero[6] = 0;
        zero[7] = 0;
        assert_eq!(
            server.handle(&zero, &s, 1).await.unwrap().result,
            Err(task::TASK_FAILED)
        );
        let mut other = balance;
        other[4] = 2;
        assert_eq!(
            server.handle(&other, &s, 1).await.unwrap().result,
            Err(task::TASK_FAILED)
        );
        assert_eq!(
            server.handle(&[80, 3, 172, 0], &s, 1).await.unwrap().result,
            Err(task::TASK_FAILED)
        );
    }
    fn session(uid: u64) -> Session {
        Session {
            title_id: 1002,
            user_id: uid,
            iv_seed: 1,
            expires_at: u32::MAX,
            key: [0; 24],
        }
    }
    fn read(name: &str, uid: u64) -> Vec<u8> {
        let mut w = Writer(vec![10, 3, 12]);
        w.string("");
        w.string(name);
        w.u64(uid);
        w.string("");
        w.0.push(0);
        w.0
    }
    fn upload(name: &str, data: &[u8]) -> Vec<u8> {
        let mut w = Writer(vec![10, 3, 10]);
        w.string("");
        w.string(name);
        w.boolean(true);
        w.blob(data).unwrap();
        w.u64(0);
        w.string("");
        w.u32(0);
        w.0.push(0);
        w.0
    }
    #[tokio::test]
    async fn upload_survives_restart_preserves_exact_bytes_and_isolates_identity() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("data");
        let s = session(u64::MAX);
        let server = LocalServices::open(root.clone()).await.unwrap();
        let request = read("the.json", s.user_id);
        assert_eq!(
            server.handle(&request, &s, 10).await.unwrap().result,
            Err(task::NO_FILE)
        );
        let data = b"{\"blacklist\":[],\"Recentmetlist\":[]}\0";
        let stored = server
            .handle(&upload("the.json", data), &s, 11)
            .await
            .unwrap()
            .result
            .unwrap();
        let mut expected = Writer::default();
        expected.u32(data.len() as u32);
        expected.u32(11);
        expected.u32(11);
        expected.boolean(true);
        expected.u64(s.user_id);
        expected.string("");
        expected.string("the.json");
        assert_eq!(stored, [expected.0]);
        drop(server);
        let server = LocalServices::open(root).await.unwrap();
        let mut blob = Writer::default();
        blob.blob(data).unwrap();
        assert_eq!(
            server
                .handle(&request, &s, 12)
                .await
                .unwrap()
                .result
                .unwrap(),
            [blob.0]
        );
        assert_eq!(
            server
                .handle(&request, &session(1), 12)
                .await
                .unwrap()
                .result,
            Err(task::TASK_FAILED)
        );
        assert_eq!(
            server
                .handle(&read("the.json", 1), &session(1), 12)
                .await
                .unwrap()
                .result,
            Err(task::NO_FILE)
        );
        let mut different_title = s.clone();
        different_title.title_id = 1;
        assert_eq!(
            server
                .handle(&request, &different_title, 12)
                .await
                .unwrap()
                .result,
            Err(task::NO_FILE)
        );
        let updated = server
            .handle(&upload("the.json", b"new"), &s, 20)
            .await
            .unwrap()
            .result
            .unwrap();
        assert_eq!(&updated[0][5..15], &[8, 11, 0, 0, 0, 8, 20, 0, 0, 0]);
    }
    #[tokio::test]
    async fn rejects_paths_corrupt_files_and_unsupported_tasks_without_success() {
        let temp = tempfile::tempdir().unwrap();
        let server = LocalServices::open(temp.path().join("data")).await.unwrap();
        let s = session(1);
        for name in ["../secret", "/etc/passwd", "a\\b", ".hidden", ""] {
            assert_eq!(
                server
                    .handle(&upload(name, b"x"), &s, 1)
                    .await
                    .unwrap()
                    .result,
                Err(task::TASK_FAILED)
            );
        }
        assert_eq!(
            server.handle(&[80, 3, 69, 0], &s, 1).await.unwrap().result,
            Err(task::TASK_FAILED)
        );
        let path = server.user_path(&s, "the.json").await.unwrap();
        tokio::fs::write(&path, b"broken").await.unwrap();
        assert!(
            server
                .handle(&upload("the.json", b"x"), &s, 1)
                .await
                .is_err()
        );
        assert_eq!(tokio::fs::read(path).await.unwrap(), b"broken");
        let mut request = Writer(vec![10, 3, 21]);
        request.string("");
        request.string("configs_s.info");
        request.0.push(0);
        assert_eq!(
            server.handle(&request.0, &s, 1).await.unwrap().result,
            Err(task::NO_FILE)
        );
        tokio::fs::write(
            server.root.join("publisher/configs_s.info"),
            vec![0; task::MAX_FILE_BYTES + 1],
        )
        .await
        .unwrap();
        assert!(server.handle(&request.0, &s, 1).await.is_err());
    }

    #[tokio::test]
    async fn publisher_listing_matches_served_bytes_and_has_stable_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let server = LocalServices::open(temp.path().join("data")).await.unwrap();
        let s = session(1);
        let encrypted = codol_protocol::publisher::crypt_configs(include_bytes!(
            "../../../configs/publisher/configs_s.ini"
        ))
        .unwrap();
        tokio::fs::write(server.root.join("publisher/configs_s.info"), &encrypted)
            .await
            .unwrap();
        let query = [10, 3, 20, 16, 0, 8, 0, 0, 0, 0, 6, 32, 0, 6, 0, 0, 0];
        let first = server.handle(&query, &s, 1).await.unwrap().result.unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(
            first,
            server
                .handle(&query, &s, 999)
                .await
                .unwrap()
                .result
                .unwrap()
        );
        // Read in the exact order of target deserializer VA 0xa5b230.
        let mut record = first[0].clone();
        record.push(0);
        let mut r = task::Reader::new(&record);
        assert_eq!(r.u32().unwrap(), encrypted.len() as u32);
        assert_ne!(r.u64().unwrap(), 0);
        let created = r.u32().unwrap();
        let modified = r.u32().unwrap();
        assert_eq!(created, modified);
        assert_ne!(modified, 0);
        assert!(r.boolean().unwrap());
        assert_eq!(r.u64().unwrap(), 0);
        assert_eq!(r.string(128).unwrap(), "configs_s.info");
        assert_eq!(r.string(50).unwrap(), "");
        r.finish().unwrap();
        let mut get = Writer(vec![10, 3, 21]);
        get.string("");
        get.string("configs_s.info");
        get.0.push(0);
        let records = server.handle(&get.0, &s, 1).await.unwrap().result.unwrap();
        let mut raw = records[0].clone();
        raw.push(0);
        let mut r = task::Reader::new(&raw);
        assert_eq!(r.blob().unwrap(), encrypted);
        r.finish().unwrap();
        let mut unsupported = query;
        unsupported[14] = 1;
        assert_eq!(
            server.handle(&unsupported, &s, 1).await.unwrap().result,
            Err(task::TASK_FAILED)
        );
        for n in 0..query.len() {
            assert!(server.handle(&query[..n], &s, 1).await.is_err());
        }
    }

    #[tokio::test]
    async fn only_authenticated_profile_lookup_gets_target_missing_profile_error() {
        let temp = tempfile::tempdir().unwrap();
        let server = LocalServices::open(temp.path().join("data")).await.unwrap();
        let s = session(u64::MAX);
        let mut public = Writer(vec![8, 3, 1]);
        public.u64(s.user_id);
        public.0.push(0);
        assert_eq!(
            server.handle(&public.0, &s, 1).await.unwrap().result,
            Err(task::NO_PROFILE)
        );
        assert_eq!(
            server.handle(&[8, 3, 2, 0], &s, 1).await.unwrap().result,
            Err(task::NO_PROFILE)
        );
        assert_eq!(
            server
                .handle(&public.0, &session(1), 1)
                .await
                .unwrap()
                .result,
            Err(task::TASK_FAILED)
        );
        assert!(server.handle(&[8, 3, 2, 0, 0], &s, 1).await.is_err());
    }

    #[tokio::test]
    async fn public_profile_persists_restarts_and_rejects_bad_crc_without_overwrite() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("data");
        let server = LocalServices::open(root.clone()).await.unwrap();
        let s = session(u64::MAX);
        let arguments = include_bytes!("../../../fixtures/protocol/public-profile-synthetic.bin");
        let mut upload = vec![8, 3, 3];
        upload.extend_from_slice(arguments);
        assert!(
            server
                .handle(&upload, &s, 1)
                .await
                .unwrap()
                .result
                .unwrap()
                .is_empty()
        );
        drop(server);
        let server = LocalServices::open(root).await.unwrap();
        let mut get = Writer(vec![8, 3, 1]);
        get.u64(s.user_id);
        get.0.push(0);
        let expected = profile::public_record(s.user_id, arguments).unwrap();
        assert_eq!(
            server
                .handle(&get.0, &s, 2)
                .await
                .unwrap()
                .result
                .unwrap()
                .as_slice(),
            std::slice::from_ref(&expected)
        );
        upload[14] ^= 1;
        assert!(server.handle(&upload, &s, 3).await.is_err());
        assert_eq!(
            server.handle(&get.0, &s, 4).await.unwrap().result.unwrap(),
            [expected]
        );
        assert_eq!(
            server.handle(&get.0, &session(1), 4).await.unwrap().result,
            Err(task::TASK_FAILED)
        );
        let mut other = Writer(vec![8, 3, 1]);
        other.u64(1);
        other.0.push(0);
        assert_eq!(
            server
                .handle(&other.0, &session(1), 4)
                .await
                .unwrap()
                .result,
            Err(task::NO_PROFILE)
        );
    }
    #[tokio::test]
    async fn private_profile_is_bound_to_session_and_survives_restart() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("data");
        let server = LocalServices::open(root.clone()).await.unwrap();
        let s = session(u64::MAX);
        let data = include_bytes!("../../../fixtures/protocol/private-profile-synthetic.bin");
        let path = server.profile_path(&s, true).await.unwrap();
        atomic_private_write(&path, data).await.unwrap();
        drop(server);
        let server = LocalServices::open(root).await.unwrap();
        let record = profile::private_record(s.user_id, data).unwrap();
        assert_eq!(
            server
                .handle(&[8, 3, 2, 0], &s, 1)
                .await
                .unwrap()
                .result
                .unwrap(),
            [record]
        );
        assert_eq!(
            server
                .handle(&[8, 3, 2, 0], &session(1), 1)
                .await
                .unwrap()
                .result,
            Err(task::NO_PROFILE)
        );
        let mut other_title = s.clone();
        other_title.title_id = 2;
        assert_eq!(
            server
                .handle(&[8, 3, 2, 0], &other_title, 1)
                .await
                .unwrap()
                .result,
            Err(task::NO_PROFILE)
        );
        let mut damaged = data.to_vec();
        damaged[11] ^= 1;
        tokio::fs::write(path, damaged).await.unwrap();
        assert!(server.handle(&[8, 3, 2, 0], &s, 1).await.is_err());
    }
    #[tokio::test]
    async fn item_grants_require_the_authenticated_identity_and_sample() {
        let temp = tempfile::tempdir().unwrap();
        let server = LocalServices::open(temp.path().join("data")).await.unwrap();
        let s = session(1);
        let request = [80, 3, 230, 16, 0, 0];
        let path = server
            .profile_path(&s, false)
            .await
            .unwrap()
            .with_file_name("inventory.json");
        let data=serde_json::to_vec(&serde_json::json!({"schema_version":1,"client_sha256":codol_protocol::CLIENT_SHA256,"title_id":s.title_id,"user_id":"1","items":[792,10001]})).unwrap();
        atomic_private_write(&path, &data).await.unwrap();
        let reply = server.handle(&request, &s, 1).await.unwrap();
        assert_eq!(reply.label, "local_player_inventory");
        let records = reply.result.unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(&records[0][16..21], &[8, 2, 0, 0, 0]);
        let other = session(2);
        let other_path = server
            .profile_path(&other, false)
            .await
            .unwrap()
            .with_file_name("inventory.json");
        atomic_private_write(&other_path, &data).await.unwrap();
        assert!(server.handle(&request, &other, 1).await.is_err());
        tokio::fs::write(&path, b"invalid").await.unwrap();
        assert!(server.handle(&request, &s, 1).await.is_err());
    }

    #[tokio::test]
    async fn full_catalogue_inventory_loads_and_refreshes_without_truncation() {
        let temp = tempfile::tempdir().unwrap();
        let server = LocalServices::open(temp.path().join("data")).await.unwrap();
        let s = session(1);
        let path = server
            .profile_path(&s, false)
            .await
            .unwrap()
            .with_file_name("inventory.json");
        let items: Vec<u32> = (1..=6772).collect();
        let data = serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version":1,"client_sha256":codol_protocol::CLIENT_SHA256,
            "title_id":s.title_id,"user_id":"1","items":items
        }))
        .unwrap();
        assert!(data.len() > 32768);
        atomic_private_write(&path, &data).await.unwrap();
        let full = server
            .handle(&[80, 3, 230, 16, 0, 0], &s, 1)
            .await
            .unwrap()
            .result
            .unwrap();
        assert_eq!(full, vec![task::player_data(s.user_id, &items).unwrap()]);
        let mut query = Writer(vec![80, 3, 203]);
        query.string("");
        query.u16(20000);
        query.0.push(0);
        let refreshed = server
            .handle(&query.0, &s, 1)
            .await
            .unwrap()
            .result
            .unwrap();
        assert_eq!(refreshed.len(), 6772);
        assert_eq!(
            refreshed,
            task::inventory_records(s.user_id, &items).unwrap()
        );
        assert!(task::reply(1, 203, Ok(&refreshed)).unwrap().len() > 65536);
    }

    #[tokio::test]
    async fn empty_lab_catalogue_only_answers_supported_unfiltered_queries() {
        let temp = tempfile::tempdir().unwrap();
        let server = LocalServices::open(temp.path().join("data")).await.unwrap();
        let s = session(1);
        let mut query = Writer(vec![80, 3, 70]);
        query.string("");
        query.u32(1);
        query.u32(2000);
        query.boolean(true);
        query.u32(0);
        query.u32(0);
        query.string("982808");
        query.0.push(0);
        assert_eq!(
            server.handle(&query.0, &s, 1).await.unwrap().result,
            Ok(vec![])
        );
        let mut links = Writer(vec![80, 3, 107]);
        links.string("");
        links.u32(1);
        links.u32(2000);
        links.string("982808");
        links.0.push(0);
        assert_eq!(
            server.handle(&links.0, &s, 1).await.unwrap().result,
            Ok(vec![])
        );
        for n in 0..links.0.len() {
            assert!(server.handle(&links.0[..n], &s, 1).await.is_err());
        }
        for n in 0..query.0.len() {
            assert!(server.handle(&query.0[..n], &s, 1).await.is_err());
        }
        let mut page = query.0.clone();
        page[6] = 2;
        assert_eq!(
            server.handle(&page, &s, 1).await.unwrap().result,
            Err(task::TASK_FAILED)
        );
        let mut filter = query.0;
        filter[18] = 1;
        assert_eq!(
            server.handle(&filter, &s, 1).await.unwrap().result,
            Err(task::TASK_FAILED)
        );
    }
}
