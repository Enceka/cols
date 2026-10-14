//! Identity-scoped user files, schema validation and quotas.
use crate::contracts::account::AccountId;
use crate::infra::storage::accounts::AccountStore;
use crate::infra::storage::atomic_private_write;
use crate::infra::storage::read_bounded;
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use codol_protocol::lobby::task;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;

pub(crate) struct FileService {
    store: Arc<AccountStore>,
}
impl FileService {
    pub(crate) fn new(store: Arc<AccountStore>) -> Self {
        Self { store }
    }
    pub(crate) async fn read(
        &self,
        session: impl Into<AccountId>,
        name: &str,
    ) -> Result<Option<Vec<u8>>> {
        let session = session.into();
        ensure!(valid_name(name), "invalid user filename");
        let path = self.store.user_path(session, name).await?;
        read_stored(&path)
            .await?
            .map(|record| STANDARD.decode(record.data_base64).map_err(Into::into))
            .transpose()
    }
    pub(crate) async fn upload(
        &self,
        session: impl Into<AccountId>,
        name: &str,
        visibility: bool,
        data: &[u8],
        now: u32,
    ) -> Result<StoredFile> {
        let session = session.into();
        ensure!(
            valid_name(name) && data.len() <= task::MAX_FILE_BYTES,
            "invalid user file"
        );
        let _lock = self.store.writes.lock().await;
        let path = self.store.user_path(session, name).await?;
        let previous = read_stored(&path).await?;
        if previous.is_none() {
            let mut entries = tokio::fs::read_dir(path.parent().context("user folder")?).await?;
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
        atomic_private_write(&path, &serde_json::to_vec(&record)?).await?;
        Ok(record)
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredFile {
    pub(crate) schema: u32,
    pub(crate) created: u32,
    pub(crate) modified: u32,
    pub(crate) visibility: bool,
    pub(crate) data_base64: String,
}
pub(crate) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() < 128
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
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
