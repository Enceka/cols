use codol_protocol::lobby::{self, Direction};
use codol_server::{
    capture::unix_time_ms,
    lobby_server::LobbyServer,
    session::{Session, SessionRegistry},
};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::oneshot,
};

fn vector() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../fixtures/protocol/lobby210-synthetic.json"
    ))
    .unwrap()
}
fn bytes(v: &serde_json::Value, name: &str) -> Vec<u8> {
    let s = v[name].as_str().unwrap();
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[tokio::test]
async fn binds_issued_ticket_and_records_authenticated_application_data() {
    exercise_lobby(false).await;
}

#[tokio::test]
async fn local_services_preserve_fifo_after_unsupported_task() {
    exercise_lobby(true).await;
}

async fn exercise_lobby(local_services: bool) {
    let temp = tempfile::tempdir().unwrap();
    let data = vector();
    let key: [u8; 24] = bytes(&data, "key").try_into().unwrap();
    let mut auth = bytes(&data, "client_auth");
    let parsed = lobby::parse_client_auth(&auth).unwrap();
    let registry = Arc::new(SessionRegistry::default());
    registry
        .insert(
            parsed.server_ticket,
            Session {
                title_id: 1002,
                user_id: u64::MAX,
                iv_seed: 0x11223344,
                expires_at: (unix_time_ms() / 1000) as u32 + 3600,
                key,
            },
        )
        .unwrap();
    let folder = temp.path().join("lobby");
    let server = LobbyServer::bind_with_services(
        "127.0.0.1:0".parse().unwrap(),
        registry,
        folder.clone(),
        local_services.then(|| temp.path().join("data")),
    )
    .await
    .unwrap();
    let address = server.address().unwrap();
    let (stop, stopped) = oneshot::channel();
    let task = tokio::spawn(server.run(async {
        let _ = stopped.await;
        Ok(())
    }));
    let mut stream = TcpStream::connect(address).await.unwrap();
    let hello: [u8; 28] = bytes(&data, "client_hello").try_into().unwrap();
    for fragment in hello.chunks(3) {
        stream.write_all(fragment).await.unwrap();
    }
    let mut reply = [0; 26];
    stream.read_exact(&mut reply).await.unwrap();
    let secrets = lobby::derive_secrets(&hello, &reply, &auth, &key).unwrap();
    let tail = auth.len() - 8;
    auth[tail..].copy_from_slice(&secrets.client_proof);
    stream.write_all(&auth).await.unwrap();
    let mut ack = [0; 14];
    stream.read_exact(&mut ack).await.unwrap();
    assert_eq!(ack.to_vec(), lobby::frame(0x83, &secrets.server_proof));
    // A shape-correct synthetic registration receives only its status reply.
    let mut registration = vec![71, 3, 4];
    for value in [0x11, 0x22] {
        registration.extend_from_slice(&[19, 8, 16, 0, 0, 0]);
        registration.extend_from_slice(&[value; 16]);
    }
    registration.extend_from_slice(&[
        8, 0, 0, 0, 0, 19, 8, 0, 0, 0, 0, 19, 8, 0, 0, 0, 0, 16, 0, 0,
    ]);
    let register = lobby::encrypt_application(
        0x86,
        &registration,
        &secrets.data_keys,
        Direction::ClientToServer,
        1,
        [0x43; 16],
    )
    .unwrap();
    stream.write_all(&register).await.unwrap();
    let mut prefix = [0; 4];
    stream.read_exact(&mut prefix).await.unwrap();
    let mut wire = vec![0; 4 + u32::from_le_bytes(prefix) as usize];
    wire[..4].copy_from_slice(&prefix);
    stream.read_exact(&mut wire[4..]).await.unwrap();
    let reply = lobby::decrypt_application(&wire, &secrets.data_keys, Direction::ServerToClient, 1)
        .unwrap();
    assert_eq!(reply.kind, 1);
    assert_eq!(reply.body, lobby::registration_status(1));
    // Synthetic unknown service: it must be recorded, never answered with fake success.
    let frame = lobby::encrypt_application(
        0x86,
        &[10, 3, 7],
        &secrets.data_keys,
        Direction::ClientToServer,
        2,
        [0x42; 16],
    )
    .unwrap();
    stream.write_all(&frame).await.unwrap();
    if local_services {
        // Pipeline a supported request after an unsupported one. Skipping the
        // failure would shift every subsequent reply in the real client's FIFO.
        let mut publisher = codol_protocol::task::Writer(vec![10, 3, 21]);
        publisher.string("");
        publisher.string("configs_s.info");
        publisher.0.push(0);
        for (sequence, body) in [(3, &[12, 3, 6, 0][..]), (4, publisher.0.as_slice())] {
            let packet = lobby::encrypt_application(
                0x86,
                body,
                &secrets.data_keys,
                Direction::ClientToServer,
                sequence,
                [0x44; 16],
            )
            .unwrap();
            stream.write_all(&packet).await.unwrap();
        }
        for sequence in 2..=4 {
            let mut prefix = [0; 4];
            stream.read_exact(&mut prefix).await.unwrap();
            let mut wire = vec![0; 4 + u32::from_le_bytes(prefix) as usize];
            wire[..4].copy_from_slice(&prefix);
            stream.read_exact(&mut wire[4..]).await.unwrap();
            let message = lobby::decrypt_application(
                &wire,
                &secrets.data_keys,
                Direction::ServerToClient,
                sequence,
            )
            .unwrap();
            assert_eq!(message.kind, 1);
            match sequence {
                2 => assert_eq!(
                    message.body,
                    codol_protocol::task::reply(2, 7, Err(4)).unwrap()
                ),
                3 => {
                    assert_eq!(
                        &message.body[..26],
                        &[
                            10, 3, 0, 0, 0, 0, 0, 0, 0, 8, 0, 0, 0, 0, 3, 6, 8, 1, 0, 0, 0, 8, 1,
                            0, 0, 0
                        ]
                    );
                    assert_eq!(message.body[26], 8);
                    let time = u32::from_le_bytes(message.body[27..31].try_into().unwrap());
                    assert!(((unix_time_ms() / 1000) as u32).abs_diff(time) <= 3);
                }
                4 => assert_eq!(
                    message.body,
                    codol_protocol::task::reply(4, 21, Err(1000)).unwrap()
                ),
                _ => unreachable!(),
            }
        }
    }
    stream.shutdown().await.unwrap();
    let mut rest = Vec::new();
    tokio::time::timeout(Duration::from_secs(3), stream.read_to_end(&mut rest))
        .await
        .unwrap()
        .unwrap();
    assert!(rest.is_empty());
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
    let state: serde_json::Value =
        serde_json::from_slice(&std::fs::read(folder.join("000001/handshake.json")).unwrap())
            .unwrap();
    assert_eq!(state["transport_authenticated"], true);
    assert_eq!(state["user_id"], u64::MAX.to_string());
    assert_eq!(
        std::fs::read(folder.join("000001/packet-001-message.bin")).unwrap(),
        [10, 3, 7]
    );
}

#[tokio::test]
async fn an_unknown_ticket_never_receives_server_authentication() {
    let temp = tempfile::tempdir().unwrap();
    let data = vector();
    let server = LobbyServer::bind(
        "127.0.0.1:0".parse().unwrap(),
        Arc::new(SessionRegistry::default()),
        temp.path().join("lobby"),
    )
    .await
    .unwrap();
    let address = server.address().unwrap();
    let (stop, stopped) = oneshot::channel();
    let task = tokio::spawn(server.run(async {
        let _ = stopped.await;
        Ok(())
    }));
    let mut stream = TcpStream::connect(address).await.unwrap();
    stream
        .write_all(&bytes(&data, "client_hello"))
        .await
        .unwrap();
    let mut reply = [0; 26];
    stream.read_exact(&mut reply).await.unwrap();
    stream
        .write_all(&bytes(&data, "client_auth"))
        .await
        .unwrap();
    let mut reply = Vec::new();
    tokio::time::timeout(Duration::from_secs(3), stream.read_to_end(&mut reply))
        .await
        .unwrap()
        .unwrap();
    assert!(reply.is_empty());
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
}
