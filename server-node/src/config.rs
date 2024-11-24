use serde::Deserialize;
use std::{error::Error, ffi::OsStr, net::SocketAddr, path::Path, str::FromStr};

#[derive(Deserialize, Debug)]
struct ConfigFile {
    address_private: SocketAddr,
    peers: Vec<SocketAddr>,
    id: u32,
    // TODO DHCP interface?
}

impl FromStr for ConfigFile {
    type Err = toml::de::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        toml::from_str(s)
    }
}

#[derive(Debug)]
pub struct Config {
    pub address_private: SocketAddr,
    pub peers: Vec<SocketAddr>,
    pub id: u32,
}

impl From<ConfigFile> for Config {
    // This implementation is a no-op for now, but down the line it's possible
    // that our server configuration struct diverges from the
    // configuration file contents
    fn from(
        ConfigFile {
            address_private,
            peers,
            id,
        }: ConfigFile,
    ) -> Self {
        Self {
            address_private,
            peers,
            id,
        }
    }
}

impl Config {
    pub fn load_toml_file(path: impl AsRef<Path>) -> Result<Self, Box<dyn Error>> {
        if path.as_ref().extension() != Some(OsStr::new("toml")) {
            return Err("Config file path should end in .toml".into());
        }
        let toml = std::fs::read_to_string(path)?;
        let conf = ConfigFile::from_str(&toml)?;
        Ok(conf.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    fn crate_path(path: impl AsRef<Path>) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(path)
    }

    fn tests_path(path: impl AsRef<Path>) -> PathBuf {
        crate_path("tests").join(path)
    }

    #[test]
    fn load_empty_toml() {
        // This should fail because there are non-optional fields
        let conf = Config::load_toml_file(tests_path("empty.toml"));
        assert!(conf.is_err());
    }

    #[test]
    fn load_wrong_extension() {
        let res = Config::load_toml_file(tests_path("config.json"));
        assert_eq!(
            res.unwrap_err().to_string(),
            "Config file path should end in .toml"
        );
    }

    #[test]
    fn load_no_extension() {
        let res = Config::load_toml_file(tests_path("config"));
        assert_eq!(
            res.unwrap_err().to_string(),
            "Config file path should end in .toml"
        );
    }

    #[test]
    fn load_nonexisting() {
        let res = Config::load_toml_file(tests_path("nonexisting.toml"));
        assert_eq!(
            res.unwrap_err()
                .downcast::<std::io::Error>()
                .unwrap()
                .kind(),
            std::io::ErrorKind::NotFound
        );
    }

    #[test]
    fn load_repository_example_file() {
        let conf = Config::load_toml_file(crate_path("config.toml")).unwrap();
        assert_eq!(conf.address_private, "0.0.0.0:1234".parse().unwrap());
        assert_eq!(conf.id, 0);
        assert!(conf.peers.is_empty());
    }

    #[test]
    fn load_test_file_1() {
        let conf = Config::load_toml_file(tests_path("test1.toml")).unwrap();
        assert_eq!(conf.address_private, "127.0.0.1:65534".parse().unwrap());
        assert_eq!(conf.id, 19999999);
        assert_eq!(
            conf.peers,
            vec![
                "1.1.1.1:123".parse().unwrap(),
                "192.168.1.1:8000".parse().unwrap()
            ]
        );
    }
}
