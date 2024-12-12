//! # Server Configuration and TOML File Loading
//!
//! This module contains the server configuration structure, along with facilities for
//! loading configuration files from the filesystem.
//!
//! Jump to [`Config::load_toml_file`] for configuration file loading.

use serde::Deserialize;
use std::{
    ffi::OsStr,
    net::{Ipv4Addr, SocketAddr},
    num::NonZero,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};
use thiserror::Error;

use crate::{dhcp::DhcpService, peer::PeerId};

// TODO: Make the field names consistent (e.g. address- prefix/postfix)
#[derive(Deserialize, Debug)]
struct ConfigFile {
    address_private: SocketAddr,
    peers: Vec<String>,
    id: PeerId,
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
    pub peers: Vec<String>,
    pub id: PeerId,
    pub heartbeat_timeout: Duration,
    pub peer_connection_timeout: Option<Duration>,
    pub prefix_length: u32,
    pub dhcp_pool: DhcpService,
    pub dhcp_address: SocketAddr,
    pub thread_count: NonZero<usize>,
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
            peer_connection_timeout,
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
            // Use thread count in config file if defined as > 0,
            // otherwise use thread::available_parallelism(),
            // and if all else fails, use a default of 8
            thread_count: thread_count.and_then(NonZero::new).unwrap_or_else(|| {
                thread::available_parallelism().unwrap_or(NonZero::new(8).unwrap())
            }),
            // If None in ConfigFile, default to 10 seconds
            // If defined as Some(0), set None to Config
            peer_connection_timeout: peer_connection_timeout
                .map(|sec| (sec != 0).then_some(Duration::from_secs(sec)))
                .unwrap_or(Some(Duration::from_secs(10))),
        }
    }
}

#[derive(Error, Debug)]
pub enum Error {
    #[error("Config file path should end in .toml")]
    FileExtension,
    #[error("Failed to read config file {path}")]
    ReadFile {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("Failed to parse config file {path}")]
    ParseFile {
        path: PathBuf,
        source: toml::de::Error,
    },
}

impl Config {
    /// Loads a .toml file from the filesystem, parses it, and initializes a [`Config`].
    pub fn load_toml_file(path: impl AsRef<Path>) -> Result<Self, Error> {
        if path.as_ref().extension() != Some(OsStr::new("toml")) {
            return Err(Error::FileExtension);
        }
        let path = path.as_ref();
        let toml = std::fs::read_to_string(path).map_err(|e| Error::ReadFile {
            path: path.to_path_buf(),
            source: e,
        })?;
        let conf: ConfigFile = toml::from_str(&toml).map_err(|e| Error::ParseFile {
            path: path.to_path_buf(),
            source: e,
        })?;
        Ok(conf.into())
    }
}
