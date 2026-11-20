//! Finds Jellyfin servers on the local network with the UDP discovery protocol.

use std::collections::HashSet;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;

use serde::Deserialize;
use tokio::net::UdpSocket;
use tokio::time::Instant;
use url::Url;

/// UDP port Jellyfin servers listen on for discovery requests.
pub const DISCOVERY_PORT: u16 = 7359;

/// Request a Jellyfin server answers with its address, id and name.
pub const DISCOVERY_MESSAGE: &str = "who is JellyfinServer?";

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
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
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

/// Sends a discovery request to each of `targets`, then collects the answers for `timeout`.
///
/// Servers are deduplicated by id, the first answer winning. Malformed answers are ignored,
/// and any network failure, such as broadcasts being refused, yields fewer or no servers
/// rather than an error.
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

    let deadline = Instant::now() + timeout;
    let mut seen = HashSet::new();
    let mut servers = Vec::new();
    let mut buffer = vec![0u8; 2048];
    loop {
        let received = tokio::time::timeout_at(deadline, socket.recv_from(&mut buffer)).await;
        let len = match received {
            Err(_) => break,
            // An unconnected socket can report an earlier send to a closed port.
            Ok(Err(_)) => continue,
            Ok(Ok((len, _))) => len,
        };
        if let Some(server) = parse_reply(&buffer[..len])
            && seen.insert(server.id.clone())
        {
            servers.push(server);
        }
    }
    servers
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn default_targets_start_with_the_limited_broadcast() {
        let targets = default_targets();
        assert_eq!(
            targets[0],
            SocketAddr::from((Ipv4Addr::BROADCAST, DISCOVERY_PORT))
        );
        assert!(targets.iter().all(|target| target.port() == DISCOVERY_PORT));
    }
}
