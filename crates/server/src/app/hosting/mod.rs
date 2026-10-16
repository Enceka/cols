//! Private configuration and supervision for the admin-owned server processes.
mod config;
mod keys;
mod process;
#[cfg(test)]
mod tests;

use crate::infra::storage::{atomic_private_write, create_directory, read_bounded, write_private};
use anyhow::{Context, Result, ensure};
pub(crate) use config::Settings;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::{
    sync::{Mutex, watch},
    task::JoinHandle,
};

struct State {
    settings: Settings,
    phase: &'static str,
    message: String,
    error: Option<String>,
    endpoints: Value,
    log_file: Option<PathBuf>,
    stop: Option<watch::Sender<bool>>,
    worker: Option<JoinHandle<()>>,
}

pub(crate) struct Hosting {
    root: PathBuf,
    admin_port: u16,
    executable: PathBuf,
    state: Mutex<State>,
    _lock: File,
}

fn revision(settings: &Settings) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(settings).expect("settings serialize"))
    )
}

impl Hosting {
    pub async fn open(root: PathBuf, address: SocketAddr) -> Result<Arc<Self>> {
        create_directory(&root).await?;
        let root = tokio::fs::canonicalize(root).await?;
        let lock_path = root.join("control.lock");
        if lock_path.exists() {
            ensure!(
                !lock_path.symlink_metadata()?.is_symlink(),
                "管理锁不能为符号链接"
            );
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options.open(lock_path)?;
        lock.try_lock().context("该开服目录已由另一管理面板使用")?;
        let workspace = std::env::current_dir()?;
        let settings = config::load(
            &root.join("server.json"),
            Settings::defaults(&workspace, &root),
            address.port(),
        )?;
        Ok(Arc::new(Self {
            root,
            admin_port: address.port(),
            executable: std::env::current_exe()?,
            _lock: lock,
            state: Mutex::new(State {
                settings,
                phase: "stopped",
                message: "设置地图与服务资料后，即可一键开服".into(),
                error: None,
                endpoints: json!({}),
                log_file: None,
                stop: None,
                worker: None,
            }),
        }))
    }

    pub async fn snapshot(&self) -> Value {
        let state = self.state.lock().await;
        let certificate = match keys::read(&self.root).await {
            Ok(value) => json!({"ready":value.is_some(), "details":value, "error":null}),
            Err(error) => json!({"ready":false,"details":null,"error":format!("{error:#}")}),
        };
        json!({
            "settings":state.settings,"revision":revision(&state.settings),"path":self.root.join("server.json"),
            "phase":state.phase,"message":state.message,"error":state.error,
            "endpoints":state.endpoints,"log_file":state.log_file,"certificate":certificate,
            "missing":state.settings.check_inputs(),
        })
    }

    async fn check_saved(&self, settings: &Settings) -> Result<()> {
        if let Some(bytes) = read_bounded(&self.root.join("server.json"), 65536).await? {
            let saved: config::Saved = serde_json::from_slice(&bytes)?;
            ensure!(
                saved.schema_version == 1 && revision(&saved.settings) == revision(settings),
                "开服配置已被外部修改，请重启管理面板加载"
            );
        }
        Ok(())
    }

    pub async fn save(&self, settings: Settings, expected: &str) -> Result<Value> {
        settings.validate(self.admin_port)?;
        let mut state = self.state.lock().await;
        ensure!(!active(state.phase), "请先停止服务再修改开服设置");
        ensure!(
            revision(&state.settings) == expected,
            "开服设置已被修改，请重新加载"
        );
        self.check_saved(&state.settings).await?;
        atomic_private_write(
            &self.root.join("server.json"),
            &serde_json::to_vec_pretty(&config::Saved {
                schema_version: 1,
                settings: settings.clone(),
            })?,
        )
        .await?;
        state.settings = settings;
        drop(state);
        Ok(self.snapshot().await)
    }

    pub async fn start(self: &Arc<Self>) -> Result<Value> {
        let mut state = self.state.lock().await;
        if active(state.phase) {
            drop(state);
            return Ok(self.snapshot().await);
        }
        let errors = state.settings.check_inputs();
        ensure!(errors.is_empty(), "{}", errors.join("；"));
        let settings = state.settings.clone();
        self.check_saved(&state.settings).await?;
        atomic_private_write(
            &self.root.join("server.json"),
            &serde_json::to_vec_pretty(&config::Saved {
                schema_version: 1,
                settings: settings.clone(),
            })?,
        )
        .await?;
        let (stop, receiver) = watch::channel(false);
        state.phase = "starting";
        state.message = "正在准备证书与服务资料".into();
        state.error = None;
        state.endpoints = json!({});
        state.stop = Some(stop);
        let this = self.clone();
        state.worker = Some(tokio::spawn(async move {
            // The monitor remains responsible for status even if the worker panics.
            let owner = this.clone();
            let outcome =
                tokio::spawn(async move { process::run(owner, settings, receiver).await }).await;
            let mut state = this.state.lock().await;
            match outcome {
                Ok(Ok(())) => {
                    state.phase = "stopped";
                    state.message = "服务已停止，玩家资料与证书已保留".into();
                }
                result => {
                    let error = match result {
                        Ok(Err(error)) => format!("{error:#}"),
                        Err(error) => format!("服务管理任务异常：{error}"),
                        _ => unreachable!(),
                    };
                    state.phase = "failed";
                    state.message = "开服失败，已回收本次服务".into();
                    state.error = Some(error);
                }
            }
            state.endpoints = json!({});
            state.stop = None;
        }));
        drop(state);
        Ok(self.snapshot().await)
    }

    pub async fn stop(&self) -> Value {
        let mut state = self.state.lock().await;
        if let Some(stop) = &state.stop {
            stop.send_replace(true);
            state.phase = "stopping";
            state.message = "正在停止本次服务并释放端口".into();
        }
        drop(state);
        self.snapshot().await
    }

    pub async fn shutdown(&self) {
        self.stop().await;
        let worker = self.state.lock().await.worker.take();
        if let Some(worker) = worker {
            let _ = worker.await;
        }
    }

    pub async fn certificate(&self, renew: bool) -> Result<Value> {
        let state = self.state.lock().await;
        if renew {
            ensure!(!active(state.phase), "请先停止服务再更新证书");
            Ok(json!(keys::ensure_identity(&self.root, true).await?))
        } else {
            Ok(json!(
                keys::read(&self.root)
                    .await?
                    .context("尚未生成证书，请先开服")?
            ))
        }
    }

    pub async fn log_file(&self) -> Option<PathBuf> {
        self.state.lock().await.log_file.clone()
    }

    pub(super) async fn progress(&self, message: &str) {
        let mut state = self.state.lock().await;
        if state.phase == "starting" {
            state.message = message.into();
        }
    }

    pub(super) async fn prepare(
        &self,
        settings: &Settings,
    ) -> Result<(PathBuf, keys::Certificate)> {
        let identity = keys::ensure_identity(&self.root, false).await?;
        create_directory(&settings.service_data_dir).await?;
        let publisher = settings.service_data_dir.join("publisher");
        create_directory(&publisher).await?;
        let files = [
            (
                "configs_s.info",
                codol_protocol::lobby::publisher::crypt_configs(include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../configs/publisher/configs_s.ini"
                )))?,
            ),
            (
                "mp_playlists_shipRetail_release.info",
                codol_protocol::lobby::publisher::encode_playlist(include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../configs/publisher/mp_playlists_shipRetail_release.info"
                )))?,
            ),
        ];
        for (name, bytes) in files {
            let path = publisher.join(name);
            if !path.try_exists()? {
                write_private(&path, &bytes).await?;
            }
        }
        let mut random = [0; 12];
        openssl::rand::rand_bytes(&mut random)?;
        let run = self.root.join("runs").join(
            random
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>(),
        );
        create_directory(&run).await?;
        self.state.lock().await.log_file = Some(run.join("server.log"));
        Ok((run, identity))
    }
}

fn active(phase: &str) -> bool {
    matches!(phase, "starting" | "running" | "stopping")
}

pub(crate) async fn observer_config(path: &Path) -> Result<()> {
    if !path.try_exists()? {
        if let Some(parent) = path.parent() {
            create_directory(parent).await?;
        }
        write_private(
            path,
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../configs/local.toml"
            )),
        )
        .await?;
    }
    Ok(())
}
