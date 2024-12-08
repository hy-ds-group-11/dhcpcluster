use client::config::Config;
use protocol::DhcpServerMessage;
use rand::Rng;
use std::{
    error::Error,
    io::{self, BufRead},
    net::SocketAddr,
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
        ["query", count, rest @ ..] => Command::Query(
            count.parse()?,
            match rest {
                [] | ["random"] => Server::Random,
                [index] => Server::Specific(index.parse()?),
                _ => return Err("Invalid query command".into()),
            },
        ),
        ["list"] => Command::List,
        ["help"] => Command::Help,
        _ => return Err("Unsupported command".into()),
    })
}

fn list(config: &Config) {
    println!("---- Configured Servers ----");
    for (i, name) in config.names.iter().enumerate() {
        println!("{i}: {name}");
    }
}

fn random_server(config: &Config) -> SocketAddr {
    let mut rng = rand::thread_rng();
    config.servers[rng.gen_range(0..config.servers.len())]
}

fn random_mac_addr() -> [u8; 6] {
    let mut rng = rand::thread_rng();
    rng.gen()
}

fn query(addr: SocketAddr) -> Result<(DhcpServerMessage, Duration), Box<dyn Error>> {
    let start = Instant::now();
    let mac_addr = random_mac_addr();
    let offer = client::get_offer(addr, mac_addr)?;
    match offer {
        Some(offer @ DhcpServerMessage::Offer { ip, .. }) => {
            match client::get_ack(addr, mac_addr, ip)? {
                Some(_) => Ok((offer, start.elapsed())),
                None => Err("Server replied to Request with Nack".into()),
            }
        }
        None => Err("Server replied to Discover with Nack".into()),
        Some(_) => unreachable!(),
    }
}

fn handle_query_command(
    count: QueryCount,
    server: Server,
    config: &Config,
) -> Result<(), Box<dyn Error>> {
    let mut results = Vec::new();
    for _ in 0..count {
        let server_addr = match server {
            Server::Random => random_server(config),
            Server::Specific(i) => *config.servers.get(i).ok_or("Invalid server index")?,
        };
        results.push(query(server_addr));
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
    let args: Vec<SocketAddr> = std::env::args()
        .skip(1)
        .map(|s| s.parse().unwrap())
        .collect();
    config.servers.extend(args);

    list(&config);
    help();

    let stdin = io::stdin().lock();
    let lines = stdin.lines();
    for line in lines {
        use Command::*;
        match parse_command(&line.unwrap()) {
            Ok(Quit) => break,
            Ok(Query(count, server)) => {
                handle_query_command(count, server, &config).unwrap_or_else(|e| println!("{e:?}"))
            }
            Ok(List) => list(&config),
            Ok(Help) => help(),
            Err(e) => println!("{e:?}"),
        }
    }

    Ok(())
}
