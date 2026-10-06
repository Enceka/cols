use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    net::SocketAddr,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub capture_dir: PathBuf,
    pub max_connections: usize,
    pub max_sessions_per_run: u64,
    pub max_bytes_per_session: usize,
    pub max_chunks_per_session: usize,
    pub idle_timeout_ms: u64,
    pub session_timeout_ms: u64,
    pub listeners: Vec<ListenerConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ListenerConfig {
    pub name: String,
    pub bind: SocketAddr,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("read config {}", path.display()))?;
        let mut config: Self = toml::from_str(&text).context("parse config")?;
        if config.capture_dir.is_relative() {
            config.capture_dir = path
                .parent()
                .unwrap_or(Path::new("."))
                .join(&config.capture_dir);
        }
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.capture_dir.as_os_str().is_empty(),
            "capture_dir is empty"
        );
        ensure!(
            (1..=256).contains(&self.max_connections),
            "max_connections must be 1..256"
        );
        ensure!(
            (1..=4096).contains(&self.max_sessions_per_run),
            "max_sessions_per_run must be 1..4096"
        );
        ensure!(
            (1..=16 * 1024 * 1024).contains(&self.max_bytes_per_session),
            "max_bytes_per_session must be 1..16 MiB"
        );
        ensure!(
            (1..=65536).contains(&self.max_chunks_per_session),
            "max_chunks_per_session must be 1..65536"
        );
        ensure!(
            (1..=3_600_000).contains(&self.idle_timeout_ms),
            "idle_timeout_ms must be 1..3600000"
        );
        ensure!(
            (1..=3_600_000).contains(&self.session_timeout_ms),
            "session_timeout_ms must be 1..3600000"
        );
        ensure!(
            !self.listeners.is_empty() && self.listeners.len() <= 8,
            "need 1..8 listeners"
        );
        let mut names = HashSet::new();
        let mut addresses = HashSet::new();
        for listener in &self.listeners {
            ensure!(
                !listener.name.is_empty()
                    && listener.name.len() <= 40
                    && listener
                        .name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
                "listener name must be 1..40 ASCII letters, digits, _ or -"
            );
            ensure!(names.insert(&listener.name), "duplicate listener name");
            // Port zero is useful for isolated tests. OS binding detects wildcard overlaps.
            ensure!(
                listener.bind.port() == 0 || addresses.insert(listener.bind),
                "duplicate bind address"
            );
        }
        Ok(())
    }
}
