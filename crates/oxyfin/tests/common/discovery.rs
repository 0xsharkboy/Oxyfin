//! A discovery responder on the loopback interface, standing in for servers on the network.

use std::net::{SocketAddr, UdpSocket};

/// Binds a responder answering each discovery request with `reply`, or never with `None`.
pub fn responder(reply: Option<Vec<u8>>) -> SocketAddr {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap_or_else(|err| panic!("bind: {err}"));
    let address = socket
        .local_addr()
        .unwrap_or_else(|err| panic!("local address: {err}"));
    std::thread::spawn(move || {
        let mut buffer = [0u8; 256];
        while let Ok((_, from)) = socket.recv_from(&mut buffer) {
            if let Some(reply) = &reply {
                let _ = socket.send_to(reply, from);
            }
        }
    });
    address
}

/// The answer of a server named "Maison" at `address`.
pub fn reply(address: &str) -> Vec<u8> {
    format!(r#"{{"Address":"{address}","Id":"maison-id","Name":"Maison","EndpointAddress":null}}"#)
        .into_bytes()
}

/// The answer of a server with this `id` and `name` at `address`.
pub fn reply_as(id: &str, name: &str, address: &str) -> Vec<u8> {
    format!(r#"{{"Address":"{address}","Id":"{id}","Name":"{name}"}}"#).into_bytes()
}
