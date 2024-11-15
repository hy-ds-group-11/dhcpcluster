use server_node::{config::Config, Server};
use std::{error::Error, net::TcpListener};

fn main() -> Result<(), Box<dyn Error>> {
    // TODO don't hardcode a single path here
    let config = dbg!(Config::load_toml_file("config.toml")?);

    let peer_listener = TcpListener::bind(config.address_private)?;
    let server = Server::new(config);
    server.start(peer_listener)?;

    Ok(())
}
