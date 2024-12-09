use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};
use thiserror::Error;

use serde::Deserialize;

#[derive(Deserialize)]
pub struct Config {
    pub servers: Vec<String>,
    pub default_port: u16,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            servers: Vec::new(),
            default_port: 4321,
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
        let conf: Config = toml::from_str(&toml).map_err(|e| Error::ParseFile {
            path: path.to_path_buf(),
            source: e,
        })?;
        Ok(conf)
    }
}
