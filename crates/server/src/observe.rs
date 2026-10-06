use crate::{
    capture::{Session, record_tcp, unix_time_ms},
    config::Config,
};
use anyhow::{Context, Result, anyhow};
use serde_json::json;
use std::{
    future::Future,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::{
    net::TcpListener,
    sync::{Semaphore, watch},
    task::JoinSet,
};

pub struct Observer {
    config: Arc<Config>,
    listeners: Vec<(String, TcpListener)>,
    directory: PathBuf,
}

impl Observer {
    pub async fn bind(config: Config) -> Result<Self> {
        config.validate()?;
        let mut listeners = Vec::new();
        // Bind everything before creating a run. Partial binding failures drop
        // already-bound sockets without leaving a half-running service.
        for endpoint in &config.listeners {
            let listener = TcpListener::bind(endpoint.bind)
                .await
                .with_context(|| format!("bind {} at {}", endpoint.name, endpoint.bind))?;
            listeners.push((endpoint.name.clone(), listener));
        }
        tokio::fs::create_dir_all(&config.capture_dir).await?;
        let directory =
            config
                .capture_dir
                .join(format!("run-{}-{}", unix_time_ms(), std::process::id()));
        // Never reuse a run directory or overwrite a previous capture.
        tokio::fs::create_dir(&directory).await?;
        let observer = Self {
            config: Arc::new(config),
            listeners,
            directory,
        };
        let metadata = json!({
            "schema_version": 1,
            "mode": "tcp_observe_no_response",
            "target_client_sha256": codol_protocol::CLIENT_SHA256,
            "client_identity_verified_on_wire": false,
            "started_unix_time_ms": unix_time_ms(),
            "listeners": observer.addresses()?,
            "config": observer.config.as_ref(),
        });
        tokio::fs::write(
            observer.directory.join("run.json"),
            serde_json::to_vec_pretty(&metadata)?,
        )
        .await?;
        Ok(observer)
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn addresses(&self) -> Result<Vec<(String, SocketAddr)>> {
        self.listeners
            .iter()
            .map(|(name, listener)| Ok((name.clone(), listener.local_addr()?)))
            .collect()
    }

    pub async fn run<F>(self, shutdown: F) -> Result<()>
    where
        F: Future<Output = std::io::Result<()>>,
    {
        let (stop, _) = watch::channel(false);
        let slots = Arc::new(Semaphore::new(self.config.max_connections));
        let sessions = Arc::new(AtomicU64::new(0));
        let mut tasks = JoinSet::new();
        for (name, listener) in self.listeners {
            let context = ListenerContext {
                name,
                config: self.config.clone(),
                directory: self.directory.clone(),
                slots: slots.clone(),
                sessions: sessions.clone(),
                stop: stop.clone(),
            };
            tasks.spawn(serve_listener(listener, context));
        }
        tokio::pin!(shutdown);
        let mut outcome = tokio::select! {
            result = &mut shutdown => result.context("wait for shutdown signal"),
            result = tasks.join_next() => flatten(result),
        };
        // Preserve the signal even if a spawned listener has not subscribed yet.
        stop.send_replace(true);
        // Cooperative shutdown lets every session flush bytes and its end event.
        while let Some(result) = tasks.join_next().await {
            if let Err(error) = flatten(Some(result))
                && outcome.is_ok()
            {
                outcome = Err(error);
            }
        }
        outcome
    }
}

fn flatten(result: Option<Result<Result<()>, tokio::task::JoinError>>) -> Result<()> {
    result
        .ok_or_else(|| anyhow!("listener task set unexpectedly empty"))?
        .context("listener task failed")?
}

struct ListenerContext {
    name: String,
    config: Arc<Config>,
    directory: PathBuf,
    slots: Arc<Semaphore>,
    sessions: Arc<AtomicU64>,
    stop: watch::Sender<bool>,
}

async fn serve_listener(listener: TcpListener, context: ListenerContext) -> Result<()> {
    let mut stop = context.stop.subscribe();
    let mut clients = JoinSet::new();
    let mut outcome = Ok(());
    loop {
        if *stop.borrow() {
            break;
        }
        tokio::select! {
            biased;
            _ = stop.changed() => break,
            completed = clients.join_next(), if !clients.is_empty() => {
                if let Err(error) = flatten(completed) {
                    outcome = Err(error);
                    break;
                }
            }
            accepted = listener.accept(), if context.sessions.load(Ordering::Relaxed) < context.config.max_sessions_per_run => {
                let (stream, peer) = match accepted {
                    Ok(value) => value,
                    Err(error) => { outcome = Err(error.into()); break; }
                };
                let Ok(permit) = context.slots.clone().try_acquire_owned() else {
                    tracing::debug!(%peer, "connection rejected: concurrent session limit");
                    continue;
                };
                let Ok(previous) = context.sessions.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                    (n < context.config.max_sessions_per_run).then_some(n + 1)
                }) else { continue; };
                let config = context.config.clone();
                let directory = context.directory.clone();
                let name = context.name.clone();
                let stop = context.stop.subscribe();
                clients.spawn(async move {
                    let _permit = permit;
                    record_tcp(stream, Session {
                        id: previous + 1, listener: &name, peer,
                        directory: &directory, config: &config,
                    }, stop).await
                });
            }
        }
    }
    if outcome.is_err() {
        context.stop.send_replace(true);
    }
    while let Some(result) = clients.join_next().await {
        if let Err(error) = flatten(Some(result)) {
            context.stop.send_replace(true);
            if outcome.is_ok() {
                outcome = Err(error);
            }
        }
    }
    outcome
}
