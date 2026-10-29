use crate::contracts::account::AccountId;
use crate::infra::storage::{accounts::AccountStore, atomic_private_write, read_bounded};
use anyhow::{Context, Result, ensure};
use codol_protocol::lobby::profile;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

const MAX_LEDGER_BYTES: usize = 64 * 1024;
const MAX_COMPLETIONS: usize = 32;
const MAX_ATTEMPTS: u32 = 10_000;
const MAX_SECONDS: u32 = 86_400;

#[derive(Clone, Debug)]
pub(crate) struct Completion {
    pub map: String,
    pub attempts: u32,
    pub seconds: u32,
    pub first_time: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    map: String,
    attempts: u32,
    seconds: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ledger {
    schema_version: u32,
    client_sha256: String,
    title_id: u32,
    user_id: String,
    completions: Vec<Saved>,
}

impl Ledger {
    fn validate(&self, account: AccountId, seed: &[u8]) -> Result<()> {
        ensure!(
            self.schema_version == 1
                && self.client_sha256 == codol_protocol::CLIENT_SHA256
                && self.title_id == account.title_id
                && self.user_id == account.user_id.to_string()
                && self.completions.len() <= MAX_COMPLETIONS,
            "story ledger identity or size mismatch"
        );
        let mut seen = std::collections::HashSet::new();
        for saved in &self.completions {
            ensure!(
                saved.map == codol_protocol::room::story::FIRST_MAP
                    && seen.insert(saved.map.clone())
                    && (1..=MAX_ATTEMPTS).contains(&saved.attempts)
                    && profile::story_unlock(
                        seed,
                        account.user_id,
                        codol_protocol::room::story::FIRST_COMPLETION_FLAG
                    )? >= 1,
                "invalid persisted story completion"
            );
        }
        Ok(())
    }
}

pub(crate) struct StoryService {
    store: Arc<AccountStore>,
}

impl StoryService {
    pub(crate) fn new(store: Arc<AccountStore>) -> Self {
        Self { store }
    }

    /// Record a finished mission once. The first completion raises the level's
    /// unlock flag; repeats only refresh the stored attempt counts.
    pub(crate) async fn complete(
        &self,
        account: AccountId,
        map: &str,
        attempts: u32,
        seconds: u32,
    ) -> Result<Completion> {
        ensure!(
            map == codol_protocol::room::story::FIRST_MAP,
            "only the first story level can be completed"
        );
        ensure!(
            (1..=MAX_ATTEMPTS).contains(&attempts),
            "story completion attempts out of range"
        );
        ensure!(
            seconds <= MAX_SECONDS,
            "story completion duration out of range"
        );
        let _lock = self.store.writes.lock().await;
        let directory = self
            .store
            .profile_directory(account.title_id, account.user_id)
            .await?;
        let private_path = self.store.profile_path(account, true).await?;
        let seed = read_bounded(&private_path, 61440)
            .await?
            .context("story completion requires an existing private profile")?;
        let path = directory.join("story.json");
        let mut ledger = match read_bounded(&path, MAX_LEDGER_BYTES).await? {
            Some(bytes) => {
                let ledger: Ledger = serde_json::from_slice(&bytes)?;
                ledger.validate(account, &seed)?;
                ledger
            }
            None => Ledger {
                schema_version: 1,
                client_sha256: codol_protocol::CLIENT_SHA256.into(),
                title_id: account.title_id,
                user_id: account.user_id.to_string(),
                completions: Vec::new(),
            },
        };
        let first_time = !ledger.completions.iter().any(|saved| saved.map == map);
        let unlocked = profile::with_story_unlock(
            &seed,
            account.user_id,
            codol_protocol::room::story::FIRST_COMPLETION_FLAG,
        )?;
        if unlocked != seed {
            atomic_private_write(&private_path, &unlocked).await?;
        }
        if first_time {
            ledger.completions.push(Saved {
                map: map.into(),
                attempts,
                seconds,
            });
            ledger.validate(account, &unlocked)?;
            let bytes = serde_json::to_vec(&ledger)?;
            ensure!(
                bytes.len() <= MAX_LEDGER_BYTES,
                "story ledger exceeds byte budget"
            );
            atomic_private_write(&path, &bytes).await?;
        }
        Ok(Completion {
            map: map.into(),
            attempts,
            seconds,
            first_time,
        })
    }
}

#[cfg(test)]
mod tests;
