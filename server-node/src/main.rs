#![cfg_attr(coverage_nightly, feature(coverage_attribute))]

use server_node::{config::Config, server::Server};
use std::{
    error::Error,
    net::TcpListener,
    path::{Path, PathBuf},
    process::exit,
};

#[cfg_attr(coverage_nightly, coverage(off))]
fn main() -> Result<(), Box<dyn Error>> {
    let config_file_path: PathBuf = std::env::args_os()
        .nth(1)
        .unwrap_or("config.toml".into())
        .into();

    let config = match Config::load_toml_file(&config_file_path) {
        Ok(config) => config,
        Err(e1) => {
            let joined_path = Path::new("server-node").join(config_file_path.clone());
            match Config::load_toml_file(&joined_path) {
                Ok(config) => config,
                Err(e2) => {
                    eprintln!("\x1b[93mCouldn't read {config_file_path:?}: {e1}");
                    eprintln!("Couldn't read {joined_path:?}: {e2}\x1b[0m");
                    exit(1);
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
