pub mod config;
mod message;

use crate::config::Config;
use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    net::{SocketAddr, TcpListener, TcpStream},
};

#[derive(Serialize, Deserialize)]
pub struct Lease {}

struct Node {
    address: SocketAddr,
    stream: Option<TcpStream>,
}

struct Cluster {
    nodes: Vec<Node>,
    #[allow(dead_code)]
    coordinator_index: Option<usize>,
    #[allow(dead_code)]
    size: u32,
}

pub struct Server {
    cluster: Cluster,
}

impl Server {
    pub fn new(config: Config) -> Result<Self, Box<dyn Error>> {
        Ok(Self {
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
                size: u32::try_from(config.nodes.len()).expect("Can't have more than 2^32 nodes"),
            },
        })
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

    pub fn start(mut self, listener: TcpListener) -> Result<(), Box<dyn Error>> {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => self
                    .assign_stream_to_node(stream)
                    .unwrap_or_else(|e| eprintln!("{e:?}")),
                Err(error) => eprintln!("{error:?}"),
            }
        }

        Ok(())
    }
}
