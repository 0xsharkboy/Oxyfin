//! Finds Jellyfin servers on the local network with the UDP discovery protocol.

use std::collections::HashSet;
use std::future::Future;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;

use reqwest::header::ACCEPT;
use serde::Deserialize;
use tokio::net::UdpSocket;
use tokio::time::Instant;
use url::Url;

use crate::PublicSystemInfo;

/// UDP port Jellyfin servers listen on for discovery requests.
pub const DISCOVERY_PORT: u16 = 7359;

/// Request a Jellyfin server answers with its address, id and name.
pub const DISCOVERY_MESSAGE: &str = "who is JellyfinServer?";

/// Most servers one discovery keeps; it stops listening once it has them.
pub const MAX_DISCOVERED_SERVERS: usize = 16;

/// Receive errors in a row after which a discovery stops listening.
const MAX_RECEIVE_ERRORS: u32 = 8;

/// A server that answered a discovery request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredServer {
    /// Server id.
    pub id: String,
    /// Display name of the server.
    pub name: String,
    /// Base URL the server published, `http` or `https`.
    pub address: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Reply {
    address: String,
    id: String,
    #[serde(default)]
    name: Option<String>,
}

fn parse_reply(bytes: &[u8]) -> Option<DiscoveredServer> {
    let reply: Reply = serde_json::from_slice(bytes).ok()?;
    if reply.id.trim().is_empty() {
        return None;
    }
    let url = Url::parse(reply.address.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    let address = reply.address.trim().to_owned();
    Some(DiscoveredServer {
        name: reply
            .name
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| address.clone()),
        id: reply.id,
        address,
    })
}

/// Where a discovery request goes by default: the limited broadcast address and,
/// where the platform lists them, the broadcast address of each IPv4 interface.
pub fn default_targets() -> Vec<SocketAddr> {
    let mut targets = vec![SocketAddr::V4(SocketAddrV4::new(
        Ipv4Addr::BROADCAST,
        DISCOVERY_PORT,
    ))];
    for address in interface_broadcasts() {
        let target = SocketAddr::V4(SocketAddrV4::new(address, DISCOVERY_PORT));
        if !targets.contains(&target) {
            targets.push(target);
        }
    }
    targets
}

#[cfg(unix)]
fn interface_broadcasts() -> Vec<Ipv4Addr> {
    use nix::net::if_::InterfaceFlags;
    let Ok(interfaces) = nix::ifaddrs::getifaddrs() else {
        return Vec::new();
    };
    interfaces
        .filter(|interface| {
            interface.flags.contains(InterfaceFlags::IFF_UP)
                && interface.flags.contains(InterfaceFlags::IFF_BROADCAST)
                && !interface.flags.contains(InterfaceFlags::IFF_LOOPBACK)
        })
        .filter_map(|interface| {
            interface
                .broadcast
                .as_ref()
                .and_then(|address| address.as_sockaddr_in().map(|address| address.ip()))
        })
        .filter(|address| !address.is_unspecified())
        .collect()
}

#[cfg(not(unix))]
fn interface_broadcasts() -> Vec<Ipv4Addr> {
    Vec::new()
}

/// Where discovery answers come from.
trait Inbox {
    fn receive(&self, buffer: &mut [u8]) -> impl Future<Output = io::Result<(usize, SocketAddr)>>;
}

impl Inbox for UdpSocket {
    fn receive(&self, buffer: &mut [u8]) -> impl Future<Output = io::Result<(usize, SocketAddr)>> {
        self.recv_from(buffer)
    }
}

/// Sends a discovery request to each of `targets`, then collects the answers for `timeout`.
///
/// Only answers coming from the port of a target count. Servers are deduplicated by id, the
/// first answer winning, and at most [`MAX_DISCOVERED_SERVERS`] are kept. Malformed answers are
/// ignored, and any network failure, such as broadcasts being refused, yields fewer or no
/// servers rather than an error.
pub async fn discover(timeout: Duration, targets: &[SocketAddr]) -> Vec<DiscoveredServer> {
    if targets.is_empty() {
        return Vec::new();
    }
    let Ok(socket) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await else {
        return Vec::new();
    };
    let _ = socket.set_broadcast(true);
    let mut sent = false;
    for target in targets {
        sent |= socket
            .send_to(DISCOVERY_MESSAGE.as_bytes(), target)
            .await
            .is_ok();
    }
    if !sent {
        return Vec::new();
    }
    let ports: HashSet<u16> = targets.iter().map(SocketAddr::port).collect();
    collect(&socket, Instant::now() + timeout, &ports).await
}

async fn collect(
    inbox: &impl Inbox,
    deadline: Instant,
    ports: &HashSet<u16>,
) -> Vec<DiscoveredServer> {
    let mut seen = HashSet::new();
    let mut servers = Vec::new();
    let mut buffer = vec![0u8; 2048];
    let mut errors = 0;
    while servers.len() < MAX_DISCOVERED_SERVERS {
        let received = tokio::time::timeout_at(deadline, inbox.receive(&mut buffer)).await;
        let (len, from) = match received {
            Err(_) => break,
            // An unconnected socket can report an earlier send to a closed port.
            Ok(Err(_)) => {
                errors += 1;
                if errors >= MAX_RECEIVE_ERRORS {
                    break;
                }
                continue;
            }
            Ok(Ok(received)) => received,
        };
        errors = 0;
        if !ports.contains(&from.port()) {
            continue;
        }
        if let Some(server) = parse_reply(&buffer[..len])
            && seen.insert(server.id.clone())
        {
            servers.push(server);
        }
    }
    servers
}

/// Version a discovered server publishes, or `None` when it does not tell within `timeout`.
///
/// The request carries no client identity and follows no redirect, since the address comes
/// from an unauthenticated answer on the network.
pub async fn discovered_version(server: &DiscoveredServer, timeout: Duration) -> Option<String> {
    let mut url = Url::parse(&server.address).ok()?;
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    let url = url.join("System/Info/Public").ok()?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .build()
        .ok()?;
    let response = client
        .get(url)
        .header(ACCEPT, "application/json")
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let bytes = response.bytes().await.ok()?;
    let info: PublicSystemInfo = crate::json::decode("System/Info/Public", &bytes).ok()?;
    Some(info.version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn reply_without_name_uses_the_address() {
        let server = parse_reply(br#"{"Address":"http://10.0.0.1:8096","Id":"a"}"#).unwrap();
        assert_eq!(server.name, "http://10.0.0.1:8096");
    }

    #[test]
    fn reply_with_endpoint_address_is_accepted() {
        let reply = br#"{"Address":"http://10.0.0.1:8096","Id":"a","Name":"N","EndpointAddress":"10.0.0.1"}"#;
        assert_eq!(parse_reply(reply).unwrap().name, "N");
    }

    #[test]
    fn reply_with_blank_id_is_ignored() {
        assert!(parse_reply(br#"{"Address":"http://10.0.0.1:8096","Id":" "}"#).is_none());
    }

    #[test]
    fn reply_with_a_user_in_the_address_is_ignored() {
        for address in [
            "http://192.168.1.20:8096@evil.example",
            "http://user:secret@192.168.1.20:8096",
            "http://:secret@192.168.1.20:8096",
        ] {
            let reply = format!(r#"{{"Address":"{address}","Id":"a"}}"#);
            assert!(parse_reply(reply.as_bytes()).is_none(), "{address}");
        }
    }

    #[test]
    fn default_targets_start_with_the_limited_broadcast() {
        let targets = default_targets();
        assert_eq!(
            targets[0],
            SocketAddr::from((Ipv4Addr::BROADCAST, DISCOVERY_PORT))
        );
        assert!(targets.iter().all(|target| target.port() == DISCOVERY_PORT));
    }

    struct Failing {
        calls: Cell<u32>,
    }

    impl Inbox for Failing {
        fn receive(&self, _: &mut [u8]) -> impl Future<Output = io::Result<(usize, SocketAddr)>> {
            self.calls.set(self.calls.get() + 1);
            std::future::ready(Err(io::Error::from(io::ErrorKind::ConnectionRefused)))
        }
    }

    #[tokio::test]
    async fn persistent_receive_errors_stop_the_discovery() {
        let inbox = Failing {
            calls: Cell::new(0),
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        let start = std::time::Instant::now();

        let servers = collect(&inbox, deadline, &HashSet::from([DISCOVERY_PORT])).await;

        assert!(servers.is_empty());
        assert_eq!(inbox.calls.get(), MAX_RECEIVE_ERRORS);
        assert!(start.elapsed() < Duration::from_secs(1));
    }
}
