use client::{config::Config, CommunicationError};
use protocol::DhcpOffer;
use rand::Rng;
use rustyline::{error::ReadlineError, DefaultEditor};
use std::{
    error::Error,
    io,
    net::{TcpStream, ToSocketAddrs},
    num::ParseIntError,
    ops::Range,
    path::{Path, PathBuf},
    process::exit,
    time::{Duration, Instant},
};
use thiserror::Error;

type ServerIndex = usize;
type QueryCount = u32;

enum QueryTarget<'a> {
    Random,
    Specific(ServerIndex),
    Arbitrary(&'a str),
}

enum QueryMode {
    Discover,
    Renew,
}

struct Query<'a> {
    batch: QueryCount,
    mode: QueryMode,
    target: QueryTarget<'a>,
    verbose: bool,
}

enum Command<'a> {
    Nop,
    Query(Query<'a>),
    List,
    Help,
    Quit,
}

fn help() {
    println!(
        r#"---- DHCP Client ----
Supported commands:
    query [OPTION]... [SERVER]
        OPTION:
            -n N  number of batch queries (default: 1)
            -r    renew
            -v    print all responses even when batch querying
        SERVER: 
            random    pick randomly from configured servers (default)
            <index>   index number of server to connect to (see list)
            <host>    arbitrary host, e.g. dhcp.example.com:4321
    list
        Lists configured servers
    help
        Prints this help
    quit
        Exit the CLI

Supported shorthands: qr, ls, h, q
"#
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
}

impl<'a> Query<'a> {
    fn parse(mut args: &[&'a str]) -> Result<Self, CommandParseError<'a>> {
        let mut batch = 1;
        let mut mode = QueryMode::Discover;
        let mut verbose = false;

        loop {
            match args {
                ["-n", count, rest @ ..] => {
                    batch = count.parse()?;
                    args = rest;
                }
                ["-r", rest @ ..] => {
                    mode = QueryMode::Renew;
                    args = rest;
                }
                ["-v", rest @ ..] => {
                    verbose = true;
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
            mode,
            target,
            verbose,
        })
    }
}

fn parse_command(line: &str) -> Result<Command, CommandParseError> {
    let command: Vec<&str> = line.split_ascii_whitespace().collect();
    match command.as_slice() {
        ["exit" | "quit" | "q"] => Ok(Command::Quit),
        ["query" | "qr", args @ ..] => Ok(Command::Query(Query::parse(args)?)),
        ["list" | "ls"] => Ok(Command::List),
        ["help" | "h"] => Ok(Command::Help),
        [] => Ok(Command::Nop),
        [cmd, ..] => Err(CommandParseError::UnrecognizedCommand(cmd)),
    }
}

fn list(config: &Config) {
    println!("---- Configured Servers ----");
    for (i, name) in config.servers.iter().enumerate() {
        println!("{i}: {name}");
    }
    println!();
}

fn random_server(config: &Config) -> &str {
    let mut rng = rand::thread_rng();
    config.servers[rng.gen_range(0..config.servers.len())].as_str()
}

fn random_mac_addr() -> [u8; 6] {
    let mut rng = rand::thread_rng();
    rng.gen()
}

#[derive(Error, Debug)]
enum QueryError {
    #[error("Name resolution failed")]
    NameResolution(#[from] io::Error),
    #[error("The server is unreachable")]
    UnreachableServer,
    #[error("Communication with the server failed")]
    Communication(#[from] CommunicationError),
    #[error("Server replied to Request with Nack")]
    RequestNack,
    #[error("Server replied to Discover with Nack")]
    DiscoverNack,
}

fn query(server: &str, default_port: u16) -> Result<(DhcpOffer, Duration), QueryError> {
    let start = Instant::now();

    // Resolve server name, try server itself first
    let mut addrs = match server.to_socket_addrs() {
        Ok(addrs) => addrs,
        Err(e) => {
            // If the server name did not contain port number, try with default port number
            if e.kind() == std::io::ErrorKind::InvalidInput {
                (server, default_port).to_socket_addrs()?
            } else {
                return Err(QueryError::NameResolution(e));
            }
        }
    }
    .peekable();

    let mac_addr = random_mac_addr();

    while let Some(addr) = addrs.next() {
        let stream = TcpStream::connect(addr)?;
        match client::get_offer(&stream, mac_addr) {
            Err(e) => match addrs.peek() {
                Some(_) => continue, // Try next DNS result
                None => return Err(e.into()),
            },
            Ok(Some(offer @ DhcpOffer { ip, .. })) => {
                if client::get_ack(&stream, mac_addr, ip)? {
                    return Ok((offer, start.elapsed()));
                } else {
                    return Err(QueryError::RequestNack);
                }
            }
            Ok(None) => return Err(QueryError::DiscoverNack),
        }
    }
    Err(QueryError::UnreachableServer)
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
        let res = match cmd.mode {
            QueryMode::Discover => query(server, config.default_port),
            QueryMode::Renew => todo!(),
        };
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
                    Ok((offer, dur)) => println!("{offer:?} received in {dur:.3?}"),
                    Err(e) => println!("failed: \x1b[93m{e}\x1b[0m"),
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
                        .and_then(|(_, dur)| dur.as_nanos().try_into().ok())
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
                    eprintln!("\x1b[93m{e1}\x1b[0m");
                    if let Some(source) = e1.source() {
                        eprintln!("{source}");
                    }
                    eprintln!("\x1b[93m{e2}\x1b[0m");
                    if let Some(source) = e2.source() {
                        eprintln!("{source}");
                    }
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
                        handle_query_command(query, &config).unwrap_or_else(|e| eprintln!("{e}"))
                    }
                    Ok(List) => list(&config),
                    Ok(Help) => help(),
                    Err(e) => eprintln!("{e}"),
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
