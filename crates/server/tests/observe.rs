use codol_server::app::config::Config;
use codol_server::diagnostics::observe::Observer;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{path::Path, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::oneshot,
};

fn config(path: &Path) -> Config {
    let mut config: Config = toml::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../configs/local.toml"
    )))
    .unwrap();
    config.capture_dir = path.into();
    for listener in &mut config.listeners {
        listener.bind.set_port(0);
    }
    config
}

fn end_events(directory: &Path) -> Vec<Value> {
    std::fs::read_dir(directory)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .map(|p| {
            let events = std::fs::read_to_string(p).unwrap();
            serde_json::from_str(events.lines().last().unwrap()).unwrap()
        })
        .collect()
}

async fn expect_eof(stream: &mut TcpStream) {
    let mut unexpected_response = Vec::new();
    tokio::time::timeout(
        Duration::from_secs(3),
        stream.read_to_end(&mut unexpected_response),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        unexpected_response.is_empty(),
        "observe mode must never fabricate a reply"
    );
}

#[tokio::test]
async fn captures_exact_stream_bytes_across_fragmented_writes() {
    let tmp = tempfile::tempdir().unwrap();
    let observer = Observer::bind(config(tmp.path())).await.unwrap();
    let address = observer.addresses().unwrap()[0].1;
    let directory = observer.directory().to_owned();
    let (stop, stopped) = oneshot::channel();
    let task = tokio::spawn(observer.run(async {
        let _ = stopped.await;
        Ok(())
    }));
    let mut client = TcpStream::connect(address).await.unwrap();
    // Synthetic byte stream, deliberately includes NUL and non-UTF8 bytes.
    let bytes: Vec<u8> = (0..32000).map(|i| (i % 256) as u8).collect();
    for fragment in bytes.chunks(997) {
        client.write_all(fragment).await.unwrap();
    }
    client.shutdown().await.unwrap();
    expect_eof(&mut client).await;
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
    let raw = std::fs::read(directory.join("000001-endpoint_a.bin")).unwrap();
    assert_eq!(raw, bytes);
    let ends = end_events(&directory);
    assert_eq!(ends.len(), 1);
    assert_eq!(ends[0]["reason"], "peer_eof");
    assert_eq!(ends[0]["bytes"], 32000);
    assert_eq!(ends[0]["sha256"], format!("{:x}", Sha256::digest(&bytes)));
    let events = std::fs::read_to_string(directory.join("000001-endpoint_a.jsonl")).unwrap();
    let mut offset = 0u64;
    for line in events.lines() {
        let event: Value = serde_json::from_str(line).unwrap();
        if event["event"] == "stream_chunk" {
            assert_eq!(event["offset"], offset);
            offset += event["length"].as_u64().unwrap();
        }
    }
    assert_eq!(offset, bytes.len() as u64);
}

#[tokio::test]
async fn shutdown_flushes_both_endpoint_sessions() {
    let tmp = tempfile::tempdir().unwrap();
    let observer = Observer::bind(config(tmp.path())).await.unwrap();
    let directory = observer.directory().to_owned();
    let addresses = observer.addresses().unwrap();
    let (stop, stopped) = oneshot::channel();
    let task = tokio::spawn(observer.run(async {
        let _ = stopped.await;
        Ok(())
    }));
    let mut clients = Vec::new();
    for (_, address) in addresses {
        let mut client = TcpStream::connect(address).await.unwrap();
        client.write_all(b"lab-input").await.unwrap();
        clients.push(client);
    }
    // Wait on observable capture events, not on a guessed scheduling delay.
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let count = std::fs::read_dir(&directory)
                .unwrap()
                .map(|e| e.unwrap().path())
                .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
                .filter(|p| std::fs::read_to_string(p).unwrap().contains("stream_chunk"))
                .count();
            if count == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    for mut client in clients {
        expect_eof(&mut client).await;
    }
    let events = end_events(&directory);
    assert_eq!(events.len(), 2);
    assert!(
        events
            .iter()
            .all(|e| e["reason"] == "shutdown" && e["bytes"] == 9)
    );
}

#[tokio::test]
async fn enforces_byte_budget_and_idle_timeout() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = config(tmp.path());
    cfg.max_bytes_per_session = 4;
    // Allow filesystem/thread scheduling while other transport tests run.
    cfg.idle_timeout_ms = 500;
    let observer = Observer::bind(cfg).await.unwrap();
    let address = observer.addresses().unwrap()[0].1;
    let directory = observer.directory().to_owned();
    let (stop, stopped) = oneshot::channel();
    let task = tokio::spawn(observer.run(async {
        let _ = stopped.await;
        Ok(())
    }));
    let mut client = TcpStream::connect(address).await.unwrap();
    client.write_all(b"abcd").await.unwrap();
    expect_eof(&mut client).await;
    let mut idle = TcpStream::connect(address).await.unwrap();
    expect_eof(&mut idle).await;
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
    let events = end_events(&directory);
    assert!(
        events
            .iter()
            .any(|e| e["reason"] == "byte_limit" && e["bytes"] == 4),
        "unexpected capture endings: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| e["reason"] == "idle_timeout" && e["bytes"] == 0)
    );
}

#[test]
fn rejects_unsafe_paths_unknown_keys_and_unbounded_settings() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = config(tmp.path());
    cfg.listeners[0].name = "../escape".into();
    assert!(cfg.validate().is_err());
    let mut cfg = config(tmp.path());
    cfg.max_connections = 0;
    assert!(cfg.validate().is_err());
    let mut cfg = config(tmp.path());
    cfg.listeners[1].name = cfg.listeners[0].name.clone();
    assert!(cfg.validate().is_err());
    assert!(
        toml::from_str::<Config>(&format!(
            "unknown = 1\n{}",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../configs/local.toml"
            ))
        ))
        .is_err()
    );
}

#[tokio::test]
async fn immediate_shutdown_does_not_lose_the_stop_signal() {
    let tmp = tempfile::tempdir().unwrap();
    let observer = Observer::bind(config(tmp.path())).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), observer.run(async { Ok(()) }))
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn capture_storage_failure_stops_the_runtime() {
    let tmp = tempfile::tempdir().unwrap();
    let observer = Observer::bind(config(tmp.path())).await.unwrap();
    let address = observer.addresses().unwrap()[0].1;
    // Remove only this test's private output directory before the first session.
    std::fs::remove_dir_all(observer.directory()).unwrap();
    let task = tokio::spawn(observer.run(std::future::pending()));
    let _client = TcpStream::connect(address).await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap();
    assert!(result.is_err());
}
