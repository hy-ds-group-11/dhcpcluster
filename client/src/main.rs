#![deny(clippy::unwrap_used, clippy::allow_attributes_without_reason)]
#![warn(clippy::perf, clippy::complexity, clippy::pedantic, clippy::suspicious)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    reason = "We're not going to write comprehensive docs"
)]
#![allow(
    clippy::cast_precision_loss,
    reason = "There are no sufficient floating point types"
)]

use client::{config::Config, CommunicationError};
use protocol::{DhcpOffer, MacAddr, MacAddrParseError};
use rand::Rng;
use rustyline::{error::ReadlineError, DefaultEditor};
use std::{
    any::Any,
    collections::HashMap,
    error::Error,
    fmt::Display,
    io,
    iter::Peekable,
    net::{AddrParseError, Ipv4Addr, SocketAddr, TcpStream, ToSocketAddrs},
    num::{NonZero, ParseIntError},
    ops::{ControlFlow, Range},
    path::{Path, PathBuf},
    process::exit,
    str::FromStr,
    sync::mpsc,
    time::{Duration, Instant},
    vec::IntoIter,
};
use thiserror::Error;
use thread_pool::ThreadPool;
use toml_config::TomlConfig;

type ServerIndex = usize;
type QueryCount = u32;

enum QueryTarget<'a> {
    Random,
    Specific(ServerIndex),
    Arbitrary(&'a str),
}

struct Query<'a> {
    threads: Option<NonZero<usize>>,
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
        "---- DHCP Client v{} ----
Supported commands:
    \x1b[32mquery\x1b[0m [\x1b[36mOPTION\x1b[0m]... [\x1b[36mSERVER\x1b[0m]
        \x1b[36mOPTION\x1b[0m:
            -n <N>           number of batch queries (default: 1)
            -t <N>           number of threads to use (default: check conf)
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
    , env!("CARGO_PKG_VERSION"));
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
        let mut threads = None;
        let mut batch = 1;
        let mut verbose = false;
        let mut mac_address = None;

        loop {
            match args {
                ["-t", count, rest @ ..] => {
                    threads = count.parse::<usize>()?.try_into().ok();
                    args = rest;
                }
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
            threads,
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
enum QueryError {
    #[error("Name resolution failed for {server}")]
    NameResolution { server: String, source: io::Error },
    #[error("Failed to set socket timeout")]
    SetSocketTimeout(#[source] io::Error),
    #[error("Failed to establish a connection to {server}")]
    Connect { server: String, source: io::Error },
    #[error("The server name {server} resolved to 0 addresses, can't reach server")]
    UnreachableServer { server: String },
    #[error("Communication with {server} failed")]
    Communication {
        server: String,
        source: CommunicationError,
    },
    #[error("Server {server} replied to Request with Nack")]
    RequestNack { server: String },
    #[error("Server {server} replied to Discover with Nack")]
    DiscoverNack { server: String },
}

fn connect_timeout(
    addr: &SocketAddr,
    timeout: Option<Duration>,
    server: &str,
) -> Result<TcpStream, QueryError> {
    let stream = if let Some(timeout) = timeout {
        TcpStream::connect_timeout(addr, timeout)
    } else {
        TcpStream::connect(addr)
    }
    .map_err(|e| QueryError::Connect {
        server: server.to_owned(),
        source: e,
    })?;
    stream
        .set_read_timeout(timeout)
        .map_err(QueryError::SetSocketTimeout)?;
    stream
        .set_write_timeout(timeout)
        .map_err(QueryError::SetSocketTimeout)?;
    Ok(stream)
}

#[derive(Clone)]
struct QuerySuccess {
    offer: DhcpOffer,
    server: String,
    started: Instant,
    time: Duration,
}

impl Display for QuerySuccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Got lease \x1b[32m{}\x1b[0m, from \x1b[32m{}\x1b[0m in \x1b[32m{:.3?}\x1b[0m",
            self.offer, self.server, self.time
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
        let mut stream = connect_timeout(&addr, timeout, server)?;
        match client::get_offer(&mut stream, mac_address) {
            Err(e) => match addrs.peek() {
                Some(_) => continue, // Try next DNS result
                None => {
                    return Err(QueryError::Communication {
                        server: server.to_owned(),
                        source: e,
                    })
                }
            },
            Ok(Some(offer @ DhcpOffer { ip, .. })) => {
                if client::get_ack(&mut stream, mac_address, ip).map_err(|e| {
                    QueryError::Communication {
                        server: server.to_owned(),
                        source: e,
                    }
                })? {
                    return Ok(QuerySuccess {
                        offer,
                        server: server.to_owned(),
                        started: start,
                        time: start.elapsed(),
                    });
                }
                return Err(QueryError::RequestNack {
                    server: server.to_owned(),
                });
            }
            Ok(None) => {
                return Err(QueryError::DiscoverNack {
                    server: server.to_owned(),
                })
            }
        }
    }
    Err(QueryError::UnreachableServer {
        server: server.to_owned(),
    })
}

fn renew(
    server: &str,
    default_port: u16,
    timeout: Option<Duration>,
    mac_address: MacAddr,
    lease: &DhcpOffer,
) -> Result<QuerySuccess, QueryError> {
    let start = Instant::now();

    let mut addrs = get_addresses(server, default_port)?;

    while let Some(addr) = addrs.next() {
        let mut stream = connect_timeout(&addr, timeout, server)?;
        match client::get_ack(&mut stream, mac_address, lease.ip) {
            Err(e) => match addrs.peek() {
                Some(_) => continue, // Try next DNS result
                None => {
                    return Err(QueryError::Communication {
                        server: server.to_owned(),
                        source: e,
                    })
                }
            },
            Ok(acked) => {
                if acked {
                    return Ok(QuerySuccess {
                        offer: DhcpOffer {
                            ip: lease.ip,
                            lease_time: lease.lease_time,
                            subnet_mask: lease.subnet_mask,
                        },
                        server: server.to_owned(),
                        started: start,
                        time: start.elapsed(),
                    });
                }
                return Err(QueryError::RequestNack {
                    server: server.to_owned(),
                });
            }
        }
    }
    Err(QueryError::UnreachableServer {
        server: server.to_owned(),
    })
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
                (server, default_port).to_socket_addrs().map_err(|e| {
                    QueryError::NameResolution {
                        server: server.to_owned(),
                        source: e,
                    }
                })?
            } else {
                return Err(QueryError::NameResolution {
                    server: server.to_owned(),
                    source: e,
                });
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
    #[error("Failed to spawn worker threads")]
    ThreadPoolExecute(#[source] io::Error),
}

fn handle_query_command(
    cmd: &Query,
    config: &Config,
    thread_pool: &ThreadPool,
) -> Result<Vec<QuerySuccess>, QueryExecutionError> {
    let server = match cmd.target {
        QueryTarget::Random => None,
        QueryTarget::Specific(i) => Some(config.servers.get(i).map(String::as_str).ok_or(
            QueryExecutionError::InvalidServerIndex(i, 0..config.servers.len()),
        )?),
        QueryTarget::Arbitrary(host) => Some(host),
    };

    // Set thread_pool size
    if let Some(threads) = cmd.threads {
        thread_pool
            .set_size(threads)
            .map_err(QueryExecutionError::ThreadPoolExecute)?;
    } else {
        thread_pool
            .set_size(config.thread_count)
            .map_err(QueryExecutionError::ThreadPoolExecute)?;
    }

    let start = Instant::now();

    let (tx, rx) = mpsc::channel();
    for _ in 0..cmd.batch {
        let default_port = config.default_port;
        let timeout = config.timeout;
        let server = server.unwrap_or_else(|| random_server(config)).to_owned();
        let mac_address = cmd.mac_address.unwrap_or_else(random_mac_addr);

        let tx = tx.clone();
        thread_pool
            .execute(move || {
                tx.send(query(&server, default_port, timeout, mac_address))
                    .expect(
                    "Invariant violated: channel dropped before query command execution finished",
                );
            })
            .map_err(QueryExecutionError::ThreadPoolExecute)?;
    }
    drop(tx);

    let results: Vec<_> = rx.into_iter().collect();
    let time = start.elapsed();
    let successful: Vec<_> = results
        .iter()
        .filter_map(|res| res.as_ref().ok())
        .cloned()
        .collect();

    match (cmd.batch, cmd.verbose) {
        (0, true) => {
            // Easter egg :)
            println!("You found the optimal arguments.");
            println!("Rate: Blazingly fast!!!");
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
                "Successful: {}{}\x1b[0m / \x1b[32m{}\x1b[0m",
                if successful.len() == cmd.batch as usize {
                    "\x1b[32m"
                } else {
                    "\x1b[31m"
                },
                successful.len(),
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
                println!(
                    "Min query time: \x1b[32m{:.3?}\x1b[0m",
                    Duration::from_nanos(min_time)
                );
            }
            if let (sum, Ok(count @ 1..)) = (
                query_times_nanos().sum::<u64>(),
                query_times_nanos().count().try_into(),
            ) {
                println!(
                    "Avg query time: \x1b[32m{:.3?}\x1b[0m",
                    Duration::from_nanos(sum / count)
                );
            }
            if let Some(max_time) = query_times_nanos().max() {
                println!(
                    "Max query time: \x1b[32m{:.3?}\x1b[0m",
                    Duration::from_nanos(max_time)
                );
            }

            let rate = (successful.len() as f64 / time.as_nanos() as f64) * 10f64.powi(9);
            println!(
                "Queries took \x1b[32m{time:.3?}\x1b[0m, rate: \x1b[32m{rate:.1}\x1b[0m leases/s"
            );
        }
    }

    Ok(successful)
}

fn handle_renew_command(
    cmd: &Renew,
    lease: &DhcpOffer,
    config: &Config,
) -> Result<Option<QuerySuccess>, QueryExecutionError> {
    let server = match cmd.target {
        QueryTarget::Random => random_server(config),
        QueryTarget::Specific(i) => config.servers.get(i).map(String::as_str).ok_or(
            QueryExecutionError::InvalidServerIndex(i, 0..config.servers.len()),
        )?,
        QueryTarget::Arbitrary(host) => host,
    };

    let res = renew(
        server,
        config.default_port,
        config.timeout,
        cmd.mac_address,
        lease,
    );

    match res {
        Ok(success) => {
            println!("{success}");
            Ok(Some(success))
        }
        Err(e) => {
            print_error(&e);
            Ok(None)
        }
    }
}

fn execute_command(
    command: Command,
    config: &Config,
    thread_pool: &ThreadPool,
    leases: &mut HashMap<Ipv4Addr, QuerySuccess>,
) -> ControlFlow<()> {
    match command {
        Command::Nop => {}
        Command::Quit => return ControlFlow::Break(()),
        Command::Query(query) => match handle_query_command(&query, config, thread_pool) {
            Ok(successful) => {
                for success in successful {
                    let ip = success.offer.ip;
                    let server = success.server.clone();
                    if let Some(previous) = leases.insert(ip, success) {
                        println!("\x1b[31mGot duplicate lease for {ip} from {server}\x1b[0m");
                        let server = previous.server;
                        println!("{ip} was previously obtained from {server}");
                    }
                }
            }
            Err(e) => print_error(&e),
        },
        Command::Renew(renew) => {
            if let Some(lease) = leases.get(&renew.ip_address) {
                match handle_renew_command(&renew, &lease.offer, config) {
                    Ok(Some(success)) => {
                        leases.insert(success.offer.ip, success);
                    }
                    Ok(None) => {}
                    Err(e) => print_error(&e),
                }
            } else {
                println!("Client currently cannot attempt to renew:");
                println!("- Leases it did not acquire previously");
                println!("- Expired leases");
                // TODO: and additionally, it doesn't associate MAC address with lease anywhere.
                // If it did, we wouldn't necessarily have to specify MAC manyally
            }
        }
        Command::GenerateMac => println!("{}", random_mac_addr()),
        Command::List => list(config),
        Command::Conf => println!("\x1b[32m{config:#?}\x1b[0m"),
        Command::Help => help(),
    }

    ControlFlow::Continue(())
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

    let thread_pool = match ThreadPool::new(
        config.thread_count,
        Box::new(|worker_id: usize, msg: Box<dyn Any>| {
            eprintln!("Worker {worker_id} panicked");
            // Try both &str and String, I don't know which type panic messages will occur in
            if let Some(msg) = msg
                .downcast_ref::<&str>()
                .map(ToString::to_string)
                .or(msg.downcast_ref::<String>().cloned())
            {
                eprintln!("{msg}");
            }
        }),
    ) {
        Ok(pool) => pool,
        Err(e) => {
            print_error(&e);
            exit(2);
        }
    };

    list(&config);
    help();

    let mut leases = HashMap::new();

    let mut rl = DefaultEditor::new()?;
    loop {
        let readline = rl.readline("> ");
        match readline {
            Ok(line) => {
                rl.add_history_entry(line.as_str())?;

                // Prune expired
                leases.retain(|_, success: &mut QuerySuccess| {
                    success.started.elapsed() < Duration::from_secs(success.offer.lease_time.into())
                });

                match parse_command(&line) {
                    Ok(command) => {
                        if execute_command(command, &config, &thread_pool, &mut leases).is_break() {
                            break;
                        }
                    }
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
