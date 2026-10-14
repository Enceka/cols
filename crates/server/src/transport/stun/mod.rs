use crate::infra::storage::write_private;
use anyhow::{Context, Result, ensure};
use codol_protocol::discovery::stun;
use codol_protocol::discovery::stun::ReplySource;
use std::{
    future::Future,
    net::{SocketAddr, SocketAddrV4},
    path::PathBuf,
    time::Duration,
};
use tokio::{net::UdpSocket, sync::mpsc, task::JoinSet};

const CAPTURE_LIMIT: u64 = 512;
struct Capture {
    index: u64,
    request: Vec<u8>,
    reply: Option<Vec<u8>>,
}

pub struct StunServer {
    primary: UdpSocket,
    alternate: UdpSocket,
    directory: PathBuf,
}
impl StunServer {
    pub async fn bind(
        primary: SocketAddrV4,
        alternate: SocketAddrV4,
        directory: PathBuf,
    ) -> Result<Self> {
        ensure!(
            primary.ip().is_loopback() && alternate.ip().is_loopback(),
            "research STUN must bind loopback"
        );
        let primary = UdpSocket::bind(primary).await?;
        let alternate = UdpSocket::bind(alternate).await?;
        tokio::fs::create_dir(&directory).await?;
        Ok(Self {
            primary,
            alternate,
            directory,
        })
    }
    pub fn address(&self) -> Result<SocketAddr> {
        Ok(self.primary.local_addr()?)
    }
    pub fn alternate_address(&self) -> Result<SocketAddr> {
        Ok(self.alternate.local_addr()?)
    }
    pub async fn run<F: Future<Output = std::io::Result<()>>>(self, shutdown: F) -> Result<()> {
        // Diagnostic disk I/O must neither delay address discovery nor end
        // the live service. Bound both queued records and total capture count.
        let (capture, mut records) = mpsc::channel::<Capture>(8);
        let directory = self.directory.clone();
        let mut writer = JoinSet::new();
        writer.spawn(async move {
            while let Some(record) = records.recv().await {
                let result = async {
                    write_private(
                        &directory.join(format!("{:04}-request.bin", record.index)),
                        &record.request,
                    )
                    .await?;
                    if let Some(reply) = record.reply {
                        write_private(
                            &directory.join(format!("{:04}-reply.bin", record.index)),
                            &reply,
                        )
                        .await?;
                    }
                    anyhow::Ok(())
                }
                .await;
                if let Err(error) = result {
                    tracing::warn!(%error,"STUN capture disabled; discovery remains available");
                    break;
                }
            }
        });
        let result = self.serve(shutdown, &capture).await;
        drop(capture);
        // A full/slow disk must not hold shutdown indefinitely either.
        if tokio::time::timeout(Duration::from_secs(1), writer.join_next())
            .await
            .is_err()
        {
            writer.abort_all();
        }
        result
    }
    async fn serve<F: Future<Output = std::io::Result<()>>>(
        &self,
        shutdown: F,
        capture: &mpsc::Sender<Capture>,
    ) -> Result<()> {
        let mut a = [0; 2048];
        let mut b = [0; 2048];
        let mut count = 0u64;
        let mut skipped = 0u64;
        tokio::pin!(shutdown);
        loop {
            let (data, peer, from_alt) = tokio::select! {
                biased;
                result=&mut shutdown=>return result.context("STUN stop signal"),
                received=self.primary.recv_from(&mut a)=>{let(n,p)=received?;(a[..n].to_vec(),p,false)},
                received=self.alternate.recv_from(&mut b)=>{let(n,p)=received?;(b[..n].to_vec(),p,true)},
            };
            count = count.saturating_add(1);
            let SocketAddr::V4(peer) = peer else {
                continue;
            };
            let SocketAddr::V4(alt) = self.alternate.local_addr()? else {
                continue;
            };
            let reply = if let Some((reply, source)) = stun::reply(&data, peer, alt) {
                let socket = if source == ReplySource::Alternate || from_alt {
                    &self.alternate
                } else {
                    &self.primary
                };
                socket.send_to(&reply, peer).await?;
                tracing::info!(request_type=data[0],bytes=data.len(),source=%socket.local_addr()?,"STUN discovery answered");
                Some(reply)
            } else {
                tracing::debug!(bytes = data.len(), "unrecognized STUN datagram");
                None
            };
            if count <= CAPTURE_LIMIT
                && capture
                    .try_send(Capture {
                        index: count,
                        request: data,
                        reply,
                    })
                    .is_err()
            {
                skipped += 1;
            }
            if count == CAPTURE_LIMIT {
                tracing::info!(
                    skipped,
                    "STUN capture budget reached; discovery remains available"
                );
            }
        }
    }
}
