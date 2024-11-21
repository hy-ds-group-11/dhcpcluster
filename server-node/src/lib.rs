pub mod config;
mod message;
mod peer;

use crate::config::Config;
use crate::peer::Peer;
use message::Message;
use serde::{Deserialize, Serialize};
use std::net::TcpStream;
use std::time::Duration;
use std::{error::Error, net::TcpListener, thread};
use std::{net::Ipv4Addr, time::SystemTime};

#[derive(Serialize, Deserialize, Debug)]
pub struct Lease {
    hardware_address: [u8; 6],    // Assume MAC address
    lease_address: Ipv4Addr,      // Assume IPv4 for now
    expiry_timestamp: SystemTime, // SystemTime as exact time is not critical, and we want a timestamp
}

struct Cluster {
    peers: Vec<Peer>,
    #[allow(dead_code)]
    coordinator_index: Option<usize>,
}

impl Cluster {
    pub fn connect(config: Config) -> Self {
        let mut peers = Vec::new();

        for peer_address in config.peers {
            match TcpStream::connect(peer_address) {
                Ok(stream) => {
                    let result = Self::start_handshake(stream, config.id);
                    match result {
                        Ok(peer) => peers.push(peer),
                        Err(e) => eprintln!("{e:?}"),
                    }
                }
                Err(e) => eprintln!("{e:?}"),
            }
        }

        Self {
            peers,
            coordinator_index: None,
        }
    }

    fn start_handshake(stream: TcpStream, id: u32) -> Result<Peer, Box<dyn Error>> {
        let result = ciborium::into_writer::<Message, &TcpStream>(&Message::Join(id), &stream);
        match result {
            Ok(_) => {
                stream
                    .set_read_timeout(Some(Duration::from_millis(500)))
                    .unwrap();
                let message = ciborium::from_reader::<Message, &TcpStream>(&stream).unwrap();
                stream.set_read_timeout(None).unwrap();

                match message {
                    Message::JoinAck(peer_id) => Ok(Peer::new(stream, peer_id)),
                    _ => panic!("Peer responded to Join with something other than JoinAck"),
                }
            }
            Err(e) => {
                eprintln!("{e:?}");
                Err(e.into())
            }
        }
    }
}

pub struct Server {
    id: u32,
    cluster: Cluster,
}

impl Server {
    pub fn new(config: Config) -> Self {
        Self {
            id: config.id,
            cluster: Cluster::connect(config),
        }
    }

    fn answer_handshake(&mut self, stream: TcpStream) {
        stream
            .set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        let message = ciborium::from_reader::<Message, &TcpStream>(&stream).unwrap();
        stream.set_read_timeout(None).unwrap();

        match message {
            Message::Join(id) => {
                let result = ciborium::into_writer::<Message, &TcpStream>(
                    &Message::JoinAck(self.id),
                    &stream,
                );
                match result {
                    Ok(_) => self.cluster.peers.push(Peer::new(stream, id)),
                    Err(e) => eprintln!("{e:?}"),
                }
            }
            _ => panic!("First message of peer wasn't Join"),
        }
    }

    fn listen_nodes(&mut self, listener: TcpListener) {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => self.answer_handshake(stream),
                Err(e) => eprintln!("{e:?}"),
            }
        }
    }

    pub fn start(mut self, peer_listener: TcpListener) -> Result<(), Box<dyn Error>> {
        let peer_listener_thread = thread::spawn(move || self.listen_nodes(peer_listener));

        // TODO

        peer_listener_thread.join().map_err(|e| {
            format!(
                "Node listener thread panicked, Err: {:?}",
                e.downcast_ref::<&str>()
            )
            .into()
        })
    }
}
