//! Bounded reads and atomic private-file replacement.
use anyhow::{Result, ensure};
use std::{io::ErrorKind, path::Path};
use tokio::{
    fs::OpenOptions,
    io::{AsyncReadExt, AsyncWriteExt},
};

pub(crate) async fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path).await?;
    file.write_all(bytes).await?;
    file.flush().await?;
    Ok(())
}

pub(crate) async fn atomic_private_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut random = [0; 8];
    openssl::rand::rand_bytes(&mut random)?;
    let temporary = path.with_extension(format!("{:016x}.tmp", u64::from_le_bytes(random)));
    let result = async {
        write_private(&temporary, bytes).await?;
        // Windows requires write access for FlushFileBuffers.
        OpenOptions::new()
            .write(true)
            .open(&temporary)
            .await?
            .sync_all()
            .await?;
        tokio::fs::rename(&temporary, path).await?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(temporary).await;
    }
    result
}

pub(crate) async fn create_directory(path: &Path) -> Result<()> {
    tokio::fs::create_dir_all(path).await?;
    let metadata = tokio::fs::symlink_metadata(path).await?;
    ensure!(
        metadata.is_dir() && !metadata.is_symlink(),
        "local data directory must be a real directory"
    );
    Ok(())
}

pub(crate) async fn read_bounded(path: &Path, limit: usize) -> Result<Option<Vec<u8>>> {
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

pub(crate) mod accounts;

#[cfg(test)]
mod tests;
