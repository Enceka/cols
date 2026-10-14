//! Shared account file layout and write coordination for backend services.
use crate::contracts::account::AccountId;
use crate::infra::storage::create_directory;
use anyhow::{Result, ensure};
use std::path::PathBuf;
use tokio::sync::Mutex;

pub(crate) struct AccountStore {
    pub(crate) root: PathBuf,
    pub(crate) writes: Mutex<()>,
}
impl AccountStore {
    pub(crate) async fn open(root: PathBuf) -> Result<Self> {
        create_directory(&root).await?;
        for name in ["publisher", "users", "profiles"] {
            create_directory(&root.join(name)).await?;
        }
        Ok(Self {
            root: tokio::fs::canonicalize(root).await?,
            writes: Mutex::new(()),
        })
    }
    pub(crate) async fn profile_path(
        &self,
        session: impl Into<AccountId>,
        private: bool,
    ) -> Result<PathBuf> {
        let session = session.into();
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
    pub(crate) async fn profile_directory(&self, title_id: u32, user_id: u64) -> Result<PathBuf> {
        let title = self.root.join("profiles").join(title_id.to_string());
        create_directory(&title).await?;
        let user = title.join(user_id.to_string());
        create_directory(&user).await?;
        Ok(user)
    }
    pub(crate) async fn user_path(
        &self,
        session: impl Into<AccountId>,
        name: &str,
    ) -> Result<PathBuf> {
        let session = session.into();
        let title = self.root.join("users").join(session.title_id.to_string());
        create_directory(&title).await?;
        let user = title.join(session.user_id.to_string());
        create_directory(&user).await?;
        Ok(user.join(format!("{name}.json")))
    }
}
