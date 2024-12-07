#![cfg_attr(coverage_nightly, feature(coverage_attribute))]

use server_node::{config::Config, server::Server};
use std::{
    error::Error,
    net::TcpListener,
    path::{Path, PathBuf},
};

#[cfg_attr(coverage_nightly, coverage(off))]
fn main() -> Result<(), Box<dyn Error>> {
    let config_file_path: PathBuf = std::env::args_os()
        .nth(1)
        .unwrap_or("config.toml".into())
        .into();

    let config = match Config::load_toml_file(&config_file_path) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("Didn't find {config_file_path:?}: {e}");
            match Config::load_toml_file(Path::new("server-node").join(config_file_path.clone())) {
                Ok(config) => config,
                Err(e) => {
                    eprintln!("Didn't find {config_file_path:?}: {e}");
                    return Err(e);
                }
            }
        }
    };

    let peer_listener = TcpListener::bind(config.address_private)?;
    let client_listener = TcpListener::bind(config.dhcp_address)?;
    let server = Server::connect(config);
    server.start(peer_listener, client_listener)?;

    Ok(())
}
