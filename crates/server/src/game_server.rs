//! Experimental game UDP transport, admitted by the local matchmaking service.
//! It is not a world simulator. Wire discoveries and limitations live in docs.
use crate::game_handshake::Peer;
use crate::{
    auth_server::write_private, capture::unix_time_ms, services::TaskResult, session::Session,
};
use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use codol_protocol::task::{self, Reader, Writer};
use std::{
    collections::HashMap,
    future::Future,
    net::{SocketAddr, SocketAddrV4},
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::net::UdpSocket;

#[derive(Clone)]
pub(crate) struct Admission {
    title: u32,
    owner: u64,
    seed: u32,
    build: i32,
    expires: u32,
    pub(crate) key: [u8; 16],
    booked: bool,
    start_command: Option<String>,
}

pub struct GameAdmissions {
    address: SocketAddrV4,
    entries: Mutex<HashMap<[u8; 8], Admission>>,
}
impl GameAdmissions {
    pub fn address(&self) -> SocketAddrV4 {
        self.address
    }
    pub(crate) fn allocate(
        &self,
        session: &Session,
        security_key: [u8; 16],
        build: i32,
        now: u32,
    ) -> Result<([u8; 8], [u8; 16])> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| anyhow::anyhow!("game admissions poisoned"))?;
        ensure!(session.expires_at > now, "expired game allocation session");
        entries.retain(|_, a| {
            a.expires > now
                && (a.title != session.title_id
                    || a.owner != session.user_id
                    || (a.seed == session.iv_seed && a.key == security_key && a.build == build))
        });
        if let Some((id, a)) = entries.iter().find(|(_, a)| {
            a.title == session.title_id && a.owner == session.user_id && a.seed == session.iv_seed
        }) {
            return Ok((*id, a.key));
        }
        ensure!(entries.len() < 32, "game admission capacity");
        let mut id = [0u8; 8];
        let key = security_key;
        loop {
            openssl::rand::rand_bytes(&mut id)?;
            if id != [0; 8] && !entries.contains_key(&id) {
                break;
            }
        }
        entries.insert(
            id,
            Admission {
                title: session.title_id,
                owner: session.user_id,
                seed: session.iv_seed,
                build,
                expires: session.expires_at.min(now.saturating_add(930)),
                key,
                booked: false,
                start_command: None,
            },
        );
        Ok((id, key))
    }
    pub(crate) fn control(
        &self,
        arguments: &[u8],
        session: &Session,
        now: u32,
    ) -> Result<TaskResult> {
        let mut r = Reader::new(arguments);
        let recipient = r.u64()?;
        let payload = r.blob()?;
        r.finish()?;
        ensure!(
            recipient == 1_000_001 && (5..=4096).contains(&payload.len()),
            "invalid game control recipient/size"
        );
        let size = u32::from_le_bytes(payload[..4].try_into()?) as usize;
        ensure!(size == payload.len() - 4, "game command length mismatch");
        let command = std::str::from_utf8(&payload[4..])?;
        ensure!(
            command.is_ascii()
                && !command
                    .bytes()
                    .any(|b| (b < 32 && !b.is_ascii_whitespace()) || b == 127),
            "invalid game command text"
        );
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| anyhow::anyhow!("game admissions poisoned"))?;
        let mut response = TaskResult::success("local_game_control", vec![]);
        if command.starts_with("mmbook ") {
            let parts = command.split(' ').collect::<Vec<_>>();
            ensure!(parts.len() == 4, "invalid mmbook fields");
            let _playlist_version = parts[1].parse::<u32>()?;
            let id: [u8; 8] = STANDARD
                .decode(parts[2])?
                .try_into()
                .map_err(|_| anyhow::anyhow!("invalid mmbook ID"))?;
            ensure!(matches!(parts[3], "0" | "1"), "unsupported booking kind");
            let admission = entries
                .get_mut(&id)
                .filter(|a| {
                    a.expires > now
                        && a.title == session.title_id
                        && a.owner == session.user_id
                        && a.seed == session.iv_seed
                })
                .ok_or_else(|| anyhow::anyhow!("booking does not belong to session"))?;
            admission.booked = true;
            let reply = b"mmbook accepted";
            let mut body = (reply.len() as u32).to_le_bytes().to_vec();
            body.extend_from_slice(reply);
            // BD_GLOBAL_INSTANT_MESSAGE=21 (target enum table 0xcd6d80).
            let mut push = Writer::default();
            push.u32(21);
            push.u64(1_000_001);
            push.string("Local Game Endpoint");
            push.blob(&body)?;
            response.label = "local_game_booked";
            response.pushes.push(push.0);
        } else if command.starts_with("mmstart ") {
            let admission = entries
                .values_mut()
                .find(|a| {
                    a.expires > now
                        && a.title == session.title_id
                        && a.owner == session.user_id
                        && a.seed == session.iv_seed
                        && a.booked
                })
                .ok_or_else(|| anyhow::anyhow!("game start requires booking"))?;
            // Capture configuration for protocol recovery; never execute received console text.
            admission.start_command = Some(command.to_owned());
            response.label = "local_game_transport_start_requested";
            tracing::info!(
                bytes = command.len(),
                "game start configuration received (simulation pending)"
            );
        } else {
            return Ok(TaskResult::failure(
                "unsupported_game_control",
                task::TASK_FAILED,
            ));
        }
        Ok(response)
    }
    pub(crate) fn revoke_other_sessions(&self, session: &Session) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.retain(|_, a| {
                a.title != session.title_id
                    || a.owner != session.user_id
                    || a.seed == session.iv_seed
            });
        }
    }
    pub(crate) fn revoke(&self, session: &Session) {
        if let Ok(mut e) = self.entries.lock() {
            e.retain(|_, a| {
                !(a.title == session.title_id
                    && a.owner == session.user_id
                    && a.seed == session.iv_seed)
            });
        }
    }
    fn get(&self, id: &[u8; 8], now: u32) -> Option<Admission> {
        self.entries
            .lock()
            .ok()?
            .get(id)
            .filter(|a| a.expires > now && a.booked && a.start_command.is_some())
            .cloned()
    }
}

pub struct GameServer {
    socket: UdpSocket,
    admissions: Arc<GameAdmissions>,
    directory: PathBuf,
}
impl GameServer {
    pub async fn bind(address: SocketAddrV4, directory: PathBuf) -> Result<Self> {
        ensure!(
            address.ip().is_loopback(),
            "experimental game endpoint must bind loopback"
        );
        let socket = UdpSocket::bind(address).await?;
        let SocketAddr::V4(address) = socket.local_addr()? else {
            anyhow::bail!("IPv4 game endpoint required")
        };
        tokio::fs::create_dir_all(&directory).await?;
        write_private(&directory.join("run.json"),&serde_json::to_vec_pretty(&serde_json::json!({
            "mode":"experimental_game_udp_transport","bind":address,"simulation":false,"max_datagrams":2048,
            "client_sha256":codol_protocol::CLIENT_SHA256
        }))?).await?;
        Ok(Self {
            socket,
            directory,
            admissions: Arc::new(GameAdmissions {
                address,
                entries: Mutex::new(HashMap::new()),
            }),
        })
    }
    pub fn admissions(&self) -> Arc<GameAdmissions> {
        self.admissions.clone()
    }
    pub async fn run<F: Future<Output = std::io::Result<()>>>(self, shutdown: F) -> Result<()> {
        tokio::pin!(shutdown);
        let mut buffer = [0; 4096];
        let mut count = 0;
        let mut total = 0;
        let mut peers = HashMap::<SocketAddrV4, Peer>::new();
        loop {
            let (n, peer) = tokio::select! {result=&mut shutdown=>{result?;return Ok(())},
            result=self.socket.recv_from(&mut buffer)=>result?};
            let SocketAddr::V4(peer) = peer else { continue };
            if !peer.ip().is_loopback() || count >= 2048 || total + n > 8 * 1024 * 1024 {
                continue;
            }
            count += 1;
            total += n;
            let wire = &buffer[..n];
            write_private(
                &self.directory.join(format!("{count:04}-request.bin")),
                wire,
            )
            .await?;
            let now = (unix_time_ms() / 1000) as u32;
            let live = self
                .admissions
                .entries
                .lock()
                .map_err(|_| anyhow::anyhow!("game admissions poisoned"))?
                .iter()
                .any(|(_, a)| a.expires > now);
            peers.retain(|_, p| self.admissions.get(&p.id, now).is_some());
            let mut decoded = None;
            let mut inner_kind = None;
            let mut inner_dropped = None;
            let mut client_server_id = None;
            let mut client_message_ack = None;
            let mut client_reliable_ack = None;
            let handled = (|| -> Result<Option<Vec<u8>>> {
                if live && n == 29 && wire[..3] == [13, 2, 0] {
                    let mut reply = wire.to_vec();
                    reply[0] = 12;
                    return Ok(Some(reply));
                } else if n == 16 && wire[..2] == [1, 2] {
                    let id: [u8; 8] = wire[8..16].try_into()?;
                    ensure!(
                        self.admissions.get(&id, now).is_some(),
                        "game INIT is not admitted/started"
                    );
                    if !peers.get(&peer).is_some_and(|p| p.is_retry(wire)) {
                        ensure!(
                            peers.len() < 32 || peers.contains_key(&peer),
                            "game peer capacity"
                        );
                        peers.insert(peer, Peer::init(wire, peer)?);
                    }
                    return Ok(Some(peers[&peer].init_reply()));
                } else if n == 189 && wire[..2] == [3, 2] {
                    let p = peers
                        .get_mut(&peer)
                        .ok_or_else(|| anyhow::anyhow!("cookie without INIT"))?;
                    let admission = self
                        .admissions
                        .get(&p.id, now)
                        .ok_or_else(|| anyhow::anyhow!("expired cookie admission"))?;
                    return Ok(Some(p.cookie(wire, &admission.key)?));
                } else if n >= 16 && wire[..2] == [6, 2] {
                    let p = peers
                        .get_mut(&peer)
                        .ok_or_else(|| anyhow::anyhow!("DATA without peer"))?;
                    let records = p
                        .records
                        .as_mut()
                        .ok_or_else(|| anyhow::anyhow!("DATA before cookie exchange"))?;
                    decoded = Some(records.open(wire, p.local_tag)?);
                    let admission = self
                        .admissions
                        .get(&p.id, now)
                        .ok_or_else(|| anyhow::anyhow!("DATA admission expired"))?;
                    let payload = decoded.as_ref().expect("decoded DATA");
                    let payload =
                        [payload.encrypted.as_slice(), payload.cleartext.as_slice()].concat();
                    if payload.len() >= 14 && payload[..4] != [255, 255, 255, 255] {
                        let header = codol_protocol::game_wire::ClientHeader::parse(&payload)?;
                        let previous_sequence = p.inner_sequence;
                        p.inner_sequence = Some(header.sequence);
                        client_server_id = Some(header.server_id);
                        client_message_ack = Some(header.message_ack);
                        client_reliable_ack = Some(header.reliable_ack);
                        inner_dropped = previous_sequence
                            .map(|previous| header.sequence.saturating_sub(previous + 1));
                    } else if payload.len() >= 4 && payload[..4] != [255, 255, 255, 255] {
                        let message =
                            codol_protocol::game_wire::incoming(&payload, p.inner_sequence)?;
                        p.inner_sequence = Some(message.sequence);
                        inner_kind = message.kind.map(|kind| format!("{kind:?}"));
                        inner_dropped = Some(message.dropped);
                    }
                    if let Some(reply) =
                        p.connection
                            .handle(&payload, admission.owner, admission.build)?
                    {
                        return Ok(Some(records.seal(&reply, p.remote_tag)?));
                    }
                }
                Ok(None)
            })();
            let connection_stage = peers.get(&peer).map(|p| p.connection.stage());
            let error = handled.as_ref().err().map(ToString::to_string);
            let response = handled.unwrap_or(None);
            if let Some(plain) = &decoded {
                write_private(
                    &self.directory.join(format!("{count:04}-plaintext.bin")),
                    &[plain.encrypted.as_slice(), plain.cleartext.as_slice()].concat(),
                )
                .await?;
            }
            if let Some(reply) = &response {
                self.socket.send_to(reply, peer).await?;
                write_private(&self.directory.join(format!("{count:04}-reply.bin")), reply).await?;
            }
            // INIT metadata only; security material remains in memory/capture.
            let admitted = if n == 16 && wire[0] == 1 {
                let id: [u8; 8] = wire[8..16].try_into()?;
                self.admissions.get(&id, now).is_some()
            } else {
                false
            };
            write_private(&self.directory.join(format!("{count:04}-event.json")),&serde_json::to_vec(&serde_json::json!({
                "peer":peer,"bytes":n,"type":wire.first(),"version_byte":wire.get(1),"admitted_init":admitted,
                "reply_bytes":response.as_ref().map_or(0,Vec::len),"unix_time_ms":unix_time_ms()
                ,"connection_stage":connection_stage,"inner_kind":inner_kind,"inner_dropped":inner_dropped,"client_server_id":client_server_id,"client_message_ack":client_message_ack,"client_reliable_ack":client_reliable_ack,"mac_verified":decoded.is_some(),"plaintext_bytes":decoded.as_ref().map(|p| p.encrypted.len() + p.cleartext.len()),"encrypted_payload_bytes":decoded.as_ref().map(|p| p.encrypted.len()),"error":error
            }))?).await?;
            tracing::info!(%peer,bytes=n,kind=?wire.first(),admitted_init=admitted,mac_verified=decoded.is_some(),"game UDP datagram captured");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        Session {
            title_id: 1002,
            user_id: u64::MAX - 1,
            iv_seed: 41,
            expires_at: 100,
            key: [0; 24],
        }
    }
    fn command(text: &str) -> Vec<u8> {
        let mut w = Writer::default();
        w.u64(1_000_001);
        let mut data = (text.len() as u32).to_le_bytes().to_vec();
        data.extend_from_slice(text.as_bytes());
        w.blob(&data).unwrap();
        w.0.push(0);
        w.0
    }
    fn admissions() -> GameAdmissions {
        GameAdmissions {
            address: "127.0.0.1:39004".parse().unwrap(),
            entries: Mutex::new(HashMap::new()),
        }
    }

    #[test]
    fn booking_is_session_bound_and_start_is_required_for_udp_admission() {
        let a = admissions();
        let s = session();
        let (id, key) = a.allocate(&s, [7; 16], -688906341, 1).unwrap();
        assert_eq!(key, [7; 16]);
        assert_eq!(a.allocate(&s, key, -688906341, 2).unwrap().0, id);
        let start = command("mmstart sv_resetServer; playlist 0");
        assert!(a.control(&start, &s, 2).is_err());
        let book = command(&format!("mmbook 4 {} 0", STANDARD.encode(id)));
        let mut other = s.clone();
        other.user_id -= 1;
        assert!(a.control(&book, &other, 2).is_err());
        other = s.clone();
        other.title_id += 1;
        assert!(a.control(&book, &other, 2).is_err());
        other = s.clone();
        other.iv_seed += 1;
        assert!(a.control(&book, &other, 2).is_err());
        let reply = a.control(&book, &s, 2).unwrap();
        let mut r = Reader::new(&reply.pushes[0]);
        assert_eq!(r.u32().unwrap(), 21);
        assert_eq!(r.u64().unwrap(), 1_000_001);
        r.string(64).unwrap();
        assert_eq!(
            r.blob().unwrap(),
            [15u32.to_le_bytes().as_slice(), b"mmbook accepted"].concat()
        );
        assert!(a.get(&id, 2).is_none());
        assert!(a.control(&start, &s, 2).unwrap().result.is_ok());
        assert!(a.get(&id, 2).is_some());
        assert!(a.control(&book, &s, 100).is_err());
        assert!(a.get(&id, 100).is_none());
        a.revoke(&s);
        assert!(a.get(&id, 2).is_none());
    }

    #[test]
    fn key_rotation_and_new_session_revoke_the_old_allocation() {
        let a = admissions();
        let mut s = session();
        let (old, _) = a.allocate(&s, [7; 16], -688906341, 1).unwrap();
        let (new, _) = a.allocate(&s, [8; 16], -688906341, 1).unwrap();
        assert_ne!(old, new);
        assert!(!a.entries.lock().unwrap().contains_key(&old));
        s.iv_seed += 1;
        a.revoke_other_sessions(&s);
        assert!(a.entries.lock().unwrap().is_empty());
        let (next, _) = a.allocate(&s, [8; 16], -688906341, 1).unwrap();
        assert_ne!(next, new);
        assert!(a.allocate(&s, [8; 16], -688906341, 100).is_err());
    }

    #[tokio::test]
    async fn udp_admission_handshake_and_authenticated_challenge_roundtrip() {
        use std::time::Duration;
        let temp = tempfile::tempdir().unwrap();
        let server = GameServer::bind("127.0.0.1:0".parse().unwrap(), temp.path().join("game"))
            .await
            .unwrap();
        let a = server.admissions();
        let now = (unix_time_ms() / 1000) as u32;
        let mut s = session();
        s.expires_at = now + 30;
        let (id, _) = a.allocate(&s, [7; 16], -688906341, now).unwrap();
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(server.run(async {
            let _ = stopped.await;
            Ok(())
        }));
        let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let mut init = vec![1, 2, 0, 0, 0, 0, 42, 0];
        init.extend_from_slice(&id);
        let mut buffer = [0; 256];
        client.send_to(&init, a.address()).await.unwrap();
        // Wait until the server has actually evaluated the rejected INIT;
        // filesystem capture can be slower than a short socket timeout.
        tokio::time::timeout(Duration::from_secs(2), async {
            while !temp.path().join("game/0001-event.json").exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), client.recv_from(&mut buffer))
                .await
                .is_err()
        );
        a.control(
            &command(&format!("mmbook 4 {} 0", STANDARD.encode(id))),
            &s,
            now,
        )
        .unwrap();
        a.control(&command("mmstart playlist 0"), &s, now).unwrap();
        client.send_to(&init, a.address()).await.unwrap();
        let (n, source) =
            tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut buffer))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(source, SocketAddr::V4(a.address()));
        assert_eq!(n, 38);
        assert_eq!(&buffer[..4], &[2, 2, 42, 0]);
        let first = buffer[..n].to_vec();
        client.send_to(&init, a.address()).await.unwrap();
        let (n, _) = tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut buffer))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&buffer[..n], first);
        let local_tag = u16::from_le_bytes([first[14], first[15]]);
        let group = openssl::ec::EcGroup::from_curve_name(openssl::nid::Nid::SECP224R1).unwrap();
        let ec = openssl::ec::EcKey::generate(&group).unwrap();
        let public = crate::game_handshake::encode_public(&ec).unwrap();
        let key = openssl::pkey::PKey::from_ec_key(ec).unwrap();
        let mut cookie = vec![3, 2];
        cookie.extend_from_slice(&local_tag.to_le_bytes());
        cookie.extend_from_slice(&[0, 0]);
        cookie.extend_from_slice(&first);
        cookie.extend_from_slice(&[0; 37]);
        cookie.extend_from_slice(&id);
        cookie.extend_from_slice(&public);
        client.send_to(&cookie, a.address()).await.unwrap();
        let (n, _) = tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut buffer))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(n, 114);
        assert_eq!(&buffer[..6], &[4, 2, 42, 0, 0, 0]);
        let shared = crate::game_handshake::shared_secret(&key, &buffer[6..106]).unwrap();
        let mut records =
            codol_protocol::game::Records::new(codol_protocol::game::derive_key(&shared, &[7; 16]));
        let mut challenge = vec![255; 4];
        challenge.extend_from_slice(format!("getchallenge 0 \"{:016x}\"\0", s.user_id).as_bytes());
        let mut challenge_number = String::new();
        for _ in 0..2 {
            client
                .send_to(&records.seal(&challenge, local_tag).unwrap(), a.address())
                .await
                .unwrap();
            let (n, _) =
                tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut buffer))
                    .await
                    .unwrap()
                    .unwrap();
            let reply = records.open(&buffer[..n], 42).unwrap();
            assert!(
                reply
                    .encrypted
                    .starts_with(b"\xff\xff\xff\xffchallengeResponse ")
            );
            assert!(reply.cleartext.is_empty());
            challenge_number = std::str::from_utf8(&reply.encrypted[22..reply.encrypted.len() - 1])
                .unwrap()
                .to_owned();
        }
        let mut connect = vec![255; 4];
        connect.extend_from_slice(format!("connect {0:016x} \"\\xuid\\{0:016x}\\challenge\\{challenge_number}\\2490\\-688906341\"", s.user_id).as_bytes());
        client
            .send_to(&records.seal(&connect, local_tag).unwrap(), a.address())
            .await
            .unwrap();
        let (n, _) = tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut buffer))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            records.open(&buffer[..n], 42).unwrap().encrypted,
            b"\xff\xff\xff\xffconnectResponse\0"
        );
        a.revoke(&s);
        client.send_to(&init, a.address()).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), client.recv_from(&mut buffer))
                .await
                .is_err()
        );
        stop.send(()).unwrap();
        task.await.unwrap().unwrap();
    }
}
