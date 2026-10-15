//! Offline account maintenance with recoverable backups.
use crate::backend::commerce::marketplace::LocalWallet;
use crate::infra::storage::accounts::AccountStore;
use crate::infra::storage::atomic_private_write;
use crate::infra::storage::read_bounded;
use anyhow::{Context, Result, ensure};
use codol_protocol::lobby::{profile, task};
use std::path::PathBuf;
pub(crate) async fn boost_test_account(
    accounts: &AccountStore,
    title_id: u32,
    user_id: u64,
    cp: u32,
    gp: u32,
) -> Result<PathBuf> {
    let _lock = accounts.writes.lock().await;
    let directory = accounts.profile_directory(title_id, user_id).await?;
    ensure!(
        read_bounded(&directory.join("progression.json"), 4 * 1024 * 1024)
            .await?
            .is_none(),
        "progressed account maintenance requires an explicit ledger migration"
    );
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
