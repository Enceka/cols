//! Operator-provided publisher files and playlist version.
pub(crate) struct PublisherFile {
    pub size: u32,
    pub id: u64,
    pub modified: u32,
    pub name: String,
}
use crate::backend::files::valid_name;
use crate::infra::storage::accounts::AccountStore;
use crate::infra::storage::read_bounded;
use anyhow::{Result, ensure};
use codol_protocol::lobby::task;
use sha2::{Digest, Sha256};
use std::sync::Arc;

pub(crate) struct PublisherService {
    store: Arc<AccountStore>,
}
impl PublisherService {
    pub(crate) fn new(store: Arc<AccountStore>) -> Self {
        Self { store }
    }
    pub(crate) async fn playlist_version(&self) -> Result<u32> {
        let bytes = tokio::fs::read(
            self.store
                .root
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
    pub(crate) async fn read(&self, name: &str) -> Result<Option<Vec<u8>>> {
        ensure!(
            crate::backend::files::valid_name(name),
            "invalid publisher filename"
        );
        read_bounded(
            &self.store.root.join("publisher").join(name),
            task::MAX_FILE_BYTES,
        )
        .await
    }
    pub(crate) async fn list(&self, limit: u16) -> Result<Vec<PublisherFile>> {
        let mut entries = tokio::fs::read_dir(self.store.root.join("publisher")).await?;
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
            let path = self.store.root.join("publisher").join(&name);
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
            records.push(PublisherFile {
                size: data.len() as u32,
                id: file_id,
                modified,
                name,
            });
        }
        Ok(records)
    }
}
