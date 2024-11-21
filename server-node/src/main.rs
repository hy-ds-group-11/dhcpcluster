use server_node::{config::Config, Server};
use std::{error::Error, net::TcpListener};

fn main() -> Result<(), Box<dyn Error>> {
    let config = match Config::load_toml_file("config.toml") {
        Ok(config) => config,
        Err(_) => Config::load_toml_file("server-node/config.toml")?,
    };
    dbg!(&config);

    let peer_listener = TcpListener::bind(config.address_private)?;
    let server = Server::new(config);
    server.start(peer_listener)?;

    Ok(())
}
