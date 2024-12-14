use serde::Deserialize;
use std::{num::NonZero, thread, time::Duration};
use toml_config::TomlConfig;

#[derive(Deserialize)]
pub struct File {
    servers: Vec<String>,
    default_port: u16,
    timeout_seconds: Option<u64>,
    thread_count: Option<usize>,
}

#[derive(Debug)]
pub struct Config {
    pub servers: Vec<String>,
    pub default_port: u16,
    pub timeout: Option<Duration>,
    pub thread_count: NonZero<usize>,
}

impl From<File> for Config {
    fn from(
        File {
            servers,
            default_port,
            timeout_seconds,
            thread_count,
        }: File,
    ) -> Self {
        Self {
            servers,
            default_port,
            timeout: timeout_seconds.map(Duration::from_secs),
            // Use thread count in config file if defined as > 0,
            // otherwise use thread::available_parallelism(),
            // and if all else fails, use a default of 8
            thread_count: thread_count.and_then(NonZero::new).unwrap_or_else(|| {
                #[allow(clippy::unwrap_used, reason = "Default thread count from literal")]
                thread::available_parallelism().unwrap_or(NonZero::new(8).unwrap())
            }),
        }
    }
}

impl TomlConfig<File> for Config {}
