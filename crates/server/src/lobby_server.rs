//! Protocol-210 handshake, capture, and optional local service dispatch.
use crate::{
    auth_server::write_private,
    capture::unix_time_ms,
    services::{LocalServices, TaskResult},
    session::SessionRegistry,
};
use anyhow::{Context, Result, ensure};
use codol_protocol::{lobby, task};
use serde_json::json;
use std::{
    future::Future,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{Semaphore, watch},
    task::JoinSet,
};

pub struct LobbyServer {
    listener: TcpListener,
    registry: Arc<SessionRegistry>,
    directory: PathBuf,
    services: Option<Arc<LocalServices>>,
    continuous_play: bool,
}
// The harness permits bounded hour-long native room/UI experiments. Keep its final observation
// inside the transport budget; byte/frame/concurrency limits remain enforced.
const MAX_SESSION_SECONDS: u64 = 3630;
const MAX_SERVICE_FRAMES: usize = 1024;
async fn record_packet(enabled: bool, path: &Path, bytes: &[u8]) -> Result<()> {
    if enabled {
        write_private(path, bytes).await?;
    }
    Ok(())
}

impl LobbyServer {
    pub async fn bind(
        bind: SocketAddr,
        registry: Arc<SessionRegistry>,
        directory: PathBuf,
    ) -> Result<Self> {
        Self::bind_with_services(bind, registry, directory, None).await
    }
    pub async fn bind_with_services(
        bind: SocketAddr,
        registry: Arc<SessionRegistry>,
        directory: PathBuf,
        service_data: Option<PathBuf>,
    ) -> Result<Self> {
        ensure!(bind.ip().is_loopback(), "research lobby must bind loopback");
        let services = match service_data {
            Some(root) => Some(Arc::new(LocalServices::open(root).await?)),
            None => None,
        };
        let listener = TcpListener::bind(bind).await?;
        tokio::fs::create_dir(&directory).await?;
        write_private(&directory.join("run.json"),&serde_json::to_vec_pretty(&json!({"mode":if services.is_some(){"lobby_210_local_services"}else{"lobby_210_handshake_observe"},"bind":listener.local_addr()?,"max_sessions":128,"max_connections":16,"max_frames":if services.is_some(){MAX_SERVICE_FRAMES}else{32},"session_timeout_seconds":MAX_SESSION_SECONDS}))?).await?;
        Ok(Self {
            listener,
            registry,
            directory,
            services,
            continuous_play: false,
        })
    }
    pub fn address(&self) -> Result<SocketAddr> {
        Ok(self.listener.local_addr()?)
    }
    pub fn services(&self) -> Option<Arc<LocalServices>> {
        self.services.clone()
    }
    pub fn with_continuous_play(mut self) -> Result<Self> {
        ensure!(
            self.services.is_some(),
            "continuous play requires local services"
        );
        self.continuous_play = true;
        Ok(self)
    }
    pub async fn with_game_endpoint(
        mut self,
        endpoint: Arc<crate::game_server::GameAdmissions>,
    ) -> Result<Self> {
        let services = self
            .services
            .take()
            .context("game endpoint requires local services")?;
        let services =
            Arc::try_unwrap(services).map_err(|_| anyhow::anyhow!("services already shared"))?;
        self.services = Some(Arc::new(services.with_game_endpoint(endpoint).await));
        Ok(self)
    }
    pub async fn run<F: Future<Output = std::io::Result<()>>>(self, shutdown: F) -> Result<()> {
        if self.continuous_play {
            write_private(&self.directory.join("play-mode.json"),&serde_json::to_vec(&json!({"session_timeout_seconds":null,"capture_frame_limit":MAX_SERVICE_FRAMES,"capture_byte_limit":1024*1024}))?).await?;
        }
        let (stop, _) = watch::channel(false);
        let slots = Arc::new(Semaphore::new(16));
        let mut tasks = JoinSet::new();
        let mut id = 0u64;
        let mut room_cleanup = tokio::time::interval(Duration::from_secs(5));
        tokio::pin!(shutdown);
        let outcome = loop {
            tokio::select! {
                biased;
                result=&mut shutdown=>break result.context("lobby stop signal"),
                result=tasks.join_next(),if !tasks.is_empty()=>{if let Some(Err(error))=result{tracing::warn!(%error,"lobby task failed");}},
                _=room_cleanup.tick(),if self.services.is_some()=>{ self.services.as_ref().expect("service guard").reap_rooms((unix_time_ms()/1000).try_into()?).await; },
                accepted=self.listener.accept(),if id<128=>{
                    let (stream,peer)=match accepted{Ok(x)=>x,Err(e)=>break Err(e.into())};
                    let Ok(permit)=slots.clone().try_acquire_owned() else{continue;};
                    id+=1;let id=id;
                    let registry=self.registry.clone();let directory=self.directory.clone();let services=self.services.clone();let mut stop=stop.subscribe();let continuous_play=self.continuous_play;
                    tasks.spawn(async move {
                        let _permit=permit;
                        let result=if *stop.borrow(){Err(anyhow::anyhow!("shutdown"))}else{
                            tokio::select! {
                                _=stop.changed()=>Err(anyhow::anyhow!("shutdown")),
                                result=async {
                                    let work=handle(stream,id,&registry,&directory,services.as_deref(),continuous_play);
                                    if continuous_play {work.await} else {tokio::time::timeout(Duration::from_secs(MAX_SESSION_SECONDS),work).await.map_err(|_|anyhow::anyhow!("lobby session timeout"))?}
                                }=>result,
                            }
                        };
                        if let Err(error)=result {
                            tracing::warn!(session_id=id,%error,"lobby session failed");
                            if let Ok(data)=serde_json::to_vec(&json!({"session_id":id,"peer":peer,"error":error.to_string()})){
                                let _=write_private(&directory.join(format!("{id:06}-error.json")),&data).await;
                            }
                        }
                    });
                }
            }
        };
        stop.send_replace(true);
        while let Some(result) = tasks.join_next().await {
            if let Err(error) = result {
                tracing::warn!(%error,"lobby shutdown task failed");
            }
        }
        outcome
    }
}

async fn read_frame<R: AsyncRead + Unpin>(stream: &mut R) -> Result<Vec<u8>> {
    let mut prefix = [0; 4];
    stream.read_exact(&mut prefix).await?;
    let length = u32::from_le_bytes(prefix) as usize;
    ensure!(length <= 65536, "lobby frame exceeds research limit");
    let mut bytes = vec![0; length + 4];
    bytes[..4].copy_from_slice(&prefix);
    stream.read_exact(&mut bytes[4..]).await?;
    Ok(bytes)
}
struct Outbound {
    stream: tokio::net::tcp::OwnedWriteHalf,
    keys: [u8; 72],
    sequence: u32,
    maximum: usize,
    mailbox_count: usize,
    mailbox_bytes: usize,
}
impl Outbound {
    async fn send(&mut self, kind: u8, body: &[u8]) -> Result<Vec<u8>> {
        let mut iv = [0; 16];
        openssl::rand::rand_bytes(&mut iv)?;
        let wire = lobby::encrypt_application(
            kind,
            body,
            &self.keys,
            lobby::Direction::ServerToClient,
            self.sequence,
            iv,
        )?;
        ensure!(
            wire.len() - 4 <= self.maximum,
            "lobby reply exceeds receive limit"
        );
        self.sequence = self.sequence.wrapping_add(1);
        self.stream.write_all(&wire).await?;
        Ok(wire)
    }
}
async fn read_with_mailbox<R: AsyncRead + Unpin>(
    reader: &mut R,
    writer: &mut Outbound,
    mut subscription: Option<&mut crate::lobby_delivery::Subscription>,
    directory: &Path,
) -> Result<Vec<u8>> {
    // Keep this exact read future alive across push writes: cancelling read_exact
    // after a partial length prefix would lose bytes and corrupt the next frame.
    let incoming = tokio::time::timeout(Duration::from_secs(60), read_frame(reader));
    tokio::pin!(incoming);
    loop {
        tokio::select! {
            frame=&mut incoming => return frame.map_err(|_| anyhow::anyhow!("lobby read timeout"))?,
            body=async { match subscription.as_mut() { Some(s)=>s.next().await, None=>std::future::pending().await } } => {
                let body=body.context("lobby mailbox closed or connection replaced")?;
                let wire=writer.send(2,&body).await?;
                writer.mailbox_bytes=writer.mailbox_bytes.saturating_add(wire.len());
                record_packet(writer.mailbox_count<MAX_SERVICE_FRAMES && writer.mailbox_bytes<=1024*1024,
                    &directory.join(format!("mailbox-{:06}-s2c.bin",writer.mailbox_count)),&wire).await?;
                writer.mailbox_count=writer.mailbox_count.saturating_add(1);
            }
        }
    }
}
async fn handle(
    mut stream: TcpStream,
    id: u64,
    registry: &SessionRegistry,
    root: &Path,
    services: Option<&LocalServices>,
    continuous_play: bool,
) -> Result<()> {
    let directory = root.join(format!("{id:06}"));
    tokio::fs::create_dir(&directory).await?;
    let mut hello = [0; 28];
    tokio::time::timeout(Duration::from_secs(10), stream.read_exact(&mut hello)).await??;
    write_private(&directory.join("client-hello.bin"), &hello).await?;
    let client_limits = lobby::read_client_hello(&hello)?;
    let mut nonce = [0; 8];
    openssl::rand::rand_bytes(&mut nonce)?;
    let mut connection = [0; 8];
    openssl::rand::rand_bytes(&mut connection)?;
    let connection_id = u64::from_le_bytes(connection).max(1);
    let server_hello = lobby::server_hello(connection_id, nonce);
    write_private(&directory.join("server-hello.bin"), &server_hello).await?;
    stream.write_all(&server_hello).await?;
    let auth = tokio::time::timeout(Duration::from_secs(10), read_frame(&mut stream)).await??;
    write_private(&directory.join("client-auth.bin"), &auth).await?;
    let parsed = lobby::parse_client_auth(&auth)?;
    let mut session = registry
        .lookup(&parsed.server_ticket)?
        .context("unknown or expired issued ticket")?;
    ensure!(
        session.title_id == parsed.title_id && session.iv_seed == parsed.iv_seed,
        "ticket metadata mismatch"
    );
    let secrets = lobby::derive_secrets(&hello, &server_hello, &auth, &session.key)?;
    ensure!(
        openssl::memcmp::eq(&parsed.proof, &secrets.client_proof),
        "client handshake proof mismatch"
    );
    let ack = lobby::frame(0x83, &secrets.server_proof);
    write_private(&directory.join("server-auth.bin"), &ack).await?;
    stream.write_all(&ack).await?;
    write_private(&directory.join("handshake.json"),&serde_json::to_vec_pretty(&json!({
        "transport_authenticated":true,"title_id":session.title_id,"user_id":session.user_id.to_string(),
        "connection_id":connection_id.to_string(),"protocol":210,"completed_unix_ms":unix_time_ms(),
        "local_services":services.is_some(),"inventory_implemented":false,
    }))?).await?;
    tracing::info!(
        session_id = id,
        protocol = 210,
        "lobby handshake authenticated; observing application frames"
    );
    let mut total = 0;
    let mut expected_sequence = 1;
    let (mut reader, writer) = stream.into_split();
    let mut outgoing = Outbound {
        stream: writer,
        keys: secrets.data_keys,
        sequence: 1,
        maximum: client_limits.max_receive as usize,
        mailbox_count: 0,
        mailbox_bytes: 0,
    };
    let mut subscription = match services {
        Some(services) => Some(services.subscribe_lobby(&session).await),
        None => None,
    };
    let mut transaction_id = 1u64;
    for index in 0..if continuous_play {
        usize::MAX
    } else if services.is_some() {
        MAX_SERVICE_FRAMES
    } else {
        32
    } {
        let frame = match read_with_mailbox(
            &mut reader,
            &mut outgoing,
            subscription.as_mut(),
            &directory,
        )
        .await
        {
            Ok(frame) => frame,
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::UnexpectedEof) =>
            {
                break;
            }
            Err(error) => return Err(error),
        };
        total = usize::saturating_add(total, frame.len());
        ensure!(
            continuous_play || total <= 1024 * 1024,
            "lobby capture budget exhausted"
        );
        let recording = total <= 1024 * 1024 && index < MAX_SERVICE_FRAMES;
        record_packet(
            recording,
            &directory.join(format!("packet-{index:03}-c2s.bin")),
            &frame,
        )
        .await?;
        if frame.len() > 4 {
            let message = lobby::decrypt_application(
                &frame,
                &secrets.data_keys,
                lobby::Direction::ClientToServer,
                expected_sequence,
            )?;
            ensure!(
                registry.confirm_active(&session)?,
                "lobby login was superseded"
            );
            // New handshakes still require a live ticket. Renew room and UDP
            // leases only after this active connection's MAC/sequence checks;
            // plaintext keepalives do not extend an allocation.
            if continuous_play {
                let now = (unix_time_ms() / 1000).try_into().unwrap_or(u32::MAX);
                session.expires_at = now.saturating_add(3600);
                ensure!(
                    registry.confirm_active(&session)?,
                    "lobby login was superseded"
                );
                if let Some(services) = services {
                    services.renew_play_session(&session, now).await?;
                }
            }
            expected_sequence = expected_sequence.wrapping_add(1);
            record_packet(
                recording,
                &directory.join(format!("packet-{index:03}-message.bin")),
                &message.body,
            )
            .await?;
            let service = if message.kind == 0x86 {
                message.body.first().copied()
            } else {
                None
            };
            let task = if message.kind == 0x86 && message.body.get(1) == Some(&3) {
                message.body.get(2).copied()
            } else {
                None
            };
            record_packet(recording,&directory.join(format!("packet-{index:03}-decoded.json")),&serde_json::to_vec_pretty(&json!({"sequence":message.sequence,"kind":message.kind,"service":service,"task":task,"body_bytes":message.body.len(),"mac_verified":true}))?).await?;
            tracing::info!(
                session_id = id,
                kind = message.kind,
                service,
                task,
                "lobby application frame authenticated and decoded"
            );
            let response = if message.kind == 0x86 {
                if let Some(services) = services {
                    task::Request::parse(&message.body)?;
                    Some(
                        match services
                            .handle_connected(
                                &message.body,
                                &session,
                                (unix_time_ms() / 1000).try_into()?,
                                subscription.as_ref().expect("service subscription"),
                            )
                            .await
                        {
                            Ok(value) => value,
                            Err(error) => {
                                tracing::warn!(session_id=id, %error, "task rejected; explicit failure reply");
                                TaskResult {
                                    label: "task_rejected",
                                    result: Err(task::TASK_FAILED),
                                    pushes: Vec::new(),
                                    deliveries: Vec::new(),
                                }
                            }
                        },
                    )
                } else if service == Some(71) && task == Some(4) {
                    lobby::registration_build(&message.body)?;
                    Some(TaskResult {
                        label: "registration",
                        result: Ok(vec![]),
                        pushes: Vec::new(),
                        deliveries: Vec::new(),
                    })
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(response) = response {
                let result = match &response.result {
                    Ok(records) => Ok(records.as_slice()),
                    Err(code) => Err(*code),
                };
                let body = task::reply(transaction_id, task.context("missing task id")?, result)?;
                record_packet(recording,&directory.join(format!("packet-{index:03}-reply.json")),&serde_json::to_vec_pretty(&json!({
                    "transaction_id":transaction_id,"service":service,"task":task,"handler":response.label,
                    "error_code":response.result.as_ref().err().copied().unwrap_or(0),
                    "result_count":response.result.as_ref().map_or(0,Vec::len),
                    "push_count":response.pushes.len(),"directed_push_count":response.deliveries.len(),
                }))?).await?;
                transaction_id = transaction_id.wrapping_add(1);
                let reply = outgoing.send(1, &body).await?;
                record_packet(
                    recording,
                    &directory.join(format!("packet-{index:03}-s2c.bin")),
                    &reply,
                )
                .await?;
                // Pushes share the encrypted S2C sequence but do not consume an
                // RPC transaction ID or disturb the client's reply FIFO.
                for (push_index, body) in response.pushes.iter().enumerate() {
                    let wire = outgoing.send(2, body).await?;
                    record_packet(
                        recording,
                        &directory.join(format!("packet-{index:03}-push-{push_index:02}.bin")),
                        &wire,
                    )
                    .await?;
                }
                if let Some(services) = services {
                    services.publish_lobby(response.deliveries).await;
                }
                tracing::info!(
                    session_id = id,
                    handler = response.label,
                    "task reply sent in request order"
                );
            }
        } else if services.is_some() {
            outgoing.stream.write_all(&[0; 4]).await?;
        }
        tracing::info!(
            session_id = id,
            index,
            bytes = frame.len(),
            "lobby application frame recorded"
        );
    }
    Ok(())
}

#[cfg(test)]
mod delivery_tests {
    use super::*;
    use crate::{
        lobby_delivery::{DirectedPush, Identity},
        session::Session,
    };
    #[tokio::test]
    async fn idle_push_does_not_cancel_a_partial_client_frame_or_consume_rpc_transaction() {
        let temp = tempfile::tempdir().unwrap();
        let services = LocalServices::open(temp.path().join("services"))
            .await
            .unwrap();
        let actor = Session {
            title_id: 1002,
            user_id: 42,
            iv_seed: 1,
            expires_at: u32::MAX,
            key: [0; 24],
        };
        let mut subscription = services.subscribe_lobby(&actor).await;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (server, _) = listener.accept().await.unwrap();
        let (mut reader, stream) = server.into_split();
        let directory = temp.path().to_path_buf();
        let work = tokio::spawn(async move {
            let mut writer = Outbound {
                stream,
                keys: [7; 72],
                sequence: 1,
                maximum: 65536,
                mailbox_count: 0,
                mailbox_bytes: 0,
            };
            let frame = read_with_mailbox(
                &mut reader,
                &mut writer,
                Some(&mut subscription),
                &directory,
            )
            .await
            .unwrap();
            (frame, writer.sequence)
        });
        client.write_all(&[2, 0]).await.unwrap();
        tokio::task::yield_now().await;
        services
            .publish_lobby(vec![DirectedPush {
                target: Identity::from(&actor),
                body: vec![1, 2, 3],
            }])
            .await;
        let wire = tokio::time::timeout(Duration::from_secs(2), read_frame(&mut client))
            .await
            .unwrap()
            .unwrap();
        let message =
            lobby::decrypt_application(&wire, &[7; 72], lobby::Direction::ServerToClient, 1)
                .unwrap();
        assert_eq!(message.kind, 2);
        assert_eq!(message.body, [1, 2, 3]);
        client.write_all(&[0, 0, 9, 8]).await.unwrap();
        let (frame, sequence) = tokio::time::timeout(Duration::from_secs(2), work)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(frame, [2, 0, 0, 0, 9, 8]);
        assert_eq!(sequence, 2);
    }
}
#[cfg(test)]
#[path = "lobby_room_transport_tests.rs"]
mod room_transport_tests;
