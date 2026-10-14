use super::*;
use crate::backend::identity::session::Session;
use crate::transport::game::server::GameServer;
use codol_protocol::lobby::task::Reader;
use codol_protocol::lobby::task::Writer;

struct Peer {
    stream: TcpStream,
    keys: [u8; 72],
    incoming: u32,
    outgoing: u32,
    transaction: u64,
}
fn bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}
impl Peer {
    async fn connect(address: SocketAddr, registry: &SessionRegistry, user: u64) -> Self {
        let session = Session {
            title_id: 1002,
            user_id: user,
            iv_seed: user as u32,
            expires_at: u32::MAX,
            key: [user as u8; 24],
        };
        let ticket = [user as u8; 128];
        registry.insert(ticket, session.clone()).unwrap();
        let mut stream = TcpStream::connect(address).await.unwrap();
        let fixture: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/protocol/lobby210-synthetic.json"
        )))
        .unwrap();
        let hello: [u8; 28] = bytes(fixture["client_hello"].as_str().unwrap())
            .try_into()
            .unwrap();
        stream.write_all(&hello).await.unwrap();
        let server = read_frame(&mut stream).await.unwrap();
        let mut encoded = Vec::new();
        let mut position = 0;
        let mut put = |value: u64, count: usize| {
            for bit in 0..count {
                if position / 8 == encoded.len() {
                    encoded.push(0);
                }
                encoded[position / 8] |= (((value >> bit) & 1) as u8) << (position % 8);
                position += 1;
            }
        };
        put(7, 8);
        put(1, 1);
        put(8, 5);
        put(1002, 32);
        put(8, 5);
        put(u64::from(session.iv_seed), 32);
        for byte in ticket {
            put(u64::from(byte), 8);
        }
        let mut auth = lobby::frame(0x82, &[encoded, vec![0; 8]].concat());
        let secrets = lobby::derive_secrets(&hello, &server, &auth, &session.key).unwrap();
        let end = auth.len();
        auth[end - 8..].copy_from_slice(&secrets.client_proof);
        stream.write_all(&auth).await.unwrap();
        let ack = read_frame(&mut stream).await.unwrap();
        assert_eq!(ack, lobby::frame(0x83, &secrets.server_proof));
        Self {
            stream,
            keys: secrets.data_keys,
            incoming: 1,
            outgoing: 1,
            transaction: 1,
        }
    }
    async fn message(&mut self) -> lobby::ApplicationMessage {
        let wire = tokio::time::timeout(Duration::from_secs(3), read_frame(&mut self.stream))
            .await
            .unwrap()
            .unwrap();
        let message = lobby::decrypt_application(
            &wire,
            &self.keys,
            lobby::Direction::ServerToClient,
            self.incoming,
        )
        .unwrap();
        self.incoming += 1;
        message
    }
    async fn rpc(&mut self, body: &[u8]) -> (u32, Vec<u8>) {
        let wire = lobby::encrypt_application(
            0x86,
            body,
            &self.keys,
            lobby::Direction::ClientToServer,
            self.outgoing,
            [self.outgoing as u8; 16],
        )
        .unwrap();
        self.outgoing += 1;
        self.stream.write_all(&wire).await.unwrap();
        let message = self.message().await;
        assert_eq!(message.kind, 1);
        let mut r = Reader::new(&message.body);
        assert_eq!(r.u64().unwrap(), self.transaction);
        self.transaction += 1;
        assert_eq!(r.u32().unwrap(), 0);
        assert_eq!(r.u8().unwrap(), body[2]);
        let count = r.u32().unwrap();
        if count > 0 {
            assert_eq!(r.u32().unwrap(), count);
        }
        (count, r.remaining().to_vec())
    }
    async fn create_room(&mut self) -> u64 {
        let (_, record) = self.rpc(&[11, 3, 13, 0]).await;
        let id = Reader::new(&record).u64().unwrap();
        let fixture: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/protocol/room-browser-synthetic.json"
        )))
        .unwrap();
        let mut description = bytes(fixture["listing"]["create_arguments"].as_str().unwrap());
        let end = description.len();
        description[end - 19..end - 11].copy_from_slice(&id.to_le_bytes());
        let mut body = vec![21, 3, 1];
        body.extend(description);
        self.rpc(&body).await;
        id
    }
}
fn member_request(id: u64) -> Vec<u8> {
    let mut w = Writer(vec![11, 3, 15]);
    w.u64(id);
    w.u32(100);
    w.0.push(0);
    w.0
}
fn broadcast(id: u64, body: &[u8]) -> Vec<u8> {
    let mut w = Writer(vec![11, 3, 4]);
    w.u64(id);
    w.blob(body).unwrap();
    w.u32(1);
    w.0.push(0);
    w.0
}

#[tokio::test]
async fn two_authenticated_tcp_peers_browse_join_exchange_state_and_leave_while_other_peer_is_idle()
{
    let temp = tempfile::tempdir().unwrap();
    let registry = Arc::new(SessionRegistry::default());
    let game = GameServer::bind("127.0.0.1:0".parse().unwrap(), temp.path().join("game"))
        .await
        .unwrap();
    let lobby = LobbyServer::bind_with_services(
        "127.0.0.1:0".parse().unwrap(),
        registry.clone(),
        temp.path().join("lobby"),
        Some(temp.path().join("services")),
    )
    .await
    .unwrap()
    .with_game_endpoint(game.admissions())
    .await
    .unwrap();
    let address = lobby.listener.local_addr().unwrap();
    let (stop, wait) = tokio::sync::oneshot::channel();
    let work = tokio::spawn(lobby.run(async {
        let _ = wait.await;
        Ok(())
    }));
    let mut owner = Peer::connect(address, &registry, 42).await;
    let mut guest = Peer::connect(address, &registry, 43).await;
    let (_, record) = owner.rpc(&[11, 3, 13, 0]).await;
    let id = Reader::new(&record).u64().unwrap();
    let fixture: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/protocol/room-browser-synthetic.json"
    )))
    .unwrap();
    let mut description = bytes(fixture["listing"]["create_arguments"].as_str().unwrap());
    let end = description.len();
    description[end - 19..end - 11].copy_from_slice(&id.to_le_bytes());
    let mut create = vec![21, 3, 1];
    create.extend(description);
    let (_, session) = owner.rpc(&create).await;
    let sid = Reader::new(&session).blob().unwrap().to_vec();
    let (count, record) = guest
        .rpc(&bytes(fixture["requests"]["first_page"].as_str().unwrap()))
        .await;
    assert_eq!(count, 1);
    let mut r = Reader::new(&record);
    r.blob().unwrap();
    assert_eq!(r.blob().unwrap(), sid);
    let mut join = Writer(vec![11, 3, 11]);
    join.u64(id);
    join.u64(0);
    join.0.push(0);
    guest.rpc(&join.0).await;
    let self_join = guest.message().await;
    assert_eq!(self_join.kind, 2);
    assert_eq!(Reader::new(&self_join.body).u32().unwrap(), 9);
    // Owner sends no RPC to obtain this notification; its mailbox wakes the writer.
    let joined = owner.message().await;
    assert_eq!(joined.kind, 2);
    let mut r = Reader::new(&joined.body);
    assert_eq!(r.u32().unwrap(), 9);
    assert_eq!(r.u64().unwrap(), 43);
    assert_eq!(guest.rpc(&member_request(id)).await.0, 2);
    for ready in [true, false] {
        let body = codol_protocol::room::messages::bot_ready(ready);
        guest.rpc(&broadcast(id, &body)).await;
        let echoed = guest.message().await;
        assert_eq!(echoed.kind, 2);
        let delivered = owner.message().await;
        let mut r = Reader::new(&delivered.body);
        assert_eq!(r.u32().unwrap(), 7);
        assert_eq!(r.u64().unwrap(), 43);
        assert_eq!(r.string(64).unwrap(), "NDM=");
        assert_eq!(r.u64().unwrap(), id);
        assert_eq!(r.blob().unwrap(), body);
    }
    let mut leave = Writer(vec![11, 3, 2]);
    leave.u64(id);
    leave.0.push(0);
    guest.rpc(&leave.0).await;
    let left = owner.message().await;
    assert_eq!(left.kind, 2);
    assert_eq!(Reader::new(&left.body).u32().unwrap(), 10);
    // RPC IDs remain FIFO although the encrypted S2C sequence includes idle pushes.
    assert_eq!(owner.rpc(&member_request(id)).await.0, 1);
    drop(owner);
    drop(guest);
    stop.send(()).unwrap();
    work.await.unwrap().unwrap();
}

#[tokio::test]
async fn closing_guest_socket_removes_membership_and_notifies_idle_owner() {
    let temp = tempfile::tempdir().unwrap();
    let registry = Arc::new(SessionRegistry::default());
    let lobby = LobbyServer::bind_with_services(
        "127.0.0.1:0".parse().unwrap(),
        registry.clone(),
        temp.path().join("lobby"),
        Some(temp.path().join("services")),
    )
    .await
    .unwrap();
    let address = lobby.address().unwrap();
    let (stop, wait) = tokio::sync::oneshot::channel();
    let work = tokio::spawn(lobby.run(async {
        let _ = wait.await;
        Ok(())
    }));
    let mut owner = Peer::connect(address, &registry, 42).await;
    let id = owner.create_room().await;
    let mut guest = Peer::connect(address, &registry, 43).await;
    let mut join = Writer(vec![11, 3, 11]);
    join.u64(id);
    join.u64(0);
    join.0.push(0);
    guest.rpc(&join.0).await;
    guest.message().await;
    owner.message().await;
    drop(guest);
    let left = owner.message().await;
    assert_eq!(left.kind, 2);
    assert_eq!(Reader::new(&left.body).u32().unwrap(), 10);
    assert_eq!(owner.rpc(&member_request(id)).await.0, 1);
    drop(owner);
    stop.send(()).unwrap();
    work.await.unwrap().unwrap();
}
