use client::{config::Config, CommunicationError};
use protocol::{DhcpOffer, MacAddr, MacAddrParseError};
use rand::Rng;
use rustyline::{error::ReadlineError, DefaultEditor};
use std::{
    error::Error,
    fmt::Display,
    io,
    iter::Peekable,
    net::{AddrParseError, Ipv4Addr, SocketAddr, TcpStream, ToSocketAddrs},
    num::ParseIntError,
    ops::Range,
    path::{Path, PathBuf},
    process::exit,
    str::FromStr,
    time::{Duration, Instant},
    vec::IntoIter,
};
use thiserror::Error;
use toml_config::TomlConfig;

type ServerIndex = usize;
type QueryCount = u32;

enum QueryTarget<'a> {
    Random,
    Specific(ServerIndex),
    Arbitrary(&'a str),
}

struct Query<'a> {
    batch: QueryCount,
    target: QueryTarget<'a>,
    verbose: bool,
    mac_address: Option<MacAddr>,
}

struct Renew<'a> {
    target: QueryTarget<'a>,
    mac_address: MacAddr,
    ip_address: Ipv4Addr,
}

enum Command<'a> {
    Nop,
    Query(Query<'a>),
    Renew(Renew<'a>),
    GenerateMac,
    List,
    Help,
    Conf,
    Quit,
}

fn print_error(mut error: &dyn Error) {
    eprintln!("\x1b[93m{error}\x1b[0m");
    while let Some(source) = error.source() {
        eprintln!("Caused by: \x1b[35m{source}\x1b[0m");
        error = source;
    }
}

fn help() {
    println!(
        "---- DHCP Client ----
Supported commands:
    \x1b[32mquery\x1b[0m [\x1b[36mOPTION\x1b[0m]... [\x1b[36mSERVER\x1b[0m]
        \x1b[36mOPTION\x1b[0m:
            -n <N>           number of batch queries (default: 1)
            -v               print all responses even when batch querying
            -m <MAC_ADDRESS> specify MAC-address (otherwise randomized)
        \x1b[36mSERVER\x1b[0m: 
            random           pick randomly from configured servers (default)
            <index>          index number of server to connect to (see list)
            <host>           arbitrary host, e.g. dhcp.example.com:4321
    \x1b[32mrenew\x1b[0m <\x1b[36mMAC_ADDRESS\x1b[0m> <\x1b[36mIP_ADDRESS\x1b[0m> [\x1b[36mSERVER\x1b[0m]
        Renew an existing lease with MAC_ADDRESS and IP_ADDRESS from SERVER
    \x1b[32mgenerate\x1b[0m
        Generate a random MAC-address
    \x1b[32mlist\x1b[0m
        List configured servers
    \x1b[32mconf\x1b[0m
        Print current configuration
    \x1b[32mhelp\x1b[0m
        Print this help
    \x1b[32mquit\x1b[0m
        Exit the CLI

Supported shorthands: qr, re, gen, ls, cf, h, q
"
    );
}

#[derive(Error, Debug)]
enum CommandParseError<'a> {
    #[error("Invalid number in command arguments")]
    InvalidNumber(#[from] ParseIntError),
    #[error("Unrecognized arguments in command: {0:?}")]
    UnrecognizedArguments(Vec<&'a str>),
    #[error("Unrecognized command: {0:?}")]
    UnrecognizedCommand(&'a str),
    #[error("Invalid MAC address in command arguments")]
    InvalidMacAddress(#[from] MacAddrParseError),
    #[error("Invalid IP address in command arguments")]
    InvalidIpAddress(#[from] AddrParseError),
}

impl<'a> Query<'a> {
    fn parse(mut args: &[&'a str]) -> Result<Self, CommandParseError<'a>> {
        let mut batch = 1;
        let mut verbose = false;
        let mut mac_address = None;

        loop {
            match args {
                ["-n", count, rest @ ..] => {
                    batch = count.parse()?;
                    args = rest;
                }
                ["-v", rest @ ..] => {
                    verbose = true;
                    args = rest;
                }
                ["-m", address, rest @ ..] => {
                    mac_address = Some(MacAddr::from_str(address)?);
                    args = rest;
                }
                [..] => break,
            }
        }

        let target = match args {
            [] | ["random"] => QueryTarget::Random,
            [arg] => match arg.parse() {
                Ok(index) => QueryTarget::Specific(index),
                Err(_) => QueryTarget::Arbitrary(arg),
            },
            args => return Err(CommandParseError::UnrecognizedArguments(args.into()))?,
        };

        Ok(Query {
            batch,
            target,
            verbose,
            mac_address,
        })
    }
}

impl<'a> Renew<'a> {
    fn parse(args: &[&'a str]) -> Result<Self, CommandParseError<'a>> {
        match args {
            [mac_address, ip_address, rest @ ..] => {
                let target = match rest {
                    [] | ["random"] => QueryTarget::Random,
                    [target] => match target.parse() {
                        Ok(index) => QueryTarget::Specific(index),
                        Err(_) => QueryTarget::Arbitrary(target),
                    },
                    rest => Err(CommandParseError::UnrecognizedArguments(rest.into()))?,
                };
                Ok(Renew {
                    target,
                    mac_address: mac_address.parse()?,
                    ip_address: ip_address.parse()?,
                })
            }
            args => Err(CommandParseError::UnrecognizedArguments(args.into()))?,
        }
    }
}

fn parse_command(line: &str) -> Result<Command, CommandParseError> {
    let command: Vec<&str> = line.split_ascii_whitespace().collect();
    match command.as_slice() {
        ["exit" | "quit" | "q"] => Ok(Command::Quit),
        ["query" | "qr", args @ ..] => Ok(Command::Query(Query::parse(args)?)),
        ["renew" | "re", args @ ..] => Ok(Command::Renew(Renew::parse(args)?)),
        ["generate" | "gen"] => Ok(Command::GenerateMac),
        ["list" | "ls"] => Ok(Command::List),
        ["conf" | "cf"] => Ok(Command::Conf),
        ["help" | "h"] => Ok(Command::Help),
        [] => Ok(Command::Nop),
        [cmd, ..] => Err(CommandParseError::UnrecognizedCommand(cmd)),
    }
}

fn list(config: &Config) {
    println!("---- Configured Servers ----");
    for (i, name) in config.servers.iter().enumerate() {
        println!("\x1b[36m{i}\x1b[0m: {name}");
    }
    println!();
}

fn random_server(config: &Config) -> &str {
    let mut rng = rand::thread_rng();
    config.servers[rng.gen_range(0..config.servers.len())].as_str()
}

fn random_mac_addr() -> MacAddr {
    let mut rng = rand::thread_rng();
    let bytes: [u8; 6] = rng.gen();
    bytes.into()
}

#[derive(Error, Debug)]
enum QueryError<'a> {
    #[error("Name resolution failed for {server}")]
    NameResolution { server: &'a str, source: io::Error },
    #[error("Failed to set socket timeout")]
    SetSocketTimeout { source: io::Error },
    #[error("Failed to establish a connection to {server}")]
    Connect { server: &'a str, source: io::Error },
    #[error("The server name {server} resolved to 0 addresses, can't reach server")]
    UnreachableServer { server: &'a str },
    #[error("Communication with {server} failed")]
    Communication {
        server: &'a str,
        source: CommunicationError,
    },
    #[error("Server {server} replied to Request with Nack")]
    RequestNack { server: &'a str },
    #[error("Server {server} replied to Discover with Nack")]
    DiscoverNack { server: &'a str },
}

fn connect_timeout<'a>(
    addr: &SocketAddr,
    timeout: Option<Duration>,
    server: &'a str,
) -> Result<TcpStream, QueryError<'a>> {
    let stream = if let Some(timeout) = timeout {
        TcpStream::connect_timeout(addr, timeout)
    } else {
        TcpStream::connect(addr)
    }
    .map_err(|e| QueryError::Connect { server, source: e })?;
    stream
        .set_read_timeout(timeout)
        .map_err(|e| QueryError::SetSocketTimeout { source: e })?;
    stream
        .set_write_timeout(timeout)
        .map_err(|e| QueryError::SetSocketTimeout { source: e })?;
    Ok(stream)
}

struct QuerySuccess<'a> {
    offer: DhcpOffer,
    server: &'a str,
    time: Duration,
}

impl Display for QuerySuccess<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Got lease \x1b[32m{}\x1b[0m, from \x1b[32m{}\x1b[0m in \x1b[32m{:.3?}\x1b[0m",
            self.offer, self.server, self.time,
        )
    }
}

fn query(
    server: &str,
    default_port: u16,
    timeout: Option<Duration>,
    mac_address: MacAddr,
) -> Result<QuerySuccess, QueryError> {
    let start = Instant::now();

    let mut addrs = get_addresses(server, default_port)?;

    while let Some(addr) = addrs.next() {
        let stream = connect_timeout(&addr, timeout, server)?;
        match client::get_offer(&stream, mac_address) {
            Err(e) => match addrs.peek() {
                Some(_) => continue, // Try next DNS result
                None => return Err(QueryError::Communication { server, source: e }),
            },
            Ok(Some(offer @ DhcpOffer { ip, .. })) => {
                if client::get_ack(&stream, mac_address, ip)
                    .map_err(|e| QueryError::Communication { server, source: e })?
                {
                    return Ok(QuerySuccess {
                        offer,
                        server,
                        time: start.elapsed(),
                    });
                } else {
                    return Err(QueryError::RequestNack { server });
                }
            }
            Ok(None) => return Err(QueryError::DiscoverNack { server }),
        }
    }
    Err(QueryError::UnreachableServer { server })
}

fn renew(
    server: &str,
    default_port: u16,
    timeout: Option<Duration>,
    mac_address: MacAddr,
    ip_address: Ipv4Addr,
) -> Result<QuerySuccess, QueryError> {
    let start = Instant::now();

    let mut addrs = get_addresses(server, default_port)?;

    while let Some(addr) = addrs.next() {
        let stream = connect_timeout(&addr, timeout, server)?;
        match client::get_ack(&stream, mac_address, ip_address) {
            Err(e) => match addrs.peek() {
                Some(_) => continue, // Try next DNS result
                None => return Err(QueryError::Communication { server, source: e }),
            },
            Ok(acked) =>
            // TODO: offer TTL and subnet mask are incorrect;
            {
                if acked {
                    return Ok(QuerySuccess {
                        offer: DhcpOffer {
                            ip: ip_address,
                            lease_time: 3600,
                            subnet_mask: 24,
                        },
                        server,
                        time: start.elapsed(),
                    });
                } else {
                    return Err(QueryError::RequestNack { server });
                }
            }
        }
    }
    Err(QueryError::UnreachableServer { server })
}

/// Resolve server name, try server itself first
fn get_addresses(
    server: &str,
    default_port: u16,
) -> Result<Peekable<IntoIter<SocketAddr>>, QueryError> {
    let addrs = match server.to_socket_addrs() {
        Ok(addrs) => addrs,
        Err(e) => {
            // If the server name did not contain port number, try with default port number
            if e.kind() == std::io::ErrorKind::InvalidInput {
                (server, default_port)
                    .to_socket_addrs()
                    .map_err(|e| QueryError::NameResolution { server, source: e })?
            } else {
                return Err(QueryError::NameResolution { server, source: e });
            }
        }
    }
    .peekable();
    Ok(addrs)
}

#[derive(Error, Debug)]
enum QueryExecutionError {
    #[error("Invalid server index {0}, expected {1:?}")]
    InvalidServerIndex(usize, Range<usize>),
}

fn handle_query_command(cmd: Query, config: &Config) -> Result<(), QueryExecutionError> {
    let mut results = Vec::new();

    let server = match cmd.target {
        QueryTarget::Random => None,
        QueryTarget::Specific(i) => Some(config.servers.get(i).map(|s| s.as_str()).ok_or(
            QueryExecutionError::InvalidServerIndex(i, 0..config.servers.len()),
        )?),
        QueryTarget::Arbitrary(host) => Some(host),
    };

    for _ in 0..cmd.batch {
        let server = server.unwrap_or_else(|| random_server(config));
        let mac_address = cmd.mac_address.unwrap_or_else(random_mac_addr);

        let res = query(server, config.default_port, config.timeout, mac_address);
        results.push(res);
    }

    match (cmd.batch, cmd.verbose) {
        (0, true) => {
            // Easter egg :)
            println!("You found the optimal arguments.");
        }
        (1, _) | (_, true) => {
            for res in results {
                match res {
                    Ok(success) => println!("{success}"),
                    Err(e) => print_error(&e),
                }
            }
        }
        _ => {
            println!(
                "Successful: {} / {}",
                results.iter().filter(|res| res.is_ok()).count(),
                cmd.batch
            );

            // Define helper closure to clean up redundant iterator chains
            let query_times_nanos = || {
                results.iter().filter_map(|res| {
                    res.as_ref()
                        .ok()
                        .and_then(|QuerySuccess { time, .. }| time.as_nanos().try_into().ok())
                })
            };

            if let Some(min_time) = query_times_nanos().min() {
                println!("Min query time: {:.3?}", Duration::from_nanos(min_time));
            }
            if let (sum, Ok(count @ 1..)) = (
                query_times_nanos().sum::<u64>(),
                query_times_nanos().count().try_into(),
            ) {
                println!("Avg query time: {:.3?}", Duration::from_nanos(sum / count))
            }
            if let Some(max_time) = query_times_nanos().max() {
                println!("Max query time: {:.3?}", Duration::from_nanos(max_time));
            }
        }
    }

    Ok(())
}

fn handle_renew_command(cmd: Renew, config: &Config) -> Result<(), QueryExecutionError> {
    let server = match cmd.target {
        QueryTarget::Random => random_server(config),
        QueryTarget::Specific(i) => config.servers.get(i).map(|s| s.as_str()).ok_or(
            QueryExecutionError::InvalidServerIndex(i, 0..config.servers.len()),
        )?,
        QueryTarget::Arbitrary(host) => host,
    };

    let res = renew(
        server,
        config.default_port,
        config.timeout,
        cmd.mac_address,
        cmd.ip_address,
    );

    match res {
        Ok(success) => println!("{success}"),
        Err(e) => print_error(&e),
    }

    Ok(())
}

fn main() -> Result<(), ReadlineError> {
    let config_file_path: PathBuf = std::env::args_os()
        .nth(1)
        .unwrap_or("config.toml".into())
        .into();

    let config = match Config::load_toml_file(&config_file_path) {
        Ok(config) => config,
        Err(e1) => {
            let joined_path = Path::new("client").join(config_file_path.clone());
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

    list(&config);
    help();

    let mut rl = DefaultEditor::new()?;
    loop {
        let readline = rl.readline("> ");
        match readline {
            Ok(line) => {
                rl.add_history_entry(line.as_str())?;
                use Command::*;
                match parse_command(&line) {
                    Ok(Nop) => {}
                    Ok(Quit) => break,
                    Ok(Query(query)) => {
                        handle_query_command(query, &config).unwrap_or_else(|e| print_error(&e))
                    }
                    Ok(Renew(renew)) => {
                        handle_renew_command(renew, &config).unwrap_or_else(|e| print_error(&e))
                    }
                    Ok(GenerateMac) => println!("{}", random_mac_addr()),
                    Ok(List) => list(&config),
                    Ok(Conf) => println!("\x1b[32m{config:#?}\x1b[0m"),
                    Ok(Help) => help(),
                    Err(e) => print_error(&e),
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("^C");
                break;
            }
            Err(ReadlineError::Eof) => {
                println!("^D");
                break;
            }
            Err(e) => {
                return Err(e);
            }
        }
    }

    Ok(())
}
