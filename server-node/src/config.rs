//! # Server Configuration and TOML File Loading
//!
//! This module contains the server configuration structure, along with facilities for
//! loading configuration files from the filesystem.
//!
//! Jump to [`Config::load_toml_file`] for configuration file loading.

use crate::{dhcp::Ipv4Range, server::peer};
use serde::Deserialize;
use std::{
    net::{Ipv4Addr, SocketAddr},
    num::NonZero,
    thread,
    time::Duration,
};
use toml_config::TomlConfig;

#[derive(Deserialize, Debug)]
pub struct DhcpSection {
    net: Ipv4Addr,
    prefix_length: u32,
    lease_time_seconds: u64,
}

#[derive(Deserialize, Debug)]
pub struct ServerSection {
    listen_cluster: SocketAddr,
    listen_dhcp: SocketAddr,
    client_timeout_seconds: Option<u64>,
    thread_count: Option<usize>,
}

#[derive(Deserialize, Debug)]
pub struct ClusterSection {
    id: peer::Id,
    heartbeat_timeout_millis: u64,
    connect_timeout_seconds: Option<u64>,
}

#[derive(Deserialize, Debug)]
pub struct Peer {
    pub id: peer::Id,
    pub host: String,
}

#[derive(Deserialize, Debug)]
pub struct File {
    dhcp: DhcpSection,
    server: ServerSection,
    cluster: ClusterSection,
    peers: Vec<Peer>,
}

/// Server configuration
///
/// Use [`Config::load_toml_file`] to initialize.
#[derive(Debug)]
pub struct Config {
    pub dhcp_pool: Ipv4Range,
    pub prefix_length: u32,
    pub lease_time: Duration,

    pub listen_cluster: SocketAddr,
    pub listen_dhcp: SocketAddr,
    pub client_timeout: Duration,
    pub thread_count: NonZero<usize>,

    pub id: peer::Id,
    pub heartbeat_timeout: Duration,
    pub connect_timeout: Option<Duration>,
    pub peers: Vec<Peer>,
}

impl From<File> for Config {
    // This implementation is a no-op for now, but down the line it's possible
    // that our server configuration struct diverges from the
    // configuration file contents
    fn from(file: File) -> Self {
        let dhcp = file.dhcp;
        let server = file.server;
        let cluster = file.cluster;

        Self {
            // DHCP
            dhcp_pool: Ipv4Range::from_cidr(dhcp.net, dhcp.prefix_length),
            prefix_length: dhcp.prefix_length,
            lease_time: Duration::from_secs(dhcp.lease_time_seconds),

            // Server
            listen_cluster: server.listen_cluster,
            listen_dhcp: server.listen_dhcp,
            client_timeout: server
                .client_timeout_seconds
                .and_then(|sec| (sec != 0).then_some(Duration::from_secs(sec)))
                .unwrap_or(Duration::from_secs(10)),
            thread_count: server
                .thread_count
                .and_then(NonZero::new)
                .unwrap_or_else(|| {
                    #[allow(clippy::unwrap_used, reason = "NonZero constructed from literal")]
                    thread::available_parallelism()
                        .map(|n| NonZero::new(usize::from(n) * 4).unwrap())
                        .unwrap_or(NonZero::new(8).unwrap())
                }),

            // Cluster
            id: cluster.id,
            heartbeat_timeout: Duration::from_millis(cluster.heartbeat_timeout_millis),
            connect_timeout: cluster
                .connect_timeout_seconds
                .map_or(Some(Duration::from_secs(10)), |sec| {
                    (sec != 0).then_some(Duration::from_secs(sec))
                }),
            peers: file.peers,
        }
    }
}

impl TomlConfig<File> for Config {}
