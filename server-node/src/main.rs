mod config;

use config::Config;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    // TODO don't hardcode a single path here
    let config = Config::from_toml_file("config.toml")?;

    println!("{config:?}");

    Ok(())
}
