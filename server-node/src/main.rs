use server_node::{config::Config, Server};
use std::{
    error::Error,
    net::TcpListener,
    path::{Path, PathBuf},
};

fn main() -> Result<(), Box<dyn Error>> {
    let config_file_path: PathBuf = std::env::args_os()
        .nth(1)
        .unwrap_or("config.toml".into())
        .into();

    let config = match Config::load_toml_file(&config_file_path) {
        Ok(config) => config,
        Err(_) => Config::load_toml_file(Path::new("server-node").join(config_file_path))?,
    };
    dbg!(&config);

    let peer_listener = TcpListener::bind(config.address_private)?;
    let server = Server::new(config);
    server.start(peer_listener)?;

    Ok(())
}
