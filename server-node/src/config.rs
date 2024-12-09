//! # Server Configuration and TOML File Loading
//!
//! This module contains the server configuration structure, along with facilities for
//! loading configuration files from the filesystem.
//!
//! Jump to [`Config::load_toml_file`] for configuration file loading.

use serde::Deserialize;
use std::{
    error::Error,
    ffi::OsStr,
    net::{Ipv4Addr, SocketAddr},
    path::Path,
    str::FromStr,
    time::Duration,
};

use crate::{dhcp::DhcpService, peer::PeerId};

#[derive(Deserialize, Debug)]
struct ConfigFile {
    address_private: SocketAddr,
    peers: Vec<SocketAddr>,
    id: PeerId,
    heartbeat_timeout: u64,
    net: Ipv4Addr,
    prefix_length: u32,
    lease_time: u64,
    dhcp_address: SocketAddr,
    thread_count: usize,
}

impl FromStr for ConfigFile {
    type Err = toml::de::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        toml::from_str(s)
    }
}

/// Server configuration
///
/// Use [`Config::load_toml_file`] to initialize.
#[derive(Debug, Clone)]
pub struct Config {
    pub address_private: SocketAddr,
    pub peers: Vec<SocketAddr>,
    pub id: PeerId,
    pub heartbeat_timeout: Duration,
    pub prefix_length: u32,
    pub dhcp_pool: DhcpService,
    pub dhcp_address: SocketAddr,
    pub thread_count: usize,
}

impl From<ConfigFile> for Config {
    // This implementation is a no-op for now, but down the line it's possible
    // that our server configuration struct diverges from the
    // configuration file contents
    fn from(
        ConfigFile {
            address_private,
            peers,
            id,
            heartbeat_timeout,
            net,
            prefix_length,
            lease_time,
            dhcp_address,
            thread_count,
        }: ConfigFile,
    ) -> Self {
        Self {
            address_private,
            peers,
            id,
            heartbeat_timeout: Duration::from_millis(heartbeat_timeout),
            prefix_length,
            dhcp_pool: DhcpService::from_cidr(net, prefix_length, Duration::from_secs(lease_time)),
            dhcp_address,
            thread_count,
        }
    }
}

impl Config {
    /// Loads a .toml file from the filesystem, parses it, and initializes a [`Config`].
    ///
    /// ## Error cases:
    /// - File extension is not .toml
    /// - File access is unsuccessful
    /// - TOML parsing failure
    pub fn load_toml_file(path: impl AsRef<Path>) -> Result<Self, Box<dyn Error>> {
        if path.as_ref().extension() != Some(OsStr::new("toml")) {
            return Err("Config file path should end in .toml".into());
        }
        let toml = std::fs::read_to_string(path)?;
        let conf = ConfigFile::from_str(&toml)?;
        Ok(conf.into())
    }
}
