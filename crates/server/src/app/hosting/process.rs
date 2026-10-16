use super::{Hosting, Settings, keys::Certificate};
use crate::{
    app::config::{Config, ListenerConfig},
    infra::storage::{read_bounded, write_private},
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::Path,
    process::Stdio,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::{Child, Command},
    sync::watch,
    task::JoinHandle,
};

const MAX_LOG: u64 = 8 * 1024 * 1024;
struct OwnedProcess {
    child: Child,
    readers: Vec<JoinHandle<()>>,
}

impl OwnedProcess {
    fn spawn(executable: &Path, args: &[String], log: Arc<Mutex<File>>) -> Result<Self> {
        let mut child = Command::new(executable)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("RUST_LOG", "info")
            .kill_on_drop(true)
            .spawn()
            .context("启动自有服务进程")?;
        let out = child.stdout.take().context("服务日志通道缺失")?;
        let err = child.stderr.take().context("服务错误日志通道缺失")?;
        Ok(Self {
            child,
            readers: vec![
                tokio::spawn(copy_log(out, log.clone())),
                tokio::spawn(copy_log(err, log)),
            ],
        })
    }
    fn check(&mut self) -> Result<()> {
        if let Some(code) = self.child.try_wait()? {
            anyhow::bail!("服务进程已退出（{code}），详细原因见运行日志")
        }
        Ok(())
    }
    async fn close(&mut self) {
        // Only this owned stdin pipe controls the child; closing it also handles parent crashes.
        drop(self.child.stdin.take());
        if tokio::time::timeout(Duration::from_secs(10), self.child.wait())
            .await
            .is_err()
        {
            let _ = self.child.start_kill();
            let _ = self.child.wait().await;
        }
        for reader in self.readers.drain(..) {
            let _ = reader.await;
        }
    }
}

async fn copy_log(mut stream: impl AsyncRead + Unpin, log: Arc<Mutex<File>>) {
    let mut bytes = [0; 4096];
    while let Ok(n) = stream.read(&mut bytes).await {
        if n == 0 {
            break;
        }
        let Ok(mut file) = log.lock() else { break };
        if file
            .metadata()
            .map(|m| m.len() + n as u64 > MAX_LOG)
            .unwrap_or(false)
        {
            let _ = file.set_len(0);
        }
        let _ = file.write_all(&bytes[..n]);
    }
}

fn args(settings: &Settings, cert: &Certificate, run: &Path) -> Vec<String> {
    let mut args = vec!["serve-auth".into()];
    for (flag, value) in [
        ("--bind", settings.address(settings.auth_port)),
        ("--cert", cert.certificate.display().to_string()),
        ("--key", cert.private_key.display().to_string()),
        ("--local-title-id", "1002".into()),
        ("--lobby-bind", settings.address(settings.lobby_port)),
        (
            "--service-data-dir",
            settings.service_data_dir.display().to_string(),
        ),
        ("--stun-bind", settings.address(settings.stun_port)),
        ("--stun-alt", settings.address(settings.stun_alt_port)),
        ("--game-bind", settings.address(settings.game_port)),
        ("--game-state", settings.game_state.display().to_string()),
        (
            "--game-map-catalog",
            settings.map_catalog.display().to_string(),
        ),
        ("--game-capture-every", "1000".into()),
        ("--capture-dir", run.join("captures").display().to_string()),
        (
            "--launcher-ready-file",
            run.join("backend-ready.json").display().to_string(),
        ),
    ] {
        args.push(flag.into());
        args.push(value);
    }
    if settings.room_bot_fill != 0 {
        args.extend(["--room-bot-fill".into(), settings.room_bot_fill.to_string()]);
    }
    args.extend(["--continuous-play".into(), "--managed-stdin".into()]);
    args
}

async fn ready(path: &Path, process: &OwnedProcess, kind: &str) -> Result<Option<Value>> {
    let Some(bytes) = read_bounded(path, 16384).await? else {
        return Ok(None);
    };
    let value: Value = serde_json::from_slice(&bytes)?;
    ensure!(
        value["schema_version"] == 1
            && value["kind"] == kind
            && value["pid"].as_u64() == process.child.id().map(u64::from)
            && value["client_sha256"] == codol_protocol::CLIENT_SHA256,
        "服务就绪记录身份不匹配"
    );
    Ok(Some(value))
}

fn check_address(value: &Value, expected_port: u16) -> Result<String> {
    let address: std::net::SocketAddr = value.as_str().context("就绪端点缺失")?.parse()?;
    ensure!(
        address.ip() == std::net::Ipv4Addr::LOCALHOST
            && address.port() != 0
            && (expected_port == 0 || address.port() == expected_port),
        "服务就绪端点不匹配"
    );
    Ok(address.to_string())
}

fn endpoints(backend: &Value, resources: &Value, settings: &Settings) -> Result<Value> {
    ensure!(
        backend["lobby_protocol"] == 210
            && backend["game_protocol"] == 3
            && backend["title_id"] == 1002
            && backend["continuous_play"] == true
            && backend["game_state_loaded"] == true
            && backend["map_catalog_loaded"] == true
            && backend["native_bot_rooms"] == false
            && (if settings.room_bot_fill == 0 {
                backend["room_bot_fill"].is_null()
            } else {
                backend["room_bot_fill"] == settings.room_bot_fill
            }),
        "服务装配与开服设置不匹配"
    );
    let mut result = serde_json::Map::new();
    for (name, port) in [
        ("auth", settings.auth_port),
        ("lobby", settings.lobby_port),
        ("stun", settings.stun_port),
        ("stun_alt", settings.stun_alt_port),
        ("game", settings.game_port),
    ] {
        result.insert(name.into(), json!(check_address(&backend[name], port)?));
    }
    ensure!(
        resources["listeners"]
            .as_object()
            .is_some_and(|o| o.len() == 1),
        "资源观察器端点不匹配"
    );
    result.insert(
        "resources".into(),
        json!(check_address(
            &resources["listeners"]["resources"],
            settings.resource_port
        )?),
    );
    Ok(Value::Object(result))
}

pub(super) async fn run(
    owner: Arc<Hosting>,
    settings: Settings,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
    if *stop.borrow() {
        return Ok(());
    }
    crate::infra::storage::create_directory(&settings.service_data_dir).await?;
    let lock_path = settings.service_data_dir.join(".codol-managed.lock");
    if lock_path.exists() {
        ensure!(
            !lock_path.symlink_metadata()?.is_symlink(),
            "服务资料锁不能为链接"
        );
    }
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let _data_lock = options.open(lock_path)?;
    _data_lock
        .try_lock()
        .context("服务资料正由另一个受管服务使用")?;
    let (run, identity) = owner.prepare(&settings).await?;
    if *stop.borrow() {
        return Ok(());
    }
    let observer = Config {
        capture_dir: run.join("resources"),
        max_connections: 16,
        max_sessions_per_run: 128,
        max_bytes_per_session: 1048576,
        max_chunks_per_session: 4096,
        idle_timeout_ms: 10000,
        session_timeout_ms: 30000,
        listeners: vec![ListenerConfig {
            name: "resources".into(),
            bind: settings.address(settings.resource_port).parse()?,
        }],
    };
    let observer_path = run.join("resources.toml");
    write_private(&observer_path, toml::to_string(&observer)?.as_bytes()).await?;
    let mut options = OpenOptions::new();
    options.create_new(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let log = Arc::new(Mutex::new(options.open(run.join("server.log"))?));
    let mut backend = OwnedProcess::spawn(
        &owner.executable,
        &args(&settings, &identity, &run),
        log.clone(),
    )?;
    let mut resources = None;
    let outcome = async {
        resources = Some(OwnedProcess::spawn(&owner.executable, &[
            "observe".into(), "--config".into(), observer_path.display().to_string(),
            "--launcher-ready-file".into(), run.join("resources-ready.json").display().to_string(), "--managed-stdin".into(),
        ], log)?);
        let resources = resources.as_mut().expect("observer spawned");
        owner.progress("证书已准备，正在加载地图并启动认证、大厅与游戏通道").await;
        let started = Instant::now();
        loop {
            if *stop.borrow() { return Ok(()) }
            backend.check()?; resources.check()?;
            if let (Some(a), Some(b)) = (ready(&run.join("backend-ready.json"), &backend, "native_services").await?, ready(&run.join("resources-ready.json"), resources, "tcp_observer").await?) {
                let endpoints = endpoints(&a, &b, &settings)?;
                let mut state = owner.state.lock().await;
                if state.phase == "starting" {
                    state.phase = "running"; state.message = "服务已就绪，可使用独立客户端连接".into(); state.endpoints = endpoints;
                }
                break;
            }
            ensure!(started.elapsed() < Duration::from_secs(240), "等待地图与服务就绪超时，详细原因见运行日志");
            tokio::select! { _ = stop.changed() => {}, _ = tokio::time::sleep(Duration::from_millis(100)) => {} }
        }
        loop {
            if *stop.borrow() { return Ok(()) }
            backend.check()?; resources.check()?;
            tokio::select! { _ = stop.changed() => {}, _ = tokio::time::sleep(Duration::from_millis(250)) => {} }
        }
    }.await;
    // Drop both control pipes before waiting, so either sibling can finish promptly.
    drop(backend.child.stdin.take());
    if let Some(child) = &mut resources {
        drop(child.child.stdin.take());
    }
    backend.close().await;
    if let Some(child) = &mut resources {
        child.close().await;
    }
    outcome
}
