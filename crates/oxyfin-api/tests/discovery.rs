//! Local network discovery against UDP responders on the loopback interface.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

mod common;

use common::fixture;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use oxyfin_api::{
    DISCOVERY_MESSAGE, DiscoveredServer, MAX_DISCOVERED_SERVERS, discover, discovered_version,
};
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

#[tokio::test]
async fn discover_stops_at_the_cap() {
    let replies = (0..MAX_DISCOVERED_SERVERS + 4)
        .map(|index| reply(&format!("id-{index}"), "Server", "http://10.0.0.1:8096"))
        .collect();
    let target = responder(replies).await;
    let start = Instant::now();

    let servers = discover(Duration::from_secs(2), &[target]).await;

    assert_eq!(servers.len(), MAX_DISCOVERED_SERVERS);
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "{:?}",
        start.elapsed()
    );
}

#[tokio::test]
async fn discover_ignores_replies_from_another_port() {
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let target = socket.local_addr().unwrap();
    let other = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    tokio::spawn(async move {
        let mut buffer = [0u8; 512];
        while let Ok((_, from)) = socket.recv_from(&mut buffer).await {
            let answer = reply("id-1", "Elsewhere", "http://10.0.0.1:8096");
            let _ = other.send_to(&answer, from).await;
        }
    });

    let servers = discover(Duration::from_millis(400), &[target]).await;

    assert!(servers.is_empty(), "{servers:?}");
}

fn found(address: &str) -> DiscoveredServer {
    DiscoveredServer {
        id: "id-1".to_owned(),
        name: "Maison".to_owned(),
        address: address.to_owned(),
    }
}

#[tokio::test]
async fn discovered_version_sends_no_identity() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/System/Info/Public"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(fixture("12.4.0", "system_info_public")),
        )
        .mount(&mock)
        .await;

    let version = discovered_version(&found(&mock.uri()), Duration::from_secs(1)).await;

    assert_eq!(version.as_deref(), Some("12.4.0"));
    let requests = mock.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    for (name, value) in requests[0].headers.iter() {
        let value = value.to_str().unwrap_or_default();
        assert!(
            !name.as_str().starts_with("x-emby") && !value.contains("MediaBrowser"),
            "{name}: {value}"
        );
    }
    assert!(!requests[0].headers.contains_key("authorization"));
}

#[tokio::test]
async fn discovered_version_follows_no_redirect() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/System/Info/Public"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/elsewhere"))
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/elsewhere"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(fixture("12.4.0", "system_info_public")),
        )
        .mount(&mock)
        .await;

    let version = discovered_version(&found(&mock.uri()), Duration::from_secs(1)).await;

    assert_eq!(version, None);
    assert_eq!(mock.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn discovered_version_gives_up_after_the_timeout() {
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/System/Info/Public"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(fixture("12.4.0", "system_info_public"))
                .set_delay(Duration::from_secs(3)),
        )
        .mount(&mock)
        .await;
    let start = Instant::now();

    let version = discovered_version(&found(&mock.uri()), Duration::from_millis(300)).await;

    assert_eq!(version, None);
    assert!(start.elapsed() < Duration::from_secs(2));
}
