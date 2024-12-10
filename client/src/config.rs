use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    time::Duration,
};
use thiserror::Error;

use serde::Deserialize;

#[derive(Deserialize)]
struct ConfigFile {
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
