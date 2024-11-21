pub mod config;
mod message;
mod peer;

use crate::config::Config;
use crate::peer::Peer;
use serde::{Deserialize, Serialize};
use std::{error::Error, net::TcpListener, thread};
use std::{net::Ipv4Addr, time::SystemTime};

#[derive(Serialize, Deserialize)]
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

pub struct Server {
    cluster: Cluster,
}

impl Server {
    pub fn new(config: Config) -> Self {
        Self {
            cluster: Cluster {
                peers: Vec::new(),
                coordinator_index: None,
            },
        }
    }

    fn listen_nodes(&mut self, listener: TcpListener) {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => self.cluster.peers.push(Peer::new(stream)),
                Err(e) => eprintln!("{e:?}"),
            }
        }
    }

    pub fn start(mut self, peer_listener: TcpListener) -> Result<(), Box<dyn Error>> {
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
