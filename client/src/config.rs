use std::{error::Error, ffi::OsStr, path::Path};

use serde::Deserialize;

#[derive(Deserialize)]
pub struct Config {
    pub servers: Vec<String>,
}

impl Config {
    pub fn load_toml_file(path: impl AsRef<Path>) -> Result<Self, Box<dyn Error>> {
        if path.as_ref().extension() != Some(OsStr::new("toml")) {
            return Err("Config file path should end in .toml".into());
        }
        let toml = std::fs::read_to_string(path)?;
        let conf: Config = toml::from_str(&toml)?;
        Ok(conf)
    }
}
