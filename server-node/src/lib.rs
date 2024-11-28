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

type CoordinatorId = Mutex<Option<u32>>;
type Leases = Mutex<Vec<Lease>>;
type Peers = Mutex<Vec<Peer>>;

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct Lease {
    hardware_address: [u8; 6],    // Assume MAC address
    lease_address: Ipv4Addr,      // Assume IPv4 for now
    expiry_timestamp: SystemTime, // SystemTime as exact time is not critical, and we want a timestamp
}

pub struct SharedState {
    #[allow(dead_code)]
    coordinator_id: CoordinatorId,
    leases: Leases,
    peers: Peers,
}

pub struct Server {
    #[allow(dead_code)]
    id: u32,
    config: Config,
    shared_state: Arc<SharedState>,
}

impl Server {
    /// Initialize cluster, and try connecting to peers.
    /// Failed handshakes are ignored, since they might be nodes that are starting later.
    /// After connecting to peers, you probably want to call [`start`].
    pub fn connect<S: MessageStream + 'static>(config: Config) -> Self {
        let coordinator_id = Mutex::new(None);
        let leases = Mutex::new(Vec::new());
        let peers = Mutex::new(Vec::new());

        let shared_state = Arc::new(SharedState {
            coordinator_id,
            leases,
            peers,
        });

        for peer_address in &config.peers {
            match S::connect(peer_address) {
                Ok(stream) => {
                    let result = Self::start_handshake(stream, &config, Arc::clone(&shared_state));
                    match result {
                        Ok(peer) => shared_state.peers.lock().unwrap().push(peer),
                        Err(e) => eprintln!("{e:?}"),
                    }
                }
                Err(e) => eprintln!("{e:?}"),
            }
        }

        Self {
            id: config.id,
            config,
            shared_state,
        }
    }

    fn start_handshake(
        mut stream: impl MessageStream + 'static,
        config: &Config,
        shared_state: Arc<SharedState>,
    ) -> Result<Peer, Box<dyn Error>> {
        let result = stream.send_message(&Message::Join(config.id));
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
                        shared_state.clone(),
                        config.heartbeat_timeout,
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
                let result = stream.send_message(&Message::JoinAck(self.config.id));
                match result {
                    Ok(_) => self.shared_state.peers.lock().unwrap().push(Peer::new(
                        stream,
                        id,
                        self.shared_state.clone(),
                        self.config.heartbeat_timeout,
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

    pub fn start(
        mut self,
        peer_listener: impl MessageListener + 'static,
    ) -> Result<(), Box<dyn Error>> {
        let peer_listener_thread = thread::spawn(move || self.listen_nodes(peer_listener));

        peer_listener_thread.join().map_err(|e| {
            format!(
                "Node listener thread panicked, Err: {:?}",
                e.downcast_ref::<&str>()
            )
            .into()
        })
    }
}
