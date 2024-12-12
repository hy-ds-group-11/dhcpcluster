use serde::Deserialize;
use std::time::Duration;
use toml_config::TomlConfig;

#[derive(Deserialize)]
pub struct ConfigFile {
    servers: Vec<String>,
    default_port: u16,
    timeout: Option<u64>,
}

#[derive(Debug)]
pub struct Config {
    pub servers: Vec<String>,
    pub default_port: u16,
    pub timeout: Option<Duration>,
}

impl From<ConfigFile> for Config {
    fn from(
        ConfigFile {
            servers,
            default_port,
            timeout,
        }: ConfigFile,
    ) -> Self {
        Self {
            servers,
            default_port,
            timeout: timeout.map(Duration::from_secs),
        }
    }
}

impl TomlConfig<ConfigFile> for Config {}
