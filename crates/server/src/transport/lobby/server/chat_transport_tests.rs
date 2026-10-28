use super::room_transport_tests::{Peer, broadcast};
use super::*;
use codol_protocol::lobby::task::Reader;
use codol_protocol::lobby::task::Writer;

fn groups_body(added: &[(u8, u64)], removed: &[(u8, u64)]) -> Vec<u8> {
    let mut w = Writer(vec![102, 3, 1]);
    w.u32(removed.len() as u32);
    for &(kind, id) in removed {
        w.u8(kind);
        w.u64(id);
    }
    w.u32(added.len() as u32);
    for &(kind, id) in added {
        w.u8(kind);
        w.u64(id);
    }
    w.0.push(0);
    w.0
}

fn text_body(group: u64, text: &[u8]) -> Vec<u8> {
    let mut payload = vec![0; 4];
    payload.extend_from_slice(&(text.len() as u32).to_le_bytes());
    payload.extend_from_slice(text);
    let mut w = Writer(vec![102, 3, 3]);
    w.u8(130);
    w.u64(group);
    w.blob(&payload).unwrap();
    w.0.push(0);
    w.0
}

#[tokio::test]
async fn newbie_subscriber_receives_public_text_sent_to_the_public_group() {
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
    let address = lobby.listener.local_addr().unwrap();
    let (stop, wait) = tokio::sync::oneshot::channel();
    let work = tokio::spawn(lobby.run(async {
        let _ = wait.await;
        Ok(())
    }));
    let mut sender = Peer::connect(address, &registry, 51).await;
    let mut reader = Peer::connect(address, &registry, 52).await;
    assert_eq!(reader.rpc(&groups_body(&[(130, 99)], &[])).await.0, 1);
    assert_eq!(sender.rpc(&text_body(10, b"tUtoR0hello")).await.0, 0);
    let delivered = reader.message().await;
    assert_eq!(delivered.kind, 2);
    let mut r = Reader::new(&delivered.body);
    // Event 34 carries the sender, its base64 nickname, and the forwarded payload.
    assert_eq!(r.u32().unwrap(), 34);
    assert_eq!(r.u64().unwrap(), 51);
    assert_eq!(r.string(64).unwrap(), "NTE=");
    assert_eq!(
        r.blob().unwrap(),
        [
            0, 0, 0, 0, 11, 0, 0, 0, 116, 85, 116, 111, 82, 48, 104, 101, 108, 108, 111
        ]
    );
    stop.send(()).unwrap();
    work.await.unwrap().unwrap();
}

struct Bits(Vec<u8>, usize);
impl Bits {
    fn new() -> Self {
        Self(Vec::new(), 0)
    }
    fn write(&mut self, value: u64, count: usize) {
        for i in 0..count {
            if self.1.is_multiple_of(8) {
                self.0.push(0);
            }
            if (value >> i) & 1 == 1 {
                let last = self.0.len() - 1;
                self.0[last] |= 1 << (self.1 % 8);
            }
            self.1 += 1;
        }
    }
}

fn member_record(user: u64, team: u8) -> Vec<u8> {
    // Type10 member metadata: one tag bit, the record type, the member id and
    // team, then the native filler fields the parser consumes.
    let mut bits = Bits::new();
    bits.write(0, 1);
    bits.write(10, 8);
    bits.write(user, 64);
    bits.write(u64::from(team), 8);
    bits.write(0, 1);
    for count in [16, 16, 8, 8, 64, 32, 1, 8, 8, 64, 8, 32, 32] {
        bits.write(0, count);
    }
    bits.0
}

fn batch_of(records: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for record in records {
        out.extend_from_slice(&(record.len() as u32).to_le_bytes());
        out.extend_from_slice(record);
    }
    out
}

fn broadcast_text(record: &[u8], channel: u64, delivery: u32) -> Vec<u8> {
    let mut w = Writer::default();
    w.0.extend_from_slice(&[11, 3, 4]);
    w.u64(channel);
    w.blob(record).unwrap();
    w.u32(delivery);
    w.0.push(0);
    w.0
}

fn directed_text(record: &[u8], channel: u64, recipient: u64, delivery: u32) -> Vec<u8> {
    let mut w = Writer::default();
    w.0.extend_from_slice(&[11, 3, 5]);
    w.u64(channel);
    w.u64(recipient);
    w.blob(record).unwrap();
    w.u32(delivery);
    w.0.push(0);
    w.0
}

fn synthetic(index: usize) -> Vec<u8> {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/protocol/chat-3.3.3.3.json"
    )))
    .unwrap();
    let hex = fixture["room_text"]["synthetic"][index]["batch_hex"]
        .as_str()
        .unwrap();
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}

fn room_text_push(body: &[u8]) -> (u64, u64, Vec<u8>) {
    let mut r = Reader::new(body);
    assert_eq!(r.u32().unwrap(), 7);
    let sender = r.u64().unwrap();
    let _name = r.string(64).unwrap();
    let channel = r.u64().unwrap();
    let payload = r.blob().unwrap().to_vec();
    (sender, channel, payload)
}

#[tokio::test]
async fn room_team_text_reaches_only_the_senders_team_and_directed_text_only_its_recipient() {
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
    let address = lobby.listener.local_addr().unwrap();
    let (stop, wait) = tokio::sync::oneshot::channel();
    let work = tokio::spawn(lobby.run(async {
        let _ = wait.await;
        Ok(())
    }));
    let mut owner = Peer::connect(address, &registry, 61).await;
    let channel = owner.create_room().await;
    let mut guest = Peer::connect(address, &registry, 62).await;
    let listing: serde_json::Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/protocol/room-browser-synthetic.json"
    )))
    .unwrap();
    guest
        .rpc(&hex(listing["requests"]["first_page"].as_str().unwrap()))
        .await;
    let mut join = Writer::default();
    join.0.extend_from_slice(&[11, 3, 11]);
    join.u64(channel);
    join.u64(0);
    join.0.push(0);
    guest.rpc(&join.0).await;
    guest.message().await;
    owner.message().await;
    // The owner publishes both member teams before any room text.
    let roster = batch_of(&[member_record(61, 2), member_record(62, 1)]);
    owner.rpc(&broadcast(channel, &roster)).await;
    let (sender, _, payload) = room_text_push(&owner.message().await.body);
    assert_eq!(sender, 61);
    assert_eq!(payload, roster);
    guest.message().await;

    // Team text from the team2 owner reaches the owner and skips team1 guest.
    let team_text = synthetic(1);
    owner.rpc(&broadcast_text(&team_text, channel, 1)).await;
    let (sender, _, payload) = room_text_push(&owner.message().await.body);
    assert_eq!(sender, 61);
    assert_eq!(payload, team_text);
    // The guest echo below is its own ordinary text, so the team text never
    // reached it.
    let plain = synthetic(0);
    guest.rpc(&broadcast_text(&plain, channel, 2)).await;
    let (sender, _, payload) = room_text_push(&guest.message().await.body);
    assert_eq!(sender, 62);
    assert_eq!(payload, plain);
    let (sender, _, payload) = room_text_push(&owner.message().await.body);
    assert_eq!(sender, 62);
    assert_eq!(payload, plain);

    // Directed text reaches only the named recipient.
    guest.rpc(&directed_text(&plain, channel, 61, 3)).await;
    let (sender, _, payload) = room_text_push(&owner.message().await.body);
    assert_eq!(sender, 62);
    assert_eq!(payload, plain);
    owner.rpc(&broadcast_text(&plain, channel, 4)).await;
    let (sender, _, payload) = room_text_push(&guest.message().await.body);
    assert_eq!(sender, 61);
    assert_eq!(payload, plain);
    room_text_push(&owner.message().await.body);
    stop.send(()).unwrap();
    work.await.unwrap().unwrap();
}

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}
