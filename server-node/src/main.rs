use server_node::{config::Config, Server};
use std::{error::Error, net::TcpListener};

fn main() -> Result<(), Box<dyn Error>> {
    // TODO don't hardcode a single path here
    let config = dbg!(Config::from_toml_file("config.toml")?);

    let listener = TcpListener::bind(config.address_private)?;
    let server = Server::new(config)?;
    server.start(listener)?;

    Ok(())
}
