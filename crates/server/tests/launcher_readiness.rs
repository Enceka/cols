//! Exercise the executable contract without commercial client assets.
use openssl::{
    asn1::Asn1Time,
    hash::MessageDigest,
    pkey::PKey,
    rsa::Rsa,
    x509::{X509, X509NameBuilder},
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};

struct OwnedChild(Child);

impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn certificate(root: &Path) {
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
    std::fs::write(root.join("cert.pem"), certificate.build().to_pem().unwrap()).unwrap();
    std::fs::write(
        root.join("key.pem"),
        key.private_key_to_pem_pkcs8().unwrap(),
    )
    .unwrap();
}

fn command(root: &Path, ready: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_codol-server"));
    command
        .current_dir(root)
        .env("RUST_LOG", "off")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
        .arg("serve-auth")
        .args([
            "--bind",
            "127.0.0.1:0",
            "--cert",
            "cert.pem",
            "--key",
            "key.pem",
            "--local-title-id",
            "1002",
            "--lobby-bind",
            "127.0.0.1:0",
            "--service-data-dir",
            "accounts",
            "--stun-bind",
            "127.0.0.1:0",
            "--stun-alt",
            "127.0.0.1:0",
            "--game-bind",
            "127.0.0.1:0",
            "--game-state",
            "map.json",
            "--game-map-catalog",
            "catalog.json",
            "--continuous-play",
            "--launcher-ready-file",
        ])
        .arg(ready);
    command
}

async fn wait_ready(child: &mut OwnedChild, path: &Path) -> Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "backend exited before readiness"
            );
            if path.is_file() {
                return serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn assembled_native_services_report_actual_addresses_even_without_logs() {
    let root = tempfile::tempdir().unwrap();
    certificate(root.path());
    std::fs::write(
        root.path().join("map.json"),
        serde_json::to_vec(&json!({
            "schema_version":1, "client_sha256":codol_protocol::CLIENT_SHA256,
            "capture_sha256":"0".repeat(64), "map":"mp_afghan", "mode":"war", "zone":"mp",
            "playlist":0, "ncs_crc":0x12345678u32, "common_crc":1, "constant_count":2,
            "asset_count":1, "observed_crc_verified":false
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        root.path().join("catalog.json"),
        br#"{"schema_version":1,"entries":[{"map":"mp_afghan","mode":"war","file":"map.json"}]}"#,
    )
    .unwrap();
    let ready = root.path().join("ready.json");
    let mut child = OwnedChild(command(root.path(), &ready).spawn().unwrap());
    let value = wait_ready(&mut child, &ready).await;
    assert_eq!(value["pid"], child.0.id());
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["kind"], "native_services");
    assert_eq!(value["client_sha256"], codol_protocol::CLIENT_SHA256);
    assert_eq!(value["lobby_protocol"], 210);
    assert_eq!(value["game_protocol"], 3);
    assert_eq!(value["title_id"], 1002);
    for name in ["auth", "lobby", "stun", "stun_alt", "game"] {
        let address: std::net::SocketAddr = value[name].as_str().unwrap().parse().unwrap();
        assert!(address.ip().is_loopback() && address.port() != 0, "{name}");
    }
    assert_eq!(value["continuous_play"], true);
    assert_eq!(value["game_state_loaded"], true);
    assert_eq!(value["map_catalog_loaded"], true);
    assert_eq!(value["native_bot_rooms"], false);
    child.0.kill().unwrap();
    child.0.wait().unwrap();

    // AUTH/lobby binding alone cannot produce a ready record for a failed catalog.
    let missing_ready = root.path().join("failed-ready.json");
    std::fs::write(
        root.path().join("catalog.json"),
        br#"{"schema_version":1,"entries":[]}"#,
    )
    .unwrap();
    let mut failed = OwnedChild(command(root.path(), &missing_ready).spawn().unwrap());
    let status = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(status) = failed.0.try_wait().unwrap() {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(!status.success());
    assert!(!missing_ready.exists());
}

#[tokio::test]
async fn observer_reports_bound_listener_without_creating_a_client_session() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("observer.toml");
    std::fs::write(&config, "capture_dir='captures'\nmax_connections=16\nmax_sessions_per_run=128\nmax_bytes_per_session=1048576\nmax_chunks_per_session=4096\nidle_timeout_ms=10000\nsession_timeout_ms=30000\n[[listeners]]\nname='resources'\nbind='127.0.0.1:0'\n").unwrap();
    let ready: PathBuf = root.path().join("ready.json");
    let mut child = OwnedChild(
        Command::new(env!("CARGO_BIN_EXE_codol-server"))
            .args(["observe", "--config"])
            .arg(config)
            .arg("--launcher-ready-file")
            .arg(&ready)
            .current_dir(root.path())
            .env("RUST_LOG", "off")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let value = wait_ready(&mut child, &ready).await;
    assert_eq!(value["kind"], "tcp_observer");
    assert_eq!(value["pid"], child.0.id());
    let address: std::net::SocketAddr = value["listeners"]["resources"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!(address.ip().is_loopback() && address.port() != 0);
    let run = std::fs::read_dir(root.path().join("captures"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(
        std::fs::read_dir(run).unwrap().count(),
        1,
        "no probing connections should be needed"
    );
    child.0.kill().unwrap();
    child.0.wait().unwrap();
}
