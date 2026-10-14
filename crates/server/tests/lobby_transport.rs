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

async fn receive(
    stream: &mut TcpStream,
    keys: &[u8; 72],
    sequence: u32,
) -> lobby::ApplicationMessage {
    let mut prefix = [0; 4];
    stream.read_exact(&mut prefix).await.unwrap();
    let mut wire = vec![0; 4 + u32::from_le_bytes(prefix) as usize];
    wire[..4].copy_from_slice(&prefix);
    stream.read_exact(&mut wire[4..]).await.unwrap();
    lobby::decrypt_application(&wire, keys, Direction::ServerToClient, sequence).unwrap()
}

#[tokio::test]
async fn binds_issued_ticket_and_records_authenticated_application_data() {
    exercise_lobby(false, false).await;
}

#[tokio::test]
async fn local_services_preserve_fifo_after_unsupported_task() {
    exercise_lobby(true, false).await;
}

#[tokio::test]
async fn continuous_play_keeps_authenticated_fifo_after_capture_byte_and_frame_budgets() {
    exercise_lobby(true, true).await;
}

async fn exercise_lobby(local_services: bool, continuous: bool) {
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
    let mut server = LobbyServer::bind_with_services(
        "127.0.0.1:0".parse().unwrap(),
        registry,
        folder.clone(),
        local_services.then(|| temp.path().join("data")),
    )
    .await
    .unwrap();
    if continuous {
        server = server.with_continuous_play().unwrap();
    }
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
        // A channel message emits an ACK and an unsolicited push. The next
        // RPC keeps transaction 7 even though its encrypted frame is sequence 8.
        let create = lobby::encrypt_application(
            0x86,
            &[11, 3, 13, 0],
            &secrets.data_keys,
            Direction::ClientToServer,
            5,
            [0x45; 16],
        )
        .unwrap();
        stream.write_all(&create).await.unwrap();
        let created = receive(&mut stream, &secrets.data_keys, 5).await;
        let channel = codol_protocol::task::Reader::new(&created.body[26..])
            .u64()
            .unwrap();
        let payload = [1, 0, 0, 0, 2];
        let mut msg = codol_protocol::task::Writer(vec![11, 3, 4]);
        msg.u64(channel);
        msg.blob(&payload).unwrap();
        msg.u32(1);
        msg.0.push(0);
        for (sequence, body) in [(6, msg.0.as_slice()), (7, &[12, 3, 6, 0][..])] {
            stream
                .write_all(
                    &lobby::encrypt_application(
                        0x86,
                        body,
                        &secrets.data_keys,
                        Direction::ClientToServer,
                        sequence,
                        [0x46; 16],
                    )
                    .unwrap(),
                )
                .await
                .unwrap();
        }
        let ack = receive(&mut stream, &secrets.data_keys, 6).await;
        assert_eq!(ack.kind, 1);
        assert_eq!(
            ack.body,
            codol_protocol::task::reply(6, 4, Ok(&[])).unwrap()
        );
        let push = receive(&mut stream, &secrets.data_keys, 7).await;
        assert_eq!(push.kind, 2);
        let mut r = codol_protocol::task::Reader::new(&push.body);
        assert_eq!(r.u32().unwrap(), 7);
        assert_eq!(r.u64().unwrap(), u64::MAX);
        assert_eq!(r.string(65).unwrap(), "MTg0NDY3NDQwNzM3MDk1NTE2MTU=");
        assert_eq!(r.u64().unwrap(), channel);
        assert_eq!(r.blob().unwrap(), payload);
        assert_eq!(r.u32().unwrap(), 1);
        assert!(r.remaining().is_empty());
        let next = receive(&mut stream, &secrets.data_keys, 8).await;
        assert_eq!(next.kind, 1);
        assert_eq!(&next.body[..9], &[10, 7, 0, 0, 0, 0, 0, 0, 0]);

        // A complete catalogue exceeds the former 64 KiB server frame cap.
        // Exercise both nested player data and standalone refresh entries
        // through real TCP/AES/MAC framing, including sentinel owner fields.
        let items: Vec<u32> = (1..=6772).collect();
        let account = temp
            .path()
            .join("data/profiles/1002")
            .join(u64::MAX.to_string());
        std::fs::create_dir_all(&account).unwrap();
        std::fs::write(
            account.join("inventory.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "schema_version":1,"client_sha256":codol_protocol::CLIENT_SHA256,
                "title_id":1002,"user_id":u64::MAX.to_string(),"items":items
            }))
            .unwrap(),
        )
        .unwrap();
        let mut refresh = codol_protocol::task::Writer(vec![80, 3, 203]);
        refresh.string("");
        refresh.u16(20000);
        refresh.0.push(0);
        for (sequence, opcode, request, records) in [
            (
                8,
                230,
                vec![80, 3, 230, 16, 0, 0],
                vec![codol_protocol::task::player_data(u64::MAX, &items).unwrap()],
            ),
            (
                9,
                203,
                refresh.0,
                codol_protocol::task::inventory_records(u64::MAX, &items).unwrap(),
            ),
        ] {
            let packet = lobby::encrypt_application(
                0x86,
                &request,
                &secrets.data_keys,
                Direction::ClientToServer,
                sequence,
                [0x47; 16],
            )
            .unwrap();
            stream.write_all(&packet).await.unwrap();
            let response = receive(&mut stream, &secrets.data_keys, sequence + 1).await;
            assert_eq!(response.kind, 1);
            assert!(response.body.len() > 65536);
            assert_eq!(
                response.body,
                codol_protocol::task::reply(u64::from(sequence), opcode, Ok(&records)).unwrap()
            );
        }
    }
    if continuous {
        let mut unknown = vec![250, 3, 1];
        unknown.resize(60_000, 0);
        for sequence in 10..28 {
            let packet = lobby::encrypt_application(
                0x86,
                &unknown,
                &secrets.data_keys,
                Direction::ClientToServer,
                sequence,
                [0x48; 16],
            )
            .unwrap();
            stream.write_all(&packet).await.unwrap();
            let response = receive(&mut stream, &secrets.data_keys, sequence + 1).await;
            assert_eq!(
                response.body,
                codol_protocol::task::reply(u64::from(sequence), 1, Err(4)).unwrap()
            );
        }
        // Cross the diagnostic frame cap without consuming crypto/RPC sequence
        // numbers. Capture stops at its byte budget; authenticated play does not.
        let keepalives = vec![0; 1050 * 4];
        stream.write_all(&keepalives).await.unwrap();
        let mut replies = vec![0xff; keepalives.len()];
        stream.read_exact(&mut replies).await.unwrap();
        assert_eq!(replies, keepalives);
        stream
            .write_all(
                &lobby::encrypt_application(
                    0x86,
                    &[12, 3, 6, 0],
                    &secrets.data_keys,
                    Direction::ClientToServer,
                    28,
                    [0x49; 16],
                )
                .unwrap(),
            )
            .await
            .unwrap();
        let response = receive(&mut stream, &secrets.data_keys, 29).await;
        assert_eq!(&response.body[..9], &[10, 28, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(&response.body[9..14], &[8, 0, 0, 0, 0]);
        assert!(!folder.join("000001/packet-1077-message.bin").exists());
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
