//! Loopback management UI with bounded diagnostic commands and a fixed config/log path.
use crate::app::config::Config;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Mutex, Semaphore},
};

const MAX_BODY: usize = 64 * 1024;
const MAX_HEADERS: usize = 8192;
const LOG_TAIL_BYTES: u64 = 128 * 1024;
const HTML: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/admin/index.html"));
const CSS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/admin/style.css"));
const JS: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/admin/app.js"));

#[derive(Clone, Serialize)]
struct LogEntry {
    time: u64,
    level: &'static str,
    source: &'static str,
    message: String,
}

struct State {
    address: SocketAddr,
    config: PathBuf,
    log_file: Option<PathBuf>,
    started: Instant,
    logs: Mutex<VecDeque<LogEntry>>,
    config_lock: Mutex<()>,
}

pub struct AdminServer {
    listener: TcpListener,
    state: Arc<State>,
}

impl AdminServer {
    pub async fn bind(
        address: SocketAddr,
        config: PathBuf,
        log_file: Option<PathBuf>,
    ) -> Result<Self> {
        ensure!(address.ip().is_loopback(), "admin must bind to loopback");
        Config::load(&config)?;
        let config = std::fs::canonicalize(config).context("resolve admin config")?;
        let listener = TcpListener::bind(address).await?;
        let address = listener.local_addr()?;
        let state = Arc::new(State {
            address,
            config,
            log_file,
            started: Instant::now(),
            logs: Mutex::new(VecDeque::new()),
            config_lock: Mutex::new(()),
        });
        state
            .record("INFO", format!("管理面板已就绪 · http://{address}"))
            .await;
        state
            .record("INFO", "配置文件已加载并通过校验".into())
            .await;
        if state.log_file.is_none() {
            state
                .record(
                    "INFO",
                    "当前显示面板操作日志；使用 --log-file 接入服务端日志".into(),
                )
                .await;
        }
        Ok(Self { listener, state })
    }

    pub fn address(&self) -> SocketAddr {
        self.state.address
    }

    pub async fn run(self) -> Result<()> {
        let limit = Arc::new(Semaphore::new(32));
        let mut tasks = tokio::task::JoinSet::new();
        let shutdown = tokio::signal::ctrl_c();
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                result = &mut shutdown => { result?; break; }
                Some(_) = tasks.join_next(), if !tasks.is_empty() => {}
                incoming = self.listener.accept() => {
                    let (stream, _) = incoming?;
                    let Ok(permit) = limit.clone().try_acquire_owned() else { continue };
                    let state = self.state.clone();
                    tasks.spawn(async move {
                        let _permit = permit;
                        let _ = tokio::time::timeout(Duration::from_secs(5), handle(stream, state)).await;
                    });
                }
            }
        }
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
        Ok(())
    }
}

impl State {
    async fn record(&self, level: &'static str, message: String) {
        let mut logs = self.logs.lock().await;
        if logs.len() == 200 {
            logs.pop_front();
        }
        logs.push_back(LogEntry {
            time: now(),
            level,
            source: "admin",
            message,
        });
    }

    async fn config_snapshot(&self) -> Result<Value> {
        let content = read_config(&self.config).await?;
        let result = validate_config(&content);
        Ok(json!({
            "content": content, "revision": revision(content.as_bytes()),
            "path": self.config, "valid": result.is_ok(),
            "values": result.as_ref().ok(),
            "error": result.err().map(|error| error.to_string()),
        }))
    }

    async fn status(&self) -> Result<Value> {
        let config = self.config_snapshot().await?;
        let parsed = validate_config(config["content"].as_str().unwrap_or_default());
        Ok(json!({
            "version": env!("CARGO_PKG_VERSION"), "client_version": "3.3.3.3",
            "address": self.address, "uptime_seconds": self.started.elapsed().as_secs(),
            "config_path": self.config, "revision": config["revision"],
            "config_valid": config["valid"], "config_error": config["error"],
            "listeners": parsed.map(|value| value.listeners).unwrap_or_default(),
            "log_file": self.log_file, "time": now(),
        }))
    }

    async fn logs(&self) -> Value {
        let mut lines = Vec::new();
        let mut warning = None;
        if let Some(path) = &self.log_file {
            match read_tail(path).await {
                Ok(tail) => lines = tail
                    .lines()
                    .rev()
                    .take(500)
                    .map(|line| {
                        let level = if line.contains("ERROR") {
                            "ERROR"
                        } else if line.contains("WARN") {
                            "WARN"
                        } else if line.contains("DEBUG") {
                            "DEBUG"
                        } else if line.contains("TRACE") {
                            "TRACE"
                        } else {
                            "INFO"
                        };
                        json!({"time": null, "level": level, "source": "server", "message": line})
                    })
                    .collect(),
                Err(error) => warning = Some(format!("读取服务端日志失败：{error}")),
            }
            lines.reverse();
        }
        lines.extend(self.logs.lock().await.iter().map(|entry| json!(entry)));
        json!({"entries": lines, "warning": warning, "file": self.log_file})
    }

    async fn command(&self, command: &str) -> Result<Value> {
        let command = command.trim();
        ensure!(command.len() <= 256, "command exceeds 256 bytes");
        let (ok, output) = match command {
            "help" | "" => (true, "CODOL 服务控制台\n\nhelp          显示可用命令\nstatus        查看面板运行状态\nlisteners     查看配置中的监听端点\ncheck-config  校验当前配置\nshow-config   显示当前 TOML 配置\ntail          查看最近 20 条日志\nclear         清空当前终端视图\n\n配置保存后需由原启动流程重新启动观察服务才能生效。".into()),
            "status" => (true, format!("CODOL Control v{}\n管理地址  http://{}\n面板运行  {}s\n配置路径  {}\n日志来源  {}", env!("CARGO_PKG_VERSION"), self.address, self.started.elapsed().as_secs(), self.config.display(), self.log_file.as_ref().map(|path| path.display().to_string()).unwrap_or("面板操作日志".into()))),
            "listeners" => {
                let config = validate_config(&read_config(&self.config).await?)?;
                (true, config.listeners.iter().map(|entry| format!("{:<20} {}  [已配置]", entry.name, entry.bind)).collect::<Vec<_>>().join("\n"))
            }
            "check-config" => match validate_config(&read_config(&self.config).await?) {
                Ok(config) => (true, format!("配置校验通过\n{} 个监听端点 · 最大 {} 个连接\n{}", config.listeners.len(), config.max_connections, self.config.display())),
                Err(error) => (false, format!("配置校验失败：{error}")),
            },
            "show-config" => (true, read_config(&self.config).await?),
            "tail" => {
                let logs = self.logs().await;
                let entries = logs["entries"].as_array().context("log entries missing")?;
                let output = entries.iter().rev().take(20).rev().map(|entry| format!("[{}] {}", entry["level"].as_str().unwrap_or("INFO"), entry["message"].as_str().unwrap_or_default())).collect::<Vec<_>>().join("\n");
                (true, logs["warning"].as_str().map(|warning| format!("{warning}\n{output}")).unwrap_or(output))
            }
            "clear" => return Ok(json!({"ok": true, "clear": true, "output": ""})),
            _ => (false, format!("未知命令：{command}\n输入 help 查看可用命令。")),
        };
        self.record(
            if ok { "INFO" } else { "WARN" },
            format!("终端执行 · {command}"),
        )
        .await;
        Ok(json!({"ok": ok, "output": output}))
    }

    async fn save(&self, content: &str, expected_revision: &str) -> Result<Value> {
        validate_config(content)?;
        let _guard = self.config_lock.lock().await;
        let original = read_config(&self.config).await?;
        if revision(original.as_bytes()) != expected_revision {
            return Ok(
                json!({"conflict": true, "error": "配置已被其它操作修改，请重新加载后再保存。"}),
            );
        }
        if original == content {
            return Ok(json!({"revision": expected_revision, "unchanged": true}));
        }
        let suffix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let backup = self.config.with_extension(format!("toml.backup-{suffix}"));
        let temporary = self.config.with_extension(format!("toml.tmp-{suffix}"));
        let permissions = tokio::fs::metadata(&self.config).await?.permissions();
        write_new(&backup, original.as_bytes(), permissions.clone()).await?;
        if let Err(error) = write_new(&temporary, content.as_bytes(), permissions).await {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(error);
        }
        if let Err(error) = tokio::fs::rename(&temporary, &self.config).await {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(error.into());
        }
        self.record(
            "INFO",
            "配置已保存，原文件备份完成；等待观察服务重新启动".into(),
        )
        .await;
        Ok(
            json!({"revision": revision(content.as_bytes()), "backup": backup, "restart_required": true}),
        )
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn revision(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn validate_config(content: &str) -> Result<Config> {
    ensure!(content.len() <= MAX_BODY, "config exceeds 64 KiB");
    let config: Config = toml::from_str(content).context("TOML 格式错误")?;
    config.validate()?;
    Ok(config)
}

async fn read_config(path: &Path) -> Result<String> {
    let file = tokio::fs::File::open(path).await?;
    let mut bytes = Vec::new();
    file.take((MAX_BODY + 1) as u64)
        .read_to_end(&mut bytes)
        .await?;
    ensure!(bytes.len() <= MAX_BODY, "config exceeds 64 KiB");
    Ok(String::from_utf8(bytes)?)
}

async fn write_new(path: &Path, bytes: &[u8], permissions: std::fs::Permissions) -> Result<()> {
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path).await?;
    file.set_permissions(permissions).await?;
    file.write_all(bytes).await?;
    file.sync_all().await?;
    Ok(())
}

async fn read_tail(path: &Path) -> Result<String> {
    ensure!(
        tokio::fs::metadata(path).await?.is_file(),
        "log path must be a regular file"
    );
    let mut file = tokio::fs::File::open(path).await?;
    let size = file.metadata().await?.len();
    let offset = size.saturating_sub(LOG_TAIL_BYTES);
    file.seek(std::io::SeekFrom::Start(offset)).await?;
    let mut bytes = Vec::new();
    file.take(LOG_TAIL_BYTES).read_to_end(&mut bytes).await?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Ok(if offset > 0 {
        text.split_once('\n')
            .map(|(_, rest)| rest.to_owned())
            .unwrap_or_default()
    } else {
        text
    })
}

struct Request {
    method: String,
    path: String,
    host: String,
    origin: Option<String>,
    json: bool,
    body: Vec<u8>,
}

async fn read_request(stream: &mut TcpStream) -> Result<Request> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 2048];
    let end = loop {
        if let Some(position) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break position + 4;
        }
        ensure!(bytes.len() < MAX_HEADERS, "header limit exceeded");
        let available = buffer.len().min(MAX_HEADERS - bytes.len());
        let count = stream.read(&mut buffer[..available]).await?;
        ensure!(count > 0, "truncated request");
        bytes.extend_from_slice(&buffer[..count]);
    };
    let mut headers = [httparse::EMPTY_HEADER; 32];
    let mut parsed = httparse::Request::new(&mut headers);
    ensure!(
        parsed.parse(&bytes[..end])?.is_complete(),
        "invalid request"
    );
    ensure!(parsed.version == Some(1), "HTTP/1.1 required");
    let method = parsed.method.context("method missing")?.to_owned();
    let path = parsed.path.context("path missing")?.to_owned();
    let mut host = None;
    let mut origin = None;
    let mut length = None;
    let mut json = false;
    for header in parsed.headers {
        let value = std::str::from_utf8(header.value)?.trim();
        match header.name.to_ascii_lowercase().as_str() {
            "host" => {
                ensure!(host.is_none(), "duplicate host");
                host = Some(value.to_owned());
            }
            "origin" => {
                ensure!(origin.is_none(), "duplicate origin");
                origin = Some(value.to_owned());
            }
            "content-length" => {
                ensure!(
                    length.is_none()
                        && !value.is_empty()
                        && value.bytes().all(|byte| byte.is_ascii_digit()),
                    "invalid content length"
                );
                let size: usize = value.parse()?;
                ensure!(size <= MAX_BODY, "body limit exceeded");
                length = Some(size);
            }
            "transfer-encoding" | "expect" => anyhow::bail!("unsupported request framing"),
            "content-type" => json = value.split(';').next() == Some("application/json"),
            _ => {}
        }
    }
    let length = length.unwrap_or(0);
    ensure!(bytes.len() <= end + length, "extra request bytes");
    while bytes.len() < end + length {
        let available = buffer.len().min(end + length - bytes.len());
        let count = stream.read(&mut buffer[..available]).await?;
        ensure!(count > 0, "truncated body");
        bytes.extend_from_slice(&buffer[..count]);
    }
    Ok(Request {
        method,
        path,
        host: host.context("host missing")?,
        origin,
        json,
        body: bytes[end..].to_vec(),
    })
}

fn trusted_request(request: &Request, address: SocketAddr) -> bool {
    let valid_host = request.host == address.to_string()
        || request.host == format!("localhost:{}", address.port());
    valid_host
        && request
            .origin
            .as_ref()
            .is_none_or(|origin| origin == &format!("http://{}", request.host))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigEdit {
    content: String,
    revision: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandInput {
    command: String,
}

async fn handle(mut stream: TcpStream, state: Arc<State>) -> Result<()> {
    let request = match read_request(&mut stream).await {
        Ok(request) => request,
        Err(_) => {
            return respond_json(&mut stream, 400, json!({"error": "请求格式无效或超出限制"}))
                .await;
        }
    };
    if !trusted_request(&request, state.address) {
        return respond_json(&mut stream, 403, json!({"error": "仅允许本地同源访问"})).await;
    }
    if request.method == "GET" {
        let asset = match request.path.as_str() {
            "/" => Some(("text/html; charset=utf-8", HTML)),
            "/style.css" => Some(("text/css; charset=utf-8", CSS)),
            "/app.js" => Some(("text/javascript; charset=utf-8", JS)),
            _ => None,
        };
        if let Some((kind, body)) = asset {
            return respond(&mut stream, 200, kind, body.as_bytes()).await;
        }
    }
    let result: Result<Value> = match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/api/status") => state.status().await,
        ("GET", "/api/logs") => Ok(state.logs().await),
        ("GET", "/api/config") => state.config_snapshot().await,
        ("POST", _) if !request.json => {
            return respond_json(&mut stream, 415, json!({"error": "需要 application/json"})).await;
        }
        ("POST", "/api/terminal") => match serde_json::from_slice::<CommandInput>(&request.body) {
            Ok(input) => state.command(&input.command).await,
            Err(error) => Err(error.into()),
        },
        ("POST", "/api/config/validate") => {
            match serde_json::from_slice::<ConfigEdit>(&request.body) {
                Ok(input) => validate_config(&input.content)
                    .map(|config| json!({"valid": true, "listeners": config.listeners.len()})),
                Err(error) => Err(error.into()),
            }
        }
        ("POST", "/api/config") => match serde_json::from_slice::<ConfigEdit>(&request.body) {
            Ok(input) => match input.revision {
                Some(revision) => state.save(&input.content, &revision).await,
                None => Err(anyhow::anyhow!("保存配置需要原文件版本")),
            },
            Err(error) => Err(error.into()),
        },
        _ => return respond_json(&mut stream, 404, json!({"error": "页面或接口不存在"})).await,
    };
    match result {
        Ok(value) => {
            respond_json(
                &mut stream,
                if value["conflict"] == true { 409 } else { 200 },
                value,
            )
            .await
        }
        Err(error) => respond_json(&mut stream, 422, json!({"error": format!("{error:#}")})).await,
    }
}

async fn respond_json(stream: &mut TcpStream, status: u16, value: Value) -> Result<()> {
    respond(
        stream,
        status,
        "application/json; charset=utf-8",
        &serde_json::to_vec(&value)?,
    )
    .await
}

async fn respond(stream: &mut TcpStream, status: u16, kind: &str, body: &[u8]) -> Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        415 => "Unsupported Media Type",
        _ => "Unprocessable Content",
    };
    stream.write_all(format!("HTTP/1.1 {status} {reason}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'\r\n\r\n", body.len()).as_bytes()).await?;
    stream.write_all(body).await?;
    stream.shutdown().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../configs/local.toml"
    ));

    async fn setup(log: Option<PathBuf>) -> (tempfile::TempDir, AdminServer) {
        let directory = tempfile::tempdir().unwrap();
        let config = directory.path().join("local.toml");
        tokio::fs::write(&config, CONFIG).await.unwrap();
        let server = AdminServer::bind("127.0.0.1:0".parse().unwrap(), config, log)
            .await
            .unwrap();
        (directory, server)
    }

    async fn exchange(server: &AdminServer, request: String) -> (u16, String) {
        let serve = async {
            let (stream, _) = server.listener.accept().await.unwrap();
            handle(stream, server.state.clone()).await.unwrap();
        };
        let client = async {
            let mut stream = TcpStream::connect(server.address()).await.unwrap();
            stream.write_all(request.as_bytes()).await.unwrap();
            let mut response = Vec::new();
            stream.read_to_end(&mut response).await.unwrap();
            let response = String::from_utf8(response).unwrap();
            let status = response.split_whitespace().nth(1).unwrap().parse().unwrap();
            (
                status,
                response.split_once("\r\n\r\n").unwrap().1.to_owned(),
            )
        };
        let (_, result) = tokio::join!(serve, client);
        result
    }

    async fn api(server: &AdminServer, method: &str, path: &str, body: Value) -> (u16, Value) {
        let body = if method == "GET" {
            String::new()
        } else {
            body.to_string()
        };
        let request = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            server.address(),
            body.len()
        );
        let (status, body) = exchange(server, request).await;
        (status, serde_json::from_str(&body).unwrap())
    }

    #[tokio::test]
    async fn config_save_validates_backs_up_and_rejects_stale_edits() {
        let (_directory, server) = setup(None).await;
        let (status, original) = api(&server, "GET", "/api/config", Value::Null).await;
        assert_eq!(status, 200);
        assert_eq!(original["content"], CONFIG);
        let invalid = CONFIG.replace("max_connections = 16", "max_connections = 0");
        let (status, _) = api(
            &server,
            "POST",
            "/api/config",
            json!({"content": invalid, "revision": original["revision"]}),
        )
        .await;
        assert_eq!(status, 422);
        assert_eq!(read_config(&server.state.config).await.unwrap(), CONFIG);
        let changed = CONFIG.replace("max_connections = 16", "max_connections = 24");
        let (status, saved) = api(
            &server,
            "POST",
            "/api/config",
            json!({"content": changed, "revision": original["revision"]}),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(read_config(&server.state.config).await.unwrap(), changed);
        assert_eq!(
            tokio::fs::read_to_string(saved["backup"].as_str().unwrap())
                .await
                .unwrap(),
            CONFIG
        );
        assert_eq!(saved["restart_required"], true);
        let (status, _) = api(
            &server,
            "POST",
            "/api/config",
            json!({"content": CONFIG, "revision": original["revision"]}),
        )
        .await;
        assert_eq!(status, 409);
        assert_eq!(read_config(&server.state.config).await.unwrap(), changed);
        let (status, unchanged) = api(
            &server,
            "POST",
            "/api/config",
            json!({"content": changed, "revision": saved["revision"]}),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(unchanged["unchanged"], true);
    }

    #[tokio::test]
    async fn local_host_and_origin_are_required() {
        let (_directory, server) = setup(None).await;
        let spoof = format!(
            "GET /api/config HTTP/1.1\r\nHost: evil.example:{}\r\n\r\n",
            server.address().port()
        );
        assert_eq!(exchange(&server, spoof).await.0, 403);
        let cross_origin = format!(
            "POST /api/config HTTP/1.1\r\nHost: {}\r\nOrigin: https://evil.example\r\nContent-Length: 0\r\n\r\n",
            server.address()
        );
        assert_eq!(exchange(&server, cross_origin).await.0, 403);
        let form = format!(
            "POST /api/config HTTP/1.1\r\nHost: {}\r\nContent-Type: text/plain\r\nContent-Length: 0\r\n\r\n",
            server.address()
        );
        assert_eq!(exchange(&server, form).await.0, 415);
        let duplicate_length = format!(
            "POST /api/config HTTP/1.1\r\nHost: {}\r\nContent-Length: 0\r\nContent-Length: 0\r\n\r\n",
            server.address()
        );
        assert_eq!(exchange(&server, duplicate_length).await.0, 400);
        let oversized = format!(
            "POST /api/config HTTP/1.1\r\nHost: {}\r\nContent-Length: {}\r\n\r\n",
            server.address(),
            MAX_BODY + 1
        );
        assert_eq!(exchange(&server, oversized).await.0, 400);
        assert_eq!(read_config(&server.state.config).await.unwrap(), CONFIG);
        assert!(
            AdminServer::bind(
                "0.0.0.0:0".parse().unwrap(),
                server.state.config.clone(),
                None
            )
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn terminal_runs_only_builtin_diagnostics_and_returns_errors() {
        let (_directory, server) = setup(None).await;
        let (status, checked) = api(
            &server,
            "POST",
            "/api/terminal",
            json!({"command":"check-config"}),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(checked["ok"], true);
        assert!(checked["output"].as_str().unwrap().contains("配置校验通过"));
        let (_, rejected) = api(
            &server,
            "POST",
            "/api/terminal",
            json!({"command":"status; echo injected"}),
        )
        .await;
        assert_eq!(rejected["ok"], false);
        let (_, status) = api(&server, "GET", "/api/status", Value::Null).await;
        assert_eq!(status["listeners"].as_array().unwrap().len(), 2);
        assert_eq!(status["config_valid"], true);
        let (_, logs) = api(&server, "GET", "/api/logs", Value::Null).await;
        assert!(
            logs["entries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["level"] == "WARN")
        );
    }

    #[tokio::test]
    async fn log_tail_is_bounded_and_follows_append_truncate_and_missing_files() {
        let logs = tempfile::tempdir().unwrap();
        let path = logs.path().join("server.log");
        let content = (0..1200)
            .map(|index| format!("INFO event {index}\n"))
            .collect::<String>();
        tokio::fs::write(&path, content).await.unwrap();
        let (_directory, server) = setup(Some(path.clone())).await;
        let (_, result) = api(&server, "GET", "/api/logs", Value::Null).await;
        let external: Vec<_> = result["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["source"] == "server")
            .collect();
        assert_eq!(external.len(), 500);
        assert_eq!(external[0]["message"], "INFO event 700");
        let mut file = tokio::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .await
            .unwrap();
        file.write_all(b"ERROR new event\n").await.unwrap();
        file.flush().await.unwrap();
        let (_, result) = api(&server, "GET", "/api/logs", Value::Null).await;
        assert!(
            result["entries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["message"] == "ERROR new event" && entry["level"] == "ERROR")
        );
        tokio::fs::write(&path, "WARN rotated\n").await.unwrap();
        let (_, result) = api(&server, "GET", "/api/logs", Value::Null).await;
        assert_eq!(result["entries"][0]["message"], "WARN rotated");
        tokio::fs::remove_file(&path).await.unwrap();
        let (_, result) = api(&server, "GET", "/api/logs", Value::Null).await;
        assert!(result["warning"].is_string());
        assert!(!result["entries"].as_array().unwrap().is_empty());
    }
}
