//! New local players use prepared starter equipment and independent data roots.
use crate::infra::storage::{atomic_private_write, create_directory, read_bounded};
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use codol_protocol::lobby::{profile, task};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub nickname: String,
    pub level: u8,
    pub unlock_all: bool,
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.nickname.trim().is_empty()
                && self.nickname == self.nickname.trim()
                && self.nickname.len() <= 31
                && !self
                    .nickname
                    .chars()
                    .any(|c| c.is_control() || matches!(c, '"' | '\\' | ';' | '^')),
            "nickname must contain 1..31 UTF-8 bytes without command/control characters"
        );
        ensure!((1..=70).contains(&self.level), "player level must be 1..70");
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Template {
    schema_version: u32,
    client_sha256: String,
    native_user_id: String,
    public_base64: String,
    starter_items: Vec<u32>,
    all_items: Vec<u32>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Player {
    schema_version: u32,
    client_sha256: String,
    title_id: u32,
    user_id: String,
    player_id: String,
    settings: Settings,
}

pub(crate) async fn nickname(root: &Path, title_id: u32, user_id: u64) -> Result<Option<String>> {
    let path = root
        .join("profiles")
        .join(title_id.to_string())
        .join(user_id.to_string())
        .join("player.json");
    let Some(bytes) = read_bounded(&path, 4096).await? else {
        return Ok(None);
    };
    let player: Player = serde_json::from_slice(&bytes)?;
    ensure!(
        player.schema_version == 1
            && player.client_sha256 == codol_protocol::CLIENT_SHA256
            && player.title_id == title_id
            && player.user_id == user_id.to_string()
            && player.player_id.len() == 32
            && player.player_id.bytes().all(|c| c.is_ascii_hexdigit()),
        "local player identity mismatch"
    );
    player.settings.validate()?;
    Ok(Some(player.settings.nickname))
}

/// Creates a complete new root; existing player data is never changed.
pub async fn create(
    source: &Path,
    template: &Path,
    parent: &Path,
    title_id: u32,
    settings: Settings,
) -> Result<PathBuf> {
    settings.validate()?;
    ensure!(title_id != 0, "title id must be nonzero");
    let bytes = read_bounded(template, 256 * 1024)
        .await?
        .context("new-player template is missing")?;
    let template: Template = serde_json::from_slice(&bytes)?;
    ensure!(
        template.schema_version == 1 && template.client_sha256 == codol_protocol::CLIENT_SHA256,
        "new-player template version mismatch"
    );
    let user_id = template.native_user_id.parse::<u64>()?;
    ensure!(user_id != 0, "invalid native user id");
    let public = STANDARD.decode(&template.public_base64)?;
    ensure!(
        public.len() <= task::MAX_FILE_BYTES,
        "starter profile exceeds limit"
    );
    profile::validate_public(&public)?;
    for items in [&template.starter_items, &template.all_items] {
        ensure!(
            !items.is_empty()
                && items.len() <= task::MAX_INVENTORY_ITEMS
                && items.iter().all(|id| (1..=65535).contains(id))
                && items.iter().collect::<std::collections::HashSet<_>>().len() == items.len(),
            "invalid template inventory"
        );
    }
    ensure!(
        template
            .starter_items
            .iter()
            .all(|id| template.all_items.contains(id)),
        "starter equipment is absent from catalogue"
    );
    // A new private seed contains no other player's stats, wallet or receipts.
    let private = profile::new_private(user_id, settings.level)?;
    profile::player_rank(&private, user_id)?;
    let items = if settings.unlock_all {
        &template.all_items
    } else {
        &template.starter_items
    };
    let inventory = serde_json::to_vec(&serde_json::json!({"schema_version":1,
        "client_sha256":codol_protocol::CLIENT_SHA256,"title_id":title_id,"user_id":user_id.to_string(),"items":items}))?;
    let wallet = serde_json::to_vec(&serde_json::json!({"schema_version":1,
        "client_sha256":codol_protocol::CLIENT_SHA256,"title_id":title_id,"user_id":user_id.to_string(),"cp":0,"gp":0}))?;
    let mut random = [0; 16];
    openssl::rand::rand_bytes(&mut random)?;
    let player_id = random
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect::<String>();
    create_directory(parent).await?;
    let parent = tokio::fs::canonicalize(parent).await?;
    let staging = parent.join(format!(".{player_id}.pending"));
    let destination = parent.join(&player_id);
    tokio::fs::create_dir(&staging).await?;
    let result: Result<()> = async {
        let data = staging.join("server-data");
        create_directory(&data).await?;
        create_directory(&data.join("publisher")).await?;
        let publisher = source.join("publisher");
        let mut entries = tokio::fs::read_dir(&publisher)
            .await
            .context("starter publisher data is missing")?;
        while let Some(entry) = entries.next_entry().await? {
            let bytes = read_bounded(&entry.path(), task::MAX_FILE_BYTES)
                .await?
                .context("publisher file disappeared")?;
            atomic_private_write(&data.join("publisher").join(entry.file_name()), &bytes).await?;
        }
        for name in ["configs_s.info", "mp_playlists_shipRetail_release.info"] {
            ensure!(
                data.join("publisher").join(name).is_file(),
                "missing required publisher file"
            );
        }
        // Only shared rules are copied. Account balances and purchase history are fresh.
        if let Some(bytes) = read_bounded(&source.join("shop.json"), 4 * 1024 * 1024).await? {
            atomic_private_write(&data.join("shop.json"), &bytes).await?;
        }
        if source.join("progression").exists() {
            create_directory(&data.join("progression")).await?;
            let mut entries = tokio::fs::read_dir(source.join("progression")).await?;
            while let Some(entry) = entries.next_entry().await? {
                let bytes = read_bounded(&entry.path(), 4 * 1024 * 1024)
                    .await?
                    .context("progression rules disappeared")?;
                atomic_private_write(&data.join("progression").join(entry.file_name()), &bytes)
                    .await?;
            }
        }
        let account = data
            .join("profiles")
            .join(title_id.to_string())
            .join(user_id.to_string());
        create_directory(&account).await?;
        for (name, bytes) in [
            ("public.bin", public.as_slice()),
            ("private.bin", private.as_slice()),
            ("inventory.json", inventory.as_slice()),
            ("wallet.json", wallet.as_slice()),
        ] {
            atomic_private_write(&account.join(name), bytes).await?;
        }
        let player = Player {
            schema_version: 1,
            client_sha256: codol_protocol::CLIENT_SHA256.into(),
            title_id,
            user_id: user_id.to_string(),
            player_id,
            settings,
        };
        atomic_private_write(
            &account.join("player.json"),
            &serde_json::to_vec_pretty(&player)?,
        )
        .await?;
        tokio::fs::rename(&staging, &destination).await?;
        Ok(())
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_dir_all(&staging).await;
    }
    result?;
    Ok(destination)
}

#[cfg(test)]
mod tests;
