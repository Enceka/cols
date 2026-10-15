//! Explicit offline awards in a marked isolated diagnostic service directory.
use crate::{
    app::services::LocalServices, contracts::account::AccountId,
    contracts::progression::MatchAward, infra::storage::read_bounded,
};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DiagnosticAward {
    title_id: u32,
    user_id: String,
    award: MatchAward,
}

pub(crate) async fn commit(root: &Path, input: &Path) -> Result<serde_json::Value> {
    let marker = read_bounded(&root.join(".codol-progression-diagnostic"), 128)
        .await?
        .context("explicit progression diagnostic marker required")?;
    ensure!(
        marker == b"isolated-offline-progression-v1\n",
        "unexpected progression diagnostic marker"
    );
    let bytes = read_bounded(input, 32768)
        .await?
        .context("diagnostic award input missing")?;
    let request: DiagnosticAward = serde_json::from_slice(&bytes)?;
    let account = AccountId {
        title_id: request.title_id,
        user_id: request.user_id.parse()?,
    };
    let services = LocalServices::open(root.to_path_buf()).await?;
    let result = services
        .commit_match_progression(account, request.award)
        .await?;
    Ok(
        serde_json::json!({"duplicate":result.duplicate,"receipt":result.receipt,
        "scope":"isolated_offline_progression_diagnostic"}),
    )
}
