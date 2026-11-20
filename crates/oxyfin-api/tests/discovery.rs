//! Local network discovery against UDP responders on the loopback interface.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use oxyfin_api::{DISCOVERY_MESSAGE, DiscoveredServer, discover};
use tokio::net::UdpSocket;

/// Binds a responder that answers each request with `replies`, and returns its address.
async fn responder(replies: Vec<Vec<u8>>) -> SocketAddr {
    let socket = UdpSocket::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|err| panic!("bind: {err}"));
    let address = socket
        .local_addr()
        .unwrap_or_else(|err| panic!("local address: {err}"));
    tokio::spawn(async move {
        let mut buffer = [0u8; 512];
        loop {
            let Ok((len, from)) = socket.recv_from(&mut buffer).await else {
                return;
            };
            if &buffer[..len] != DISCOVERY_MESSAGE.as_bytes() {
                continue;
            }
            for reply in &replies {
                let _ = socket.send_to(reply, from).await;
            }
        }
    });
    address
}

fn reply(id: &str, name: &str, address: &str) -> Vec<u8> {
    serde_json::json!({
        "Address": address,
        "Id": id,
        "Name": name,
        "EndpointAddress": null,
    })
    .to_string()
    .into_bytes()
}

fn sorted(mut servers: Vec<DiscoveredServer>) -> Vec<DiscoveredServer> {
    servers.sort_by(|a, b| a.id.cmp(&b.id));
    servers
}

#[tokio::test]
async fn discover_parses_replies() {
    let first = responder(vec![reply("id-1", "Maison", "http://192.168.1.20:8096")]).await;
    let second = responder(vec![
        reply("id-1", "Maison again", "http://192.168.1.20:8096"),
        reply("id-2", "Salon", "http://192.168.1.42:8096"),
    ])
    .await;

    let servers = discover(Duration::from_millis(500), &[first, second]).await;

    let servers = sorted(servers);
    assert_eq!(servers.len(), 2, "{servers:?}");
    assert_eq!(servers[0].id, "id-1");
    assert!(servers[0].name.starts_with("Maison"));
    assert_eq!(servers[0].address, "http://192.168.1.20:8096");
    assert_eq!(
        servers[1],
        DiscoveredServer {
            id: "id-2".to_owned(),
            name: "Salon".to_owned(),
            address: "http://192.168.1.42:8096".to_owned(),
        }
    );
}

#[tokio::test]
async fn discover_ignores_malformed() {
    let target = responder(vec![
        b"not json".to_vec(),
        vec![0xff, 0xfe, 0x00],
        br#"{"Name":"No id","Address":"http://10.0.0.1:8096"}"#.to_vec(),
        br#"{"Id":"no-address","Name":"No address"}"#.to_vec(),
        reply("ftp", "Wrong scheme", "ftp://10.0.0.2"),
        reply("bad", "Not a URL", "nowhere"),
        br#"[1, 2, 3]"#.to_vec(),
        reply("good", "Maison", "https://jellyfin.local"),
    ])
    .await;

    let servers = discover(Duration::from_millis(500), &[target]).await;

    assert_eq!(
        servers,
        vec![DiscoveredServer {
            id: "good".to_owned(),
            name: "Maison".to_owned(),
            address: "https://jellyfin.local".to_owned(),
        }]
    );
}

#[tokio::test]
async fn discover_times_out_empty() {
    let silent = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let closed = {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        socket.local_addr().unwrap()
    };
    let timeout = Duration::from_millis(300);
    let start = Instant::now();

    let servers = discover(timeout, &[silent.local_addr().unwrap(), closed]).await;

    let elapsed = start.elapsed();
    assert!(servers.is_empty(), "{servers:?}");
    assert!(elapsed >= timeout, "returned after {elapsed:?}");
    assert!(elapsed < timeout * 3, "returned after {elapsed:?}");
}

#[tokio::test]
async fn discover_still_reads_replies_after_a_closed_target() {
    let closed = {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        socket.local_addr().unwrap()
    };
    let target = responder(vec![reply("id-1", "Maison", "http://127.0.0.1:8096")]).await;

    let servers = discover(Duration::from_millis(500), &[closed, target]).await;

    assert_eq!(servers.len(), 1, "{servers:?}");
}

#[tokio::test]
async fn discover_without_targets_returns_at_once() {
    let start = Instant::now();

    let servers = discover(Duration::from_secs(2), &[]).await;

    assert!(servers.is_empty());
    assert!(start.elapsed() < Duration::from_millis(500));
}
