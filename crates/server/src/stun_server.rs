use crate::auth_server::write_private;
use anyhow::{Context, Result, ensure};
use codol_protocol::stun::{self, ReplySource};
use std::{
    future::Future,
    net::{SocketAddr, SocketAddrV4},
    path::PathBuf,
};
use tokio::net::UdpSocket;

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
    pub async fn run<F: Future<Output = std::io::Result<()>>>(self, shutdown: F) -> Result<()> {
        let mut a = [0; 2048];
        let mut b = [0; 2048];
        let mut count = 0;
        tokio::pin!(shutdown);
        loop {
            let (data, peer, from_alt) = tokio::select! {
                biased;
                result=&mut shutdown=>return result.context("STUN stop signal"),
                received=self.primary.recv_from(&mut a)=>{let(n,p)=received?;(a[..n].to_vec(),p,false)},
                received=self.alternate.recv_from(&mut b)=>{let(n,p)=received?;(b[..n].to_vec(),p,true)},
            };
            if count >= 512 {
                continue;
            }
            count += 1;
            let SocketAddr::V4(peer) = peer else {
                continue;
            };
            let SocketAddr::V4(alt) = self.alternate.local_addr()? else {
                continue;
            };
            write_private(
                &self.directory.join(format!("{count:04}-request.bin")),
                &data,
            )
            .await?;
            if let Some((reply, source)) = stun::reply(&data, peer, alt) {
                let socket = if source == ReplySource::Alternate || from_alt {
                    &self.alternate
                } else {
                    &self.primary
                };
                socket.send_to(&reply, peer).await?;
                write_private(
                    &self.directory.join(format!("{count:04}-reply.bin")),
                    &reply,
                )
                .await?;
                tracing::info!(request_type=data[0],bytes=data.len(),source=%socket.local_addr()?,"STUN discovery answered");
            } else {
                tracing::info!(bytes = data.len(), "unrecognized STUN datagram recorded");
            }
        }
    }
}
