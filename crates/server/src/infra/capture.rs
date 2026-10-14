use crate::app::config::Config;
use anyhow::{Context, Result};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    net::SocketAddr,
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    fs::{File, OpenOptions},
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::watch,
    time::Instant,
};

pub fn unix_time_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

async fn create(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    options
        .open(path)
        .await
        .with_context(|| format!("create {}", path.display()))
}

async fn event(file: &mut File, value: serde_json::Value) -> Result<()> {
    let mut bytes = serde_json::to_vec(&value)?;
    bytes.push(b'\n');
    file.write_all(&bytes).await?;
    // Events are visible during a run; end events mark a fully closed capture.
    file.flush().await?;
    Ok(())
}

pub struct Session<'a> {
    pub id: u64,
    pub listener: &'a str,
    pub peer: SocketAddr,
    pub directory: &'a Path,
    pub config: &'a Config,
}

pub async fn record_tcp(
    mut stream: TcpStream,
    session: Session<'_>,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
    let started = Instant::now();
    let deadline = started + Duration::from_millis(session.config.session_timeout_ms);
    let stem = format!("{:06}-{}", session.id, session.listener);
    let mut raw = create(&session.directory.join(format!("{stem}.bin"))).await?;
    let mut events = create(&session.directory.join(format!("{stem}.jsonl"))).await?;
    event(
        &mut events,
        json!({
            "event": "session_start", "schema_version": 1,
            "session_id": session.id, "listener": session.listener,
            "peer": session.peer, "local": stream.local_addr()?,
            "transport": "tcp", "direction": "client_to_server",
            "unix_time_ms": unix_time_ms(),
        }),
    )
    .await?;

    let mut buffer = [0u8; 8192];
    let mut total = 0usize;
    let mut chunks = 0usize;
    let mut hash = Sha256::new();
    let mut detail = None;
    let reason = loop {
        if *stop.borrow() {
            break "shutdown";
        }
        if total == session.config.max_bytes_per_session {
            break "byte_limit";
        }
        if chunks == session.config.max_chunks_per_session {
            break "chunk_limit";
        }
        let available = buffer
            .len()
            .min(session.config.max_bytes_per_session - total);
        let read = tokio::select! {
            biased;
            _ = stop.changed() => { break "shutdown"; }
            _ = tokio::time::sleep_until(deadline) => { break "session_timeout"; }
            _ = tokio::time::sleep(Duration::from_millis(session.config.idle_timeout_ms)) => { break "idle_timeout"; }
            read = stream.read(&mut buffer[..available]) => read,
        };
        let count = match read {
            Ok(0) => break "peer_eof",
            Ok(count) => count,
            Err(error) => {
                detail = Some(error.to_string());
                break "read_error";
            }
        };
        raw.write_all(&buffer[..count])
            .await
            .context("write capture bytes")?;
        hash.update(&buffer[..count]);
        event(
            &mut events,
            json!({
                "event": "stream_chunk", "offset": total, "length": count,
                "elapsed_us": started.elapsed().as_micros(),
            }),
        )
        .await?;
        total += count;
        chunks += 1;
    };
    raw.flush().await?;
    event(
        &mut events,
        json!({
            "event": "session_end", "reason": reason, "detail": detail,
            "bytes": total, "chunks": chunks, "sha256": format!("{:x}", hash.finalize()),
            "elapsed_us": started.elapsed().as_micros(),
        }),
    )
    .await?;
    tracing::info!(
        session_id = session.id,
        listener = session.listener,
        bytes = total,
        reason,
        "capture closed"
    );
    Ok(())
}
