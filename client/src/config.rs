use serde::Deserialize;
use std::{num::NonZero, thread, time::Duration};
use toml_config::TomlConfig;

#[derive(Deserialize)]
pub struct ConfigFile {
    servers: Vec<String>,
    default_port: u16,
    timeout: Option<u64>,
    thread_count: Option<usize>,
}

#[derive(Debug)]
pub struct Config {
    pub servers: Vec<String>,
    pub default_port: u16,
    pub timeout: Option<Duration>,
    pub thread_count: NonZero<usize>,
}

impl From<ConfigFile> for Config {
    fn from(
        ConfigFile {
            servers,
            default_port,
            timeout,
            thread_count,
        }: ConfigFile,
    ) -> Self {
        Self {
            servers,
            default_port,
            timeout: timeout.map(Duration::from_secs),
            // Use thread count in config file if defined as > 0,
            // otherwise use thread::available_parallelism(),
            // and if all else fails, use a default of 8
            thread_count: thread_count.and_then(NonZero::new).unwrap_or_else(|| {
                thread::available_parallelism().unwrap_or(NonZero::new(8).unwrap())
            }),
        }
    }
}

impl TomlConfig<ConfigFile> for Config {}
