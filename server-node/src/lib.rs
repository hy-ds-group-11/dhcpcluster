mod config;
mod message;

use crate::config::Config;
use serde::{Deserialize, Serialize};
use std::{error::Error, net::SocketAddr};

#[derive(Serialize, Deserialize)]
pub struct Lease {}

struct Node {
    address: SocketAddr,
}

struct Cluster {
    active_nodes: Vec<Node>,
    nodes: Vec<Node>,
    coordinator_index: Option<usize>,
    size: u32,
}

pub struct Server {
    cluster: Cluster,
}

impl Server {
    pub fn new(config: Config) -> Self {
        Self {
            cluster: Cluster {
                active_nodes: Vec::new(),
                nodes: config
                    .nodes
                    .iter()
                    .map(|a| Node { address: a.clone() })
                    .collect(),
                coordinator_index: None,
                size: u32::try_from(config.nodes.len()).expect("Can't have more than 2^32 nodes"),
            },
        }
    }

    pub fn start() -> Result<(), Box<dyn Error>> {
        Ok(())
    }
}
