use std::{
    error::Error,
    ffi::OsStr,
    net::{SocketAddr, ToSocketAddrs},
    path::Path,
};

use serde::Deserialize;

#[derive(Deserialize)]
pub struct Config {
    #[serde(skip_deserializing)]
    pub servers: Vec<SocketAddr>,
    #[serde(rename = "servers")]
    pub names: Vec<String>,
}

impl Config {
    pub fn load_toml_file(path: impl AsRef<Path>) -> Result<Self, Box<dyn Error>> {
        if path.as_ref().extension() != Some(OsStr::new("toml")) {
            return Err("Config file path should end in .toml".into());
        }
        let toml = std::fs::read_to_string(path)?;
        let mut conf: Config = toml::from_str(&toml)?;
        conf.servers = conf
            .names
            .iter()
            .map(|name| name.to_socket_addrs().unwrap().next().unwrap())
            .collect();
        Ok(conf)
    }
}
