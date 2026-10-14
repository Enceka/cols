//! In-process OpenSSL transport for the observed legacy AUTH endpoint.
use crate::backend::identity::session::Session;
use crate::backend::identity::session::SessionRegistry;
use crate::infra::capture::unix_time_ms;
use crate::infra::storage::write_private;
use crate::transport::http;
use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use codol_protocol::auth::request::Task10Request;
use codol_protocol::auth::request::local_ticket_response;
use codol_protocol::auth::request::parse_task10;
use codol_protocol::auth::ticket::Ticket;
use openssl::ssl::{
    Ssl, SslContext, SslContextBuilder, SslFiletype, SslMethod, SslOptions, SslVersion,
};
use serde_json::json;
use std::{
    future::Future,
    net::SocketAddr,
    path::{Path, PathBuf},
    pin::Pin,
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream},
    sync::{Semaphore, watch},
    task::JoinSet,
};
use tokio_openssl::SslStream;

#[derive(Clone, Debug)]
pub struct AuthOptions {
    pub bind: SocketAddr,
    pub certificate: PathBuf,
    pub key: PathBuf,
    pub capture_dir: PathBuf,
    /// A bounded JSON fixture for response experiments; absence means HTTP 501.
    pub reply_file: Option<PathBuf>,
    /// Issue local test-account tickets for this title only. Not QQ authentication.
    pub local_title_id: Option<u32>,
}

pub struct AuthServer {
    listener: TcpListener,
    context: SslContext,
    directory: PathBuf,
    reply: Option<Arc<Vec<u8>>>,
    local_title_id: Option<u32>,
    registry: Arc<SessionRegistry>,
}

impl AuthServer {
    pub async fn bind(options: AuthOptions) -> Result<Self> {
        ensure!(
            options.bind.ip().is_loopback(),
            "legacy AUTH experiments must bind loopback"
        );
        ensure!(
            options.reply_file.is_none() || options.local_title_id.is_none(),
            "select fixture replies or local tickets"
        );
        let mut context = SslContextBuilder::new(SslMethod::tls_server())?;
        context.set_min_proto_version(Some(SslVersion::TLS1))?;
        context.set_max_proto_version(Some(SslVersion::TLS1))?;
        context.set_security_level(0);
        context.set_cipher_list("AES128-SHA")?;
        context.set_options(SslOptions::NO_COMPRESSION | SslOptions::NO_TICKET);
        context
            .set_certificate_chain_file(&options.certificate)
            .context("load AUTH certificate")?;
        context
            .set_private_key_file(&options.key, SslFiletype::PEM)
            .context("load AUTH key")?;
        context.check_private_key()?;
        let reply = match &options.reply_file {
            Some(path) => {
                ensure!(
                    std::fs::metadata(path)?.len() <= 65536,
                    "reply fixture exceeds limit"
                );
                let bytes = std::fs::read(path)?;
                serde_json::from_slice::<serde_json::Value>(&bytes)
                    .context("reply fixture must be JSON")?;
                Some(Arc::new(bytes))
            }
            None => None,
        };
        let listener = TcpListener::bind(options.bind).await?;
        tokio::fs::create_dir_all(&options.capture_dir).await?;
        let directory =
            options
                .capture_dir
                .join(format!("auth-{}-{}", unix_time_ms(), std::process::id()));
        tokio::fs::create_dir(&directory).await?;
        write_private(&directory.join("run.json"), &serde_json::to_vec_pretty(&json!({
            "schema_version": 1, "mode": "rust_legacy_auth",
            "bind": listener.local_addr()?, "openssl": openssl::version::version(),
            "reply_mode": if options.local_title_id.is_some() { "local_test_tickets" } else if reply.is_some() { "experimental_fixture" } else { "record_and_501" },
            "local_title_id":options.local_title_id,
            "target_client_sha256": codol_protocol::CLIENT_SHA256,
            "max_connections": 16, "max_sessions": 128, "session_timeout_seconds": 30,
        }))?).await?;
        Ok(Self {
            listener,
            context: context.build(),
            directory,
            reply,
            local_title_id: options.local_title_id,
            registry: Arc::new(SessionRegistry::default()),
        })
    }

    pub fn address(&self) -> Result<SocketAddr> {
        Ok(self.listener.local_addr()?)
    }
    pub async fn with_local_players(self, root: Option<&Path>) -> Result<Self> {
        if let (Some(root), Some(title)) = (root, self.local_title_id) {
            let path = root.join("profiles").join(title.to_string());
            if path.exists() {
                let mut entries = tokio::fs::read_dir(path).await?;
                while let Some(entry) = entries.next_entry().await? {
                    let Ok(user_id) = entry.file_name().to_string_lossy().parse::<u64>() else {
                        continue;
                    };
                    if let Some(name) =
                        crate::backend::players::nickname(root, title, user_id).await?
                    {
                        self.registry.set_display_name(title, user_id, name)?;
                    }
                }
            }
        }
        Ok(self)
    }
    pub fn directory(&self) -> &Path {
        &self.directory
    }
    pub fn registry(&self) -> Arc<SessionRegistry> {
        self.registry.clone()
    }

    pub async fn run<F: Future<Output = std::io::Result<()>>>(self, shutdown: F) -> Result<()> {
        let (stop, _) = watch::channel(false);
        let slots = Arc::new(Semaphore::new(16));
        let mut sessions = JoinSet::new();
        let mut next_id = 0u64;
        tokio::pin!(shutdown);
        let outcome = loop {
            tokio::select! {
                biased;
                result = &mut shutdown => break result.context("shutdown signal"),
                done = sessions.join_next(), if !sessions.is_empty() => {
                    if let Some(Err(error)) = done { tracing::warn!(%error, "AUTH session task failed"); }
                }
                accepted = self.listener.accept(), if next_id < 128 => {
                    let (stream, peer) = match accepted { Ok(value) => value, Err(error) => break Err(error.into()) };
                    let Ok(permit) = slots.clone().try_acquire_owned() else { continue; };
                    next_id += 1;
                    let id = next_id;
                    let context = self.context.clone();
                    let directory = self.directory.clone();
                    let reply = self.reply.clone();
                    let local_title_id = self.local_title_id;
                    let registry=self.registry.clone();
                    let mut stop = stop.subscribe();
                    sessions.spawn(async move {
                        let _permit = permit;
                        let work = handle(stream, id, &context, &directory, reply.as_deref().map(Vec::as_slice),local_title_id,&registry);
                        let result = if *stop.borrow() { Err(anyhow::anyhow!("shutdown")) } else {
                            tokio::select! {
                                _ = stop.changed() => Err(anyhow::anyhow!("shutdown")),
                                result = tokio::time::timeout(Duration::from_secs(30), work) => {
                                    match result { Ok(result) => result, Err(_) => Err(anyhow::anyhow!("AUTH session timeout")) }
                                }
                            }
                        };
                        if let Err(error) = result {
                            tracing::warn!(session_id=id, %error, "AUTH session ended without response completion");
                            let metadata = json!({"session_id":id,"peer":peer,"completed":false,"error":error.to_string()});
                            let _ = write_private(&directory.join(format!("{id:06}-error.json")), &serde_json::to_vec(&metadata).unwrap_or_default()).await;
                        }
                    });
                }
            }
        };
        stop.send_replace(true);
        while let Some(result) = sessions.join_next().await {
            if let Err(error) = result {
                tracing::warn!(%error, "AUTH shutdown task failed");
            }
        }
        outcome
    }
}

async fn handle(
    stream: TcpStream,
    id: u64,
    context: &SslContext,
    directory: &Path,
    reply: Option<&[u8]>,
    local_title_id: Option<u32>,
    registry: &SessionRegistry,
) -> Result<()> {
    let peer = stream.peer_addr()?;
    let mut stream = SslStream::new(Ssl::new(context)?, stream)?;
    tokio::time::timeout(Duration::from_secs(10), Pin::new(&mut stream).accept())
        .await
        .context("TLS handshake timeout")??;
    tracing::info!(
        session_id = id,
        version = stream.ssl().version_str(),
        cipher = stream.ssl().current_cipher().map(|c| c.name()),
        "AUTH TLS established"
    );
    let request = match http::read_auth(&mut stream).await {
        Ok(request) => request,
        Err(error) => {
            http::write_response(
                &mut stream,
                error.status,
                br#"{"error":"invalid HTTP request"}"#,
            )
            .await?;
            let _ = tokio::time::timeout(Duration::from_secs(1), Pin::new(&mut stream).shutdown())
                .await;
            return Err(error.into());
        }
    };
    write_private(
        &directory.join(format!("{id:06}-request.http")),
        &request.wire,
    )
    .await?;
    write_private(
        &directory.join(format!("{id:06}-request.json")),
        &request.body,
    )
    .await?;
    let parsed = parse_task10(&request.body);
    let fallback = br#"{"error":"authentication response is not implemented"}"#;
    let invalid = br#"{"error":"unsupported or malformed AUTH task"}"#;
    let generated = if let (Some(title), Ok(auth)) = (local_title_id, &parsed) {
        if auth.title_id == title {
            Some(issue_local(auth, id, directory, registry).await?)
        } else {
            None
        }
    } else {
        None
    };
    let (status, body) = match &parsed {
        Ok(auth) if local_title_id.is_some() && local_title_id != Some(auth.title_id) => {
            (400, b"{\"error\":\"unexpected title_id\"}".as_slice())
        }
        Ok(_) if generated.is_some() => (200, generated.as_deref().unwrap_or_default()),
        Ok(_) => match reply {
            Some(bytes) => (200, bytes),
            None => (501, fallback.as_slice()),
        },
        Err(_) => (400, invalid.as_slice()),
    };
    write_private(&directory.join(format!("{id:06}-response.json")), body).await?;
    http::write_response(&mut stream, status, body).await?;
    let title_id = parsed.as_ref().ok().map(|request| request.title_id);
    let metadata = json!({
        "session_id":id, "peer":peer, "completed":true, "http_status":status,
        "transaction_id":request.transaction_id, "title_id":title_id,
        "body_bytes":request.body.len(), "response_bytes":body.len(),
        "tls_version":stream.ssl().version_str(), "cipher":stream.ssl().current_cipher().map(|c|c.name()),
        "auth_parse_error":parsed.as_ref().err().map(|e|e.to_string()),
    });
    write_private(
        &directory.join(format!("{id:06}-session.json")),
        &serde_json::to_vec_pretty(&metadata)?,
    )
    .await?;
    // Persist completion before close_notify makes EOF visible to the peer.
    let _ = tokio::time::timeout(Duration::from_secs(1), Pin::new(&mut stream).shutdown()).await;
    tracing::info!(session_id = id, status, title_id, "AUTH request processed");
    Ok(())
}

async fn issue_local(
    request: &Task10Request,
    id: u64,
    directory: &Path,
    registry: &SessionRegistry,
) -> Result<Vec<u8>> {
    let issued_at = u32::try_from(unix_time_ms() / 1000).context("clock outside ticket range")?;
    let expires_at = issued_at
        .checked_add(3600)
        .context("ticket expiry overflow")?;
    let mut key = [0u8; 24];
    openssl::rand::rand_bytes(&mut key)?;
    let ticket = Ticket {
        title_id: request.title_id,
        issued_at,
        expires_at,
        license_id: u64::from(request.title_id),
        user_id: request.user_id,
        username: registry
            .display_name(request.title_id, request.user_id)?
            .map(|name| STANDARD.encode(name))
            .unwrap_or_else(|| format!("Player_{:016x}", request.user_id)),
        session_key: key,
    };
    let response = local_ticket_response(request, &ticket)?;
    let server_ticket: [u8; 128] = STANDARD
        .decode(&response.server_ticket)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("server ticket size"))?;
    registry.insert(
        server_ticket,
        Session {
            title_id: ticket.title_id,
            user_id: ticket.user_id,
            iv_seed: request.iv_seed,
            expires_at,
            key,
        },
    )?;
    // Private experiment state lets the later LSG implementation bind this
    // exact ticket to its account/key; never print the key to console logs.
    let state = json!({"schema_version":1,"title_id":ticket.title_id,
        "user_id":ticket.user_id.to_string(),"issued_at":issued_at,"expires_at":expires_at,
        "username":ticket.username,"iv_seed":request.iv_seed,
        "server_ticket":response.server_ticket,
        "session_key_hex":key.iter().map(|b|format!("{b:02x}")).collect::<String>(),
        "scope":"local_test_identity_issuance_only; see lobby handshake records for peer acceptance"});
    write_private(
        &directory.join(format!("{id:06}-local-session.json")),
        &serde_json::to_vec_pretty(&state)?,
    )
    .await?;
    Ok(serde_json::to_vec(&response)?)
}
