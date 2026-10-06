use codol_server::stun_server::StunServer;
use std::time::Duration;
use tokio::{net::UdpSocket, sync::oneshot};

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
    client.send_to(&[30, 3, 0], primary).await.unwrap();
    let (n, from) = tokio::time::timeout(Duration::from_secs(2), client.recv_from(&mut buffer))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(n, 15);
    assert_eq!(from, primary);
    assert_eq!(&buffer[..3], &[31, 3, 0]);
    assert_eq!(
        u16::from_le_bytes([buffer[7], buffer[8]]),
        client.local_addr().unwrap().port()
    );
    client.send_to(&[20, 2, 0, 0], primary).await.unwrap();
    let (_, from) = client.recv_from(&mut buffer).await.unwrap();
    assert_eq!(from, primary);
    let alternate_port = u16::from_le_bytes([buffer[13], buffer[14]]);
    client.send_to(&[20, 2, 0, 2], primary).await.unwrap();
    let (_, from) = client.recv_from(&mut buffer).await.unwrap();
    assert_eq!(from.port(), alternate_port);
    assert_ne!(from.port(), primary.port());
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
}
