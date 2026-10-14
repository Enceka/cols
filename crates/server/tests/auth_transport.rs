use codol_server::transport::auth::AuthOptions;
use codol_server::transport::auth::AuthServer;
use openssl::{
    asn1::Asn1Time,
    hash::MessageDigest,
    pkey::PKey,
    rsa::Rsa,
    ssl::{SslConnector, SslMethod, SslVerifyMode, SslVersion},
    x509::{X509, X509NameBuilder},
};
use std::{path::Path, pin::Pin, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::oneshot,
};
use tokio_openssl::SslStream;

fn options(folder: &Path, fixture: bool) -> AuthOptions {
    let key = PKey::from_rsa(Rsa::generate(2048).unwrap()).unwrap();
    let mut name = X509NameBuilder::new().unwrap();
    name.append_entry_by_text("CN", "localhost").unwrap();
    let name = name.build();
    let mut certificate = X509::builder().unwrap();
    certificate.set_version(2).unwrap();
    certificate.set_subject_name(&name).unwrap();
    certificate.set_issuer_name(&name).unwrap();
    certificate.set_pubkey(&key).unwrap();
    certificate
        .set_not_before(&Asn1Time::days_from_now(0).unwrap())
        .unwrap();
    certificate
        .set_not_after(&Asn1Time::days_from_now(1).unwrap())
        .unwrap();
    certificate.sign(&key, MessageDigest::sha256()).unwrap();
    let cert_path = folder.join("certificate.pem");
    let key_path = folder.join("key.pem");
    std::fs::write(&cert_path, certificate.build().to_pem().unwrap()).unwrap();
    std::fs::write(&key_path, key.private_key_to_pem_pkcs8().unwrap()).unwrap();
    let reply = folder.join("reply.json");
    std::fs::write(&reply, br#"{"auth_task":"11","code":"701"}"#).unwrap();
    AuthOptions {
        bind: "127.0.0.1:0".parse().unwrap(),
        certificate: cert_path,
        key: key_path,
        capture_dir: folder.join("captures"),
        reply_file: fixture.then_some(reply),
        local_title_id: None,
    }
}

async fn connect(address: std::net::SocketAddr) -> SslStream<TcpStream> {
    let mut builder = SslConnector::builder(SslMethod::tls_client()).unwrap();
    builder
        .set_min_proto_version(Some(SslVersion::TLS1))
        .unwrap();
    builder
        .set_max_proto_version(Some(SslVersion::TLS1))
        .unwrap();
    builder.set_security_level(0);
    builder.set_cipher_list("AES128-SHA").unwrap();
    // Test-only self-signed endpoint. Real CODOL experiments use the lab CA.
    builder.set_verify(SslVerifyMode::NONE);
    let ssl = builder
        .build()
        .configure()
        .unwrap()
        .into_ssl("localhost")
        .unwrap();
    let mut stream = SslStream::new(ssl, TcpStream::connect(address).await.unwrap()).unwrap();
    Pin::new(&mut stream).connect().await.unwrap();
    stream
}

#[tokio::test]
async fn tls_continue_fragmented_body_and_fixture_response() {
    let temp = tempfile::tempdir().unwrap();
    let server = AuthServer::bind(options(temp.path(), true)).await.unwrap();
    let address = server.address().unwrap();
    let directory = server.directory().to_owned();
    let (stop, stopped) = oneshot::channel();
    let task = tokio::spawn(server.run(async {
        let _ = stopped.await;
        Ok(())
    }));
    let mut stream = connect(address).await;
    let body = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/protocol/auth-task10-sanitized.json"
    ));
    let header = format!(
        "POST /auth/ HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nExpect: 100-continue\r\nX-TransactionID: 1\r\n\r\n",
        body.len()
    );
    for chunk in header.as_bytes().chunks(7) {
        stream.write_all(chunk).await.unwrap();
    }
    let mut interim = [0u8; 25];
    stream.read_exact(&mut interim).await.unwrap();
    assert_eq!(&interim, b"HTTP/1.1 100 Continue\r\n\r\n");
    for chunk in body.chunks(9) {
        stream.write_all(chunk).await.unwrap();
    }
    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(3), stream.read_to_end(&mut response))
        .await
        .unwrap()
        .unwrap();
    assert!(response.starts_with(b"HTTP/1.1 200 OK\r\n"));
    assert!(response.ends_with(br#"{"auth_task":"11","code":"701"}"#));
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(
        std::fs::read(directory.join("000001-request.json")).unwrap(),
        body
    );
    let result: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("000001-session.json")).unwrap())
            .unwrap();
    assert_eq!(result["http_status"], 200);
    assert_eq!(result["tls_version"], "TLSv1");
    assert_eq!(result["title_id"], 1002);
}

#[tokio::test]
async fn rejects_ambiguous_lengths_and_recovers_for_next_connection() {
    let temp = tempfile::tempdir().unwrap();
    let server = AuthServer::bind(options(temp.path(), false)).await.unwrap();
    let address = server.address().unwrap();
    let (stop, stopped) = oneshot::channel();
    let task = tokio::spawn(server.run(async {
        let _ = stopped.await;
        Ok(())
    }));
    for (lengths, expected) in [
        ("Content-Length: 3\r\nContent-Length: 4\r\n", "400"),
        ("Content-Length: 16385\r\n", "413"),
        ("Content-Length: 3\r\nTransfer-Encoding: chunked\r\n", "400"),
    ] {
        let mut stream = connect(address).await;
        stream.write_all(format!("POST /auth/ HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\n{lengths}Expect: 100-continue\r\n\r\n").as_bytes()).await.unwrap();
        let mut response = Vec::new();
        tokio::time::timeout(Duration::from_secs(3), stream.read_to_end(&mut response))
            .await
            .unwrap()
            .unwrap();
        assert!(response.starts_with(format!("HTTP/1.1 {expected}").as_bytes()));
        assert!(!response.windows(12).any(|s| s == b"100 Continue"));
    }
    let mut stream = connect(address).await;
    let body = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/protocol/auth-task10-sanitized.json"
    ));
    let header = format!(
        "POST /auth/ HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes()).await.unwrap();
    stream.write_all(body).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    assert!(response.starts_with(b"HTTP/1.1 501"));
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn shutdown_cancels_an_incomplete_tls_handshake() {
    let temp = tempfile::tempdir().unwrap();
    let server = AuthServer::bind(options(temp.path(), false)).await.unwrap();
    let address = server.address().unwrap();
    let (stop, stopped) = oneshot::channel();
    let task = tokio::spawn(server.run(async {
        let _ = stopped.await;
        Ok(())
    }));
    let _client = TcpStream::connect(address).await.unwrap();
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn local_tickets_bind_request_identity_and_use_fresh_session_keys() {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let temp = tempfile::tempdir().unwrap();
    let mut settings = options(temp.path(), false);
    settings.local_title_id = Some(1002);
    let data = temp.path().join("players");
    let account = data.join("profiles/1002/2");
    std::fs::create_dir_all(&account).unwrap();
    std::fs::write(account.join("player.json"), serde_json::to_vec(&serde_json::json!({
        "schema_version":1,"client_sha256":codol_protocol::CLIENT_SHA256,"title_id":1002,"user_id":"2",
        "player_id":"11111111111111111111111111111111", "settings":{"nickname":"同名玩家","level":37,"unlock_all":true}
    })).unwrap()).unwrap();
    let server = AuthServer::bind(settings)
        .await
        .unwrap()
        .with_local_players(Some(&data))
        .await
        .unwrap();
    let address = server.address().unwrap();
    let (stop, stopped) = oneshot::channel();
    let task = tokio::spawn(server.run(async {
        let _ = stopped.await;
        Ok(())
    }));
    let body = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/protocol/auth-task10-sanitized.json"
    ));
    let mut keys = Vec::new();
    for _ in 0..2 {
        let mut stream = connect(address).await;
        stream.write_all(format!("POST /auth/ HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",body.len()).as_bytes()).await.unwrap();
        stream.write_all(body).await.unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        let start = response.windows(4).position(|v| v == b"\r\n\r\n").unwrap() + 4;
        let response: serde_json::Value = serde_json::from_slice(&response[start..]).unwrap();
        assert_eq!(response["auth_task"], "11");
        assert_eq!(response["code"], "700");
        let bytes: [u8; 128] = STANDARD
            .decode(response["client_ticket"].as_str().unwrap())
            .unwrap()
            .try_into()
            .unwrap();
        let plain = codol_protocol::auth::ticket::decrypt_ticket(&bytes, 1).unwrap();
        assert_eq!(&plain[5..9], &1002u32.to_le_bytes());
        assert_eq!(&plain[25..33], &2u64.to_le_bytes());
        let end = plain[33..97].iter().position(|&byte| byte == 0).unwrap();
        assert_eq!(
            STANDARD.decode(&plain[33..33 + end]).unwrap(),
            "同名玩家".as_bytes()
        );
        keys.push(plain[97..121].to_vec());
    }
    assert_ne!(keys[0], keys[1]);
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
}
