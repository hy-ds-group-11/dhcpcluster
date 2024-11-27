#![cfg_attr(coverage_nightly, feature(coverage_attribute))]

pub mod config;
mod message;
mod peer;

use crate::{config::Config, peer::Peer};
use message::{Message, MessageListener, MessageStream};
use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    net::Ipv4Addr,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, SystemTime},
};

type Leases = Arc<Mutex<Vec<Lease>>>;

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct Lease {
    hardware_address: [u8; 6],    // Assume MAC address
    lease_address: Ipv4Addr,      // Assume IPv4 for now
    expiry_timestamp: SystemTime, // SystemTime as exact time is not critical, and we want a timestamp
}

pub struct Cluster {
    server: Server,
    peers: Vec<Peer>,
    #[allow(dead_code)]
    coordinator_id: Option<u32>,
}

impl Cluster {
    /// Initialize cluster, and try connecting to peers.
    /// Failed handshakes are ignored, since they might be nodes that are starting later.
    /// After connecting to peers, `run_server(stream)` needs to be called.
    pub fn connect<S: MessageStream + 'static>(config: Config) -> Self {
        let mut peers = Vec::new();
        let server = Server::new(&config);

        for peer_address in config.peers {
            match S::connect(peer_address) {
                Ok(stream) => {
                    let result = Self::start_handshake(stream, config.id, &server);
                    match result {
                        Ok(peer) => peers.push(peer),
                        Err(e) => eprintln!("{e:?}"),
                    }
                }
                Err(e) => eprintln!("{e:?}"),
            }
        }

        Self {
            server,
            peers,
            coordinator_id: None,
        }
    }

    fn start_handshake(
        mut stream: impl MessageStream + 'static,
        id: u32,
        server: &Server,
    ) -> Result<Peer, Box<dyn Error>> {
        let result = stream.send_message(&Message::Join(id));
        match result {
            Ok(_) => {
                stream
                    .set_read_timeout(Some(Duration::from_millis(500)))
                    .unwrap();
                let message = stream.receive_message().unwrap();
                stream.set_read_timeout(None).unwrap();

                match dbg!(message) {
                    Message::JoinAck(peer_id) => Ok(Peer::new(
                        stream,
                        peer_id,
                        Arc::clone(&server.leases),
                        server.config.heartbeat_timeout,
                    )),
                    _ => panic!("Peer responded to Join with something other than JoinAck"),
                }
            }
            Err(e) => {
                eprintln!("{e:?}");
                Err(e.into())
            }
        }
    }

    fn answer_handshake(&mut self, mut stream: impl MessageStream + 'static) {
        stream
            .set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        let message = stream.receive_message().unwrap();
        stream.set_read_timeout(None).unwrap();

        match message {
            Message::Join(id) => {
                let result = stream.send_message(&Message::JoinAck(self.server.id));
                match result {
                    Ok(_) => self.peers.push(Peer::new(
                        stream,
                        id,
                        Arc::clone(&self.server.leases),
                        self.server.config.heartbeat_timeout,
                    )),
                    Err(e) => eprintln!("{e:?}"),
                }
            }
            _ => panic!("First message of peer wasn't Join"),
        }
    }

    fn listen_nodes(&mut self, listener: impl MessageListener + 'static) {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => self.answer_handshake(stream),
                Err(e) => eprintln!("{e:?}"),
            }
        }
    }

    pub fn start_server(
        mut self,
        peer_listener: impl MessageListener + 'static,
    ) -> Result<(), Box<dyn Error>> {
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

pub struct Server {
    id: u32,
    leases: Leases,
    config: Config,
}

impl Server {
    pub fn new(config: &Config) -> Self {
        Self {
            id: config.id,
            leases: Arc::new(Mutex::new(Vec::new())),
            config: config.clone(),
        }
    }
}
