//! # Server Configuration and TOML File Loading
//!
//! This module contains the server configuration structure, along with facilities for
//! loading configuration files from the filesystem.
//!
//! Jump to [`Config::load_toml_file`] for configuration file loading.

use serde::Deserialize;
use std::{error::Error, ffi::OsStr, net::SocketAddr, path::Path, str::FromStr, time::Duration};

#[derive(Deserialize, Debug)]
struct ConfigFile {
    address_private: SocketAddr,
    peers: Vec<SocketAddr>,
    id: u32,
    heartbeat_timeout: u64,
    // TODO DHCP interface?
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
    pub id: u32,
    pub heartbeat_timeout: Duration,
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
        }: ConfigFile,
    ) -> Self {
        Self {
            address_private,
            peers,
            id,
            heartbeat_timeout: Duration::from_millis(heartbeat_timeout),
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
