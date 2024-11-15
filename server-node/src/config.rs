use serde::Deserialize;
use std::{error::Error, net::SocketAddr, path::Path, str::FromStr};

#[derive(Deserialize, Debug)]
struct ConfigFile {
    address_private: SocketAddr,
    nodes: Vec<SocketAddr>,
    id: u32,
    // TODO DHCP interface?
}

impl FromStr for ConfigFile {
    type Err = toml::de::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        toml::from_str(s)
    }
}

#[derive(Debug)]
pub struct Config {
    pub address_private: SocketAddr,
    pub nodes: Vec<SocketAddr>,
    pub id: u32,
}

impl From<ConfigFile> for Config {
    // This implementation is a no-op for now, but down the line it's possible
    // that our server configuration struct diverges from the
    // configuration file contents
    fn from(
        ConfigFile {
            address_private,
            nodes,
            id,
        }: ConfigFile,
    ) -> Self {
        Self {
            address_private,
            nodes,
            id,
        }
    }
}

impl Config {
    pub fn load_toml_file(path: impl AsRef<Path>) -> Result<Self, Box<dyn Error>> {
        let toml = std::fs::read_to_string(path)?;
        let conf = ConfigFile::from_str(&toml)?;
        Ok(conf.into())
    }
}
