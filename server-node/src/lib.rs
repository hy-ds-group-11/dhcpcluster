pub mod config;
mod message;

use crate::config::Config;
use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    net::{SocketAddr, TcpListener, TcpStream},
    thread,
};

#[derive(Serialize, Deserialize)]
pub struct Lease {/* TODO */}

struct Node {
    address: SocketAddr,
    stream: Option<TcpStream>,
}

struct Cluster {
    nodes: Vec<Node>,
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
                nodes: config
                    .nodes
                    .iter()
                    .map(|address| Node {
                        address: *address,
                        stream: None,
                    })
                    .collect(),
                coordinator_index: None,
            },
        }
    }

    fn assign_stream_to_node(&mut self, stream: TcpStream) -> Result<(), Box<dyn Error>> {
        let addr = stream.peer_addr()?;
        let node = self
            .cluster
            .nodes
            .iter_mut()
            .find(|node| node.address == addr);
        match node {
            Some(node) => node.stream = Some(stream),
            None => return Err(format!("No matching node with address {addr} found").into()),
        }
        Ok(())
    }

    fn listen_nodes(&mut self, listener: TcpListener) {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => self
                    .assign_stream_to_node(stream)
                    .unwrap_or_else(|e| eprintln!("{e:?}")),
                Err(e) => eprintln!("{e:?}"),
            }
        }
    }

    pub fn start(mut self, peer_listener: TcpListener) -> Result<(), Box<dyn Error>> {
        let peer_listener_thread = thread::spawn(move || self.listen_nodes(peer_listener));

        peer_listener_thread.join().map_err(|e| {
            format!(
                "Node listener thread paniced, Err: {:?}",
                e.downcast_ref::<&str>()
            )
            .into()
        })
    }
}
