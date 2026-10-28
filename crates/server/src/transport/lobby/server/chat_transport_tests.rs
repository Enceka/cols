use super::room_transport_tests::Peer;
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
