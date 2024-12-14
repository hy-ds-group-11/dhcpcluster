#![deny(clippy::unwrap_used, clippy::allow_attributes_without_reason)]
#![warn(clippy::perf, clippy::complexity, clippy::pedantic, clippy::suspicious)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    reason = "We're not going to write comprehensive docs"
)]
#![allow(
    clippy::cast_precision_loss,
    reason = "There are no sufficient floating point types"
)]

use serde::de::DeserializeOwned;
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Refusing to load {0}, file extension isn't .toml")]
    FileExtension(PathBuf),
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

pub trait TomlConfig<F: DeserializeOwned>: Sized + From<F> {
    /// Loads a .toml file from the filesystem, parses it, and initializes a [`Self`].
    fn load_toml_file(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        if path.extension() != Some(OsStr::new("toml")) {
            return Err(Error::FileExtension(path.to_path_buf()));
        }
        let toml = std::fs::read_to_string(path).map_err(|e| Error::ReadFile {
            path: path.to_path_buf(),
            source: e,
        })?;
        let conf: F = toml::from_str(&toml).map_err(|e| Error::ParseFile {
            path: path.to_path_buf(),
            source: e,
        })?;
        Ok(conf.into())
    }
}
