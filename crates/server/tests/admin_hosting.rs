//! The admin API drives real owned processes, using only synthetic map metadata.
use serde_json::{Value, json};
use std::{net::SocketAddr, path::Path, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream, UdpSocket},
    process::{Child, Command},
};

async fn launch(root: &Path) -> (Child, SocketAddr) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_codol-server"))
        .args(["admin", "--bind", "127.0.0.1:0", "--state-dir"])
        .arg(root.join("control"))
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    tokio::time::timeout(Duration::from_secs(10), output.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    let address = line
        .trim()
        .strip_prefix("CODOL Control: http://")
        .unwrap()
        .parse()
        .unwrap();
    (child, address)
}

async fn api(address: SocketAddr, method: &str, path: &str, body: Value) -> (u16, Value) {
    let body = if method == "GET" {
        String::new()
    } else {
        body.to_string()
    };
    let mut socket = TcpStream::connect(address).await.unwrap();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    socket.write_all(request.as_bytes()).await.unwrap();
    let mut bytes = Vec::new();
    socket.read_to_end(&mut bytes).await.unwrap();
    let response = String::from_utf8(bytes).unwrap();
    let (headers, body) = response.split_once("\r\n\r\n").unwrap();
    (
        headers.split_whitespace().nth(1).unwrap().parse().unwrap(),
        serde_json::from_str(body).unwrap(),
    )
}

async fn configure(address: SocketAddr, root: &Path, resource_port: u16) {
    std::fs::write(root.join("map.json"), serde_json::to_vec(&json!({
        "schema_version":1,"client_sha256":codol_protocol::CLIENT_SHA256,"capture_sha256":"0".repeat(64),
        "map":"mp_afghan","mode":"war","zone":"mp","playlist":0,"ncs_crc":0x12345678u32,
        "common_crc":1,"constant_count":2,"asset_count":1,"observed_crc_verified":false
    })).unwrap()).unwrap();
    std::fs::write(
        root.join("catalog.json"),
        br#"{"schema_version":1,"entries":[{"map":"mp_afghan","mode":"war","file":"map.json"}]}"#,
    )
    .unwrap();
    let (_, value) = api(address, "GET", "/api/hosting", Value::Null).await;
    let mut settings = value["settings"].clone();
    settings["service_data_dir"] = json!(root.join("accounts"));
    settings["game_state"] = json!(root.join("map.json"));
    settings["map_catalog"] = json!(root.join("catalog.json"));
    for field in [
        "auth_port",
        "lobby_port",
        "stun_port",
        "stun_alt_port",
        "game_port",
        "resource_port",
        "room_bot_fill",
    ] {
        settings[field] = json!(0);
    }
    settings["resource_port"] = json!(resource_port);
    let (code, result) = api(
        address,
        "POST",
        "/api/hosting/config",
        json!({"settings":settings,"revision":value["revision"]}),
    )
    .await;
    assert_eq!(code, 200, "{result}");
}

async fn phase(address: SocketAddr, expected: &str) -> Value {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let (_, status) = api(address, "GET", "/api/hosting", Value::Null).await;
            if status["phase"] == expected {
                return status;
            }
            assert!(status["phase"] != "failed", "{status}");
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    })
    .await
    .unwrap()
}

async fn assert_released(endpoints: &Value) {
    for name in ["auth", "lobby", "resources"] {
        let address: SocketAddr = endpoints[name].as_str().unwrap().parse().unwrap();
        let listener = TcpListener::bind(address).await.unwrap();
        drop(listener);
    }
    for name in ["stun", "stun_alt", "game"] {
        let address: SocketAddr = endpoints[name].as_str().unwrap().parse().unwrap();
        let socket = UdpSocket::bind(address).await.unwrap();
        drop(socket);
    }
}

#[tokio::test]
async fn one_click_creates_keys_starts_all_endpoints_and_restarts_without_resetting_data() {
    let root = tempfile::tempdir().unwrap();
    let (mut admin, address) = launch(root.path()).await;
    configure(address, root.path(), 0).await;
    let (a, b) = tokio::join!(
        api(address, "POST", "/api/hosting/start", json!({})),
        api(address, "POST", "/api/hosting/start", json!({}))
    );
    assert_eq!(a.0, 200);
    assert_eq!(b.0, 200);
    let running = phase(address, "running").await;
    let endpoints = &running["endpoints"];
    assert_eq!(endpoints.as_object().unwrap().len(), 6);
    for name in ["auth", "lobby", "resources"] {
        assert!(
            TcpStream::connect(endpoints[name].as_str().unwrap())
                .await
                .is_ok()
        );
    }
    let (_, certificate) = api(address, "GET", "/api/hosting/certificate", Value::Null).await;
    assert!(
        certificate["pem"]
            .as_str()
            .unwrap()
            .starts_with("-----BEGIN CERTIFICATE-----")
    );
    assert!(!certificate.to_string().contains("PRIVATE KEY"));
    assert_eq!(
        api(address, "POST", "/api/hosting/certificate", json!({}))
            .await
            .0,
        422
    );
    let (_, unchanged) = api(address, "POST", "/api/hosting/start", json!({})).await;
    assert_eq!(unchanged["endpoints"], running["endpoints"]);
    std::fs::write(
        root.path().join("accounts/profiles/preserve"),
        b"existing player",
    )
    .unwrap();
    assert_eq!(
        api(address, "POST", "/api/hosting/stop", json!({})).await.0,
        200
    );
    phase(address, "stopped").await;
    assert_released(endpoints).await;
    api(address, "POST", "/api/hosting/start", json!({})).await;
    let restarted = phase(address, "running").await;
    assert_eq!(
        restarted["certificate"]["details"]["fingerprint"],
        certificate["fingerprint"]
    );
    assert_eq!(
        std::fs::read(root.path().join("accounts/profiles/preserve")).unwrap(),
        b"existing player"
    );
    api(address, "POST", "/api/hosting/stop", json!({})).await;
    phase(address, "stopped").await;
    assert_released(&restarted["endpoints"]).await;
    admin.kill().await.unwrap();
}

#[tokio::test]
async fn occupied_resource_port_fails_without_leaving_auth_or_game_running_and_can_retry() {
    let root = tempfile::tempdir().unwrap();
    let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (mut admin, address) = launch(root.path()).await;
    configure(address, root.path(), occupied.local_addr().unwrap().port()).await;
    api(address, "POST", "/api/hosting/start", json!({})).await;
    let failed = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let (_, state) = api(address, "GET", "/api/hosting", Value::Null).await;
            if state["phase"] == "failed" {
                break state;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    assert!(failed["error"].is_string());
    let (_, logs) = api(address, "GET", "/api/logs", Value::Null).await;
    assert!(
        logs["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|line| line["source"] == "server")
    );
    drop(occupied);
    api(address, "POST", "/api/hosting/start", json!({})).await;
    let running = phase(address, "running").await;
    api(address, "POST", "/api/hosting/stop", json!({})).await;
    phase(address, "stopped").await;
    assert_released(&running["endpoints"]).await;
    admin.kill().await.unwrap();
}

#[tokio::test]
async fn cancelling_startup_does_not_publish_running_endpoints() {
    let root = tempfile::tempdir().unwrap();
    let (mut admin, address) = launch(root.path()).await;
    configure(address, root.path(), 0).await;
    api(address, "POST", "/api/hosting/start", json!({})).await;
    api(address, "POST", "/api/hosting/stop", json!({})).await;
    let stopped = phase(address, "stopped").await;
    assert_eq!(stopped["endpoints"], json!({}));
    api(address, "POST", "/api/hosting/start", json!({})).await;
    let running = phase(address, "running").await;
    api(address, "POST", "/api/hosting/stop", json!({})).await;
    phase(address, "stopped").await;
    assert_released(&running["endpoints"]).await;
    admin.kill().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn stopping_the_panel_cooperatively_releases_its_services() {
    let root = tempfile::tempdir().unwrap();
    let (mut admin, address) = launch(root.path()).await;
    configure(address, root.path(), 0).await;
    api(address, "POST", "/api/hosting/start", json!({})).await;
    let running = phase(address, "running").await;
    assert!(
        std::process::Command::new("kill")
            .args(["-INT", &admin.id().unwrap().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let status = tokio::time::timeout(Duration::from_secs(15), admin.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success());
    assert_released(&running["endpoints"]).await;
}

#[tokio::test]
async fn preparing_local_resources_adopts_only_validated_paths_and_preserves_other_settings() {
    let root = tempfile::tempdir().unwrap();
    let (mut admin, address) = launch(root.path()).await;
    configure(address, root.path(), 0).await;
    let (_, before) = api(address, "GET", "/api/hosting", Value::Null).await;
    let request = json!({"kind":"catalog","source":root.path().join("catalog.json"),"output_parent":root.path().join("resources")});
    let (code, _) = api(address, "POST", "/api/resources/prepare", request.clone()).await;
    assert_eq!(code, 200);
    let ready = phase(address, "stopped").await;
    assert_eq!(ready["resources"]["result"]["maps"], 1);
    let mut expected = before["settings"].clone();
    expected["game_state"] = ready["settings"]["game_state"].clone();
    expected["map_catalog"] = ready["settings"]["map_catalog"].clone();
    assert_eq!(ready["settings"], expected);
    assert!(Path::new(ready["settings"]["map_catalog"].as_str().unwrap()).is_file());
    std::fs::write(root.path().join("catalog.json"), b"{}").unwrap();
    assert_eq!(
        api(address, "POST", "/api/resources/prepare", request)
            .await
            .0,
        200
    );
    let failed = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let (_, value) = api(address, "GET", "/api/hosting", Value::Null).await;
            if value["phase"] == "failed" {
                break value;
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(failed["settings"], ready["settings"]);
    assert!(failed["error"].is_string());
    assert!(!failed["resources"]["result"].is_object());
    admin.kill().await.unwrap();
}
