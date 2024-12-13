//! # Server Configuration and TOML File Loading
//!
//! This module contains the server configuration structure, along with facilities for
//! loading configuration files from the filesystem.
//!
//! Jump to [`Config::load_toml_file`] for configuration file loading.

use crate::{dhcp, peer};
use serde::Deserialize;
use std::{
    net::{Ipv4Addr, SocketAddr},
    num::NonZero,
    thread,
    time::Duration,
};
use toml_config::TomlConfig;

// TODO: Make the field names consistent (e.g. address- prefix/postfix)
#[derive(Deserialize, Debug)]
pub struct File {
    address_private: SocketAddr,
    peers: Vec<(peer::Id, String)>,
    id: peer::Id,
    heartbeat_timeout: u64,
    peer_connection_timeout: Option<u64>,
    net: Ipv4Addr,
    prefix_length: u32,
    lease_time: u64,
    dhcp_address: SocketAddr,
    thread_count: Option<usize>,
}

/// Server configuration
///
/// Use [`Config::load_toml_file`] to initialize.
#[derive(Debug, Clone)]
pub struct Config {
    pub address_private: SocketAddr,
    pub peers: Vec<(peer::Id, String)>,
    pub id: peer::Id,
    pub heartbeat_timeout: Duration,
    pub peer_connection_timeout: Option<Duration>,
    pub prefix_length: u32,
    pub dhcp_pool: dhcp::Pool,
    pub dhcp_address: SocketAddr,
    pub thread_count: NonZero<usize>,
}

impl From<File> for Config {
    // This implementation is a no-op for now, but down the line it's possible
    // that our server configuration struct diverges from the
    // configuration file contents
    fn from(
        File {
            address_private,
            peers,
            id,
            heartbeat_timeout,
            net,
            prefix_length,
            lease_time,
            dhcp_address,
            thread_count,
            peer_connection_timeout,
        }: File,
    ) -> Self {
        Self {
            address_private,
            peers,
            id,
            heartbeat_timeout: Duration::from_millis(heartbeat_timeout),
            prefix_length,
            dhcp_pool: dhcp::Pool::from_cidr(net, prefix_length, Duration::from_secs(lease_time)),
            dhcp_address,
            // Use thread count in config file if defined as > 0,
            // otherwise use thread::available_parallelism(),
            // and if all else fails, use a default of 8
            thread_count: thread_count.and_then(NonZero::new).unwrap_or_else(|| {
                #[allow(clippy::unwrap_used, reason = "Default thread count from literal")]
                thread::available_parallelism().unwrap_or(NonZero::new(8).unwrap())
            }),
            // If None in ConfigFile, default to 10 seconds
            // If defined as Some(0), set None to Config
            peer_connection_timeout: peer_connection_timeout
                .map_or(Some(Duration::from_secs(10)), |sec| {
                    (sec != 0).then_some(Duration::from_secs(sec))
                }),
        }
    }
}

impl TomlConfig<File> for Config {}
