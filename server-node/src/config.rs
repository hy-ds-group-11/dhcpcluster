use serde::Deserialize;
use std::{error::Error, net::SocketAddr, path::Path, str::FromStr};

#[derive(Deserialize, Debug)]
struct ConfigFile {
    address_private: SocketAddr,
    // TODO DHCP interface?
}

impl FromStr for ConfigFile {
    type Err = toml::de::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        toml::from_str(s)
    }
}

impl ConfigFile {
    pub fn from_toml_file(path: impl AsRef<Path>) -> Result<Self, Box<dyn Error>> {
        let toml = std::fs::read_to_string(path)?;
        let conf = Self::from_str(&toml)?;
        Ok(conf)
    }
}

#[derive(Debug)]
pub struct Config {
    pub address_private: SocketAddr,
}

impl Config {
    fn from_config_file(ConfigFile { address_private }: ConfigFile) -> Self {
        Self { address_private }
    }

    pub fn from_toml_file(path: impl AsRef<Path>) -> Result<Self, Box<dyn Error>> {
        Ok(Self::from_config_file(ConfigFile::from_toml_file(path)?))
    }
}
