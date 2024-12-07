use client::config::Config;
use protocol::DhcpServerMessage;
use rand::Rng;
use rustyline::{error::ReadlineError, DefaultEditor};
use std::{
    error::Error,
    net::ToSocketAddrs,
    time::{Duration, Instant},
};

type ServerIndex = usize;
type QueryCount = u32;

enum Server {
    Random,
    Specific(ServerIndex),
}

enum Command {
    Quit,
    Query(QueryCount, Server),
    List,
    Help,
}

fn help() {
    println!(
        r#"---- DHCP Client ----
Supported commands:
    quit
    query N [random|<index>]
    list
    help
"#
    );
}

fn parse_command(line: &str) -> Result<Command, Box<dyn Error>> {
    let command: Vec<&str> = line.split_ascii_whitespace().collect();
    Ok(match command.as_slice() {
        ["exit" | "quit" | "q"] => Command::Quit,
        ["query" | "qr", count, rest @ ..] => Command::Query(
            count.parse()?,
            match rest {
                [] | ["random"] => Server::Random,
                [index] => Server::Specific(index.parse()?),
                _ => return Err("Invalid query command".into()),
            },
        ),
        ["list" | "ls"] => Command::List,
        ["help" | "h"] => Command::Help,
        _ => return Err("Unsupported command".into()),
    })
}

fn list(config: &Config) {
    println!("---- Configured Servers ----");
    for (i, name) in config.servers.iter().enumerate() {
        println!("{i}: {name}");
    }
}

fn random_server(config: &Config) -> &str {
    let mut rng = rand::thread_rng();
    config.servers[rng.gen_range(0..config.servers.len())].as_str()
}

fn random_mac_addr() -> [u8; 6] {
    let mut rng = rand::thread_rng();
    rng.gen()
}

fn query(server: impl ToSocketAddrs) -> Result<(DhcpServerMessage, Duration), Box<dyn Error>> {
    let start = Instant::now();

    // Resolve server name
    let mut addrs = server.to_socket_addrs()?.peekable();

    let mac_addr = random_mac_addr();

    while let Some(addr) = addrs.next() {
        match client::get_offer(addr, mac_addr) {
            Err(e) => match addrs.peek() {
                Some(_) => continue, // Try next DNS result
                None => return Err(e),
            },
            Ok(Some(offer @ DhcpServerMessage::Offer { ip, .. })) => {
                match client::get_ack(addr, mac_addr, ip)? {
                    Some(_) => return Ok((offer, start.elapsed())),
                    None => return Err("Server replied to Request with Nack".into()),
                }
            }
            Ok(None) => return Err("Server replied to Discover with Nack".into()),
            Ok(Some(_)) => unreachable!(), // Unreachable, because get_offer returns only the Offer variant. TODO: Fix this at type level
        }
    }
    Err("Can't reach server".into())
}

fn handle_query_command(
    count: QueryCount,
    server: Server,
    config: &Config,
) -> Result<(), Box<dyn Error>> {
    let mut results = Vec::new();

    let server = if let Server::Specific(i) = server {
        Some(
            config
                .servers
                .get(i)
                .map(|s| s.as_str())
                .ok_or("Invalid server index")?,
        )
    } else {
        None
    };

    for _ in 0..count {
        results.push(query(server.unwrap_or_else(|| random_server(config))));
    }

    match count {
        0..=3 => {
            for res in results {
                println!("{res:#?}");
            }
        }
        _ => {
            println!(
                "Successful: {} / {}",
                results.iter().filter(|res| res.is_ok()).count(),
                count
            );
            println!(
                "Min query time: {:?}",
                results
                    .iter()
                    .filter_map(|res| res.as_ref().ok().map(|(_, dur)| dur))
                    .min()
            );
            // TODO: avg
            println!(
                "Max query time: {:?}",
                results
                    .iter()
                    .filter_map(|res| res.as_ref().ok().map(|(_, dur)| dur))
                    .max()
            );
        }
    }

    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut config = Config::load_toml_file("config.toml")?;
    config.servers.extend(std::env::args().skip(1));

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
                    Ok(Quit) => break,
                    Ok(Query(count, server)) => handle_query_command(count, server, &config)
                        .unwrap_or_else(|e| println!("{e:?}")),
                    Ok(List) => list(&config),
                    Ok(Help) => help(),
                    Err(e) => println!("{e:?}"),
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
            Err(err) => {
                println!("Error: {:?}", err);
                break;
            }
        }
    }

    Ok(())
}
