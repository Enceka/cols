use codol_server::stun_server::StunServer;
use std::time::Duration;
use tokio::{net::UdpSocket, sync::oneshot};

async fn discovery(
    client: &UdpSocket,
    server: std::net::SocketAddr,
    request: &[u8],
    buffer: &mut [u8],
) -> (usize, std::net::SocketAddr) {
    // UDP has no delivery guarantee, including the first datagram on macOS.
    // Retry the identical idempotent discovery, without changing each reply
    // deadline; a dead service or the old512-request cutoff still fails.
    for _ in 0..3 {
        client.send_to(request, server).await.unwrap();
        if let Ok(reply) =
            tokio::time::timeout(Duration::from_secs(2), client.recv_from(buffer)).await
        {
            return reply.unwrap();
        }
    }
    panic!("STUN discovery exhausted three attempts");
}

#[tokio::test]
async fn observed_version_three_and_changed_port_requests_use_real_endpoints() {
    let temp = tempfile::tempdir().unwrap();
    let server = StunServer::bind(
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1:0".parse().unwrap(),
        temp.path().join("stun"),
    )
    .await
    .unwrap();
    let primary = server.address().unwrap();
    let (stop, stopped) = oneshot::channel();
    let task = tokio::spawn(server.run(async {
        let _ = stopped.await;
        Ok(())
    }));
    let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut buffer = [0; 64];
    let (n, from) = discovery(&client, primary, &[30, 3, 0], &mut buffer).await;
    assert_eq!(n, 15);
    assert_eq!(from, primary);
    assert_eq!(&buffer[..3], &[31, 3, 0]);
    assert_eq!(
        u16::from_le_bytes([buffer[7], buffer[8]]),
        client.local_addr().unwrap().port()
    );
    let (_, from) = discovery(&client, primary, &[20, 2, 0, 0], &mut buffer).await;
    assert_eq!(from, primary);
    let alternate_port = u16::from_le_bytes([buffer[13], buffer[14]]);
    let (_, from) = discovery(&client, primary, &[20, 2, 0, 2], &mut buffer).await;
    assert_eq!(from.port(), alternate_port);
    assert_ne!(from.port(), primary.port());
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn discovery_survives_capture_failure_and_requests_after_the_recording_limit() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("stun");
    let server = StunServer::bind(
        "127.0.0.1:0".parse().unwrap(),
        "127.0.0.1:0".parse().unwrap(),
        directory.clone(),
    )
    .await
    .unwrap();
    // Deterministic write failure even when the test runs with full privileges.
    std::fs::remove_dir(&directory).unwrap();
    std::fs::write(&directory, b"not a directory").unwrap();
    let primary = server.address().unwrap();
    let (stop, stopped) = oneshot::channel();
    let task = tokio::spawn(server.run(async {
        let _ = stopped.await;
        Ok(())
    }));
    let client = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut buffer = [0u8; 64];
    for index in 0..520 {
        let request = if index < 515 {
            vec![30, 3, 0]
        } else {
            vec![20, 2, 0, 2]
        };
        let (size, from) = discovery(&client, primary, &request, &mut buffer).await;
        assert_eq!(size, 15);
        if index < 515 {
            assert_eq!(buffer[0], 31);
            assert_eq!(from, primary);
        } else {
            assert_eq!(buffer[0], 21);
            assert_ne!(from.port(), primary.port());
        }
    }
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(std::fs::read(directory).unwrap(), b"not a directory");
}
