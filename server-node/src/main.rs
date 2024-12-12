#![cfg_attr(coverage_nightly, feature(coverage_attribute))]

use server_node::{config::Config, server::Server};
use std::{
    error::Error,
    net::TcpListener,
    path::{Path, PathBuf},
    process::exit,
};

fn print_error(mut error: &dyn Error) {
    eprintln!("\x1b[93m{error}\x1b[0m");
    while let Some(source) = error.source() {
        eprintln!("Caused by: \x1b[35m{source}\x1b[0m");
        error = source;
    }
}

#[cfg_attr(coverage_nightly, coverage(off))]
fn main() {
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
                    print_error(&e1);
                    print_error(&e2);
                    exit(1);
                }
            }
        }
    };

    // Start listening
    let peer_listener = match TcpListener::bind(config.address_private) {
        Ok(listener) => listener,
        Err(e) => {
            print_error(&e);
            exit(2);
        }
    };
    let client_listener = match TcpListener::bind(config.dhcp_address) {
        Ok(listener) => listener,
        Err(e) => {
            print_error(&e);
            exit(3);
        }
    };
    Server::start(config, peer_listener, client_listener);
}
