//! # DHCP Cluster - Server Implementation
//!
//! This crate contains a distributed DHCP server implementation, with a custom protocol between nodes.
//!
//! For the protocol definition, look into the [`message`] module.
//!
//! The server architecture comprises of threads, which use blocking operations to communicate over [`TcpStream`]s.
//! There are two threads per active peer, one for receiving messages and one for sending messages.
//! There is also a server logic thread, handling bookkeeping for the peer- and client events.
//!
//! For the communication thread implementation, look into the [`peer`] module.

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]

pub mod config;
pub mod message;
pub mod peer;

use crate::{config::Config, peer::Peer};
use message::Message;
use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    net::{Ipv4Addr, TcpListener, TcpStream},
    sync::mpsc::{channel, Receiver, Sender},
    thread,
    time::SystemTime,
};

#[derive(Debug, PartialEq)]
enum ServerRole {
    WaitingForElection,
    Coordinator,
    Follower,
}

/// A DHCP Lease
#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct Lease {
    hardware_address: [u8; 6],    // Assume MAC address
    lease_address: Ipv4Addr,      // Assume IPv4 for now
    expiry_timestamp: SystemTime, // SystemTime as exact time is not critical, and we want a timestamp
}

pub enum ServerThreadMessage {
    NewConnection(TcpStream),
    PeerLost(u32),
    ProtocolMessage(Message),
}

/// The distributed DHCP server
pub struct Server {
    config: Config,
    rx: Option<Receiver<ServerThreadMessage>>,
    tx: Sender<ServerThreadMessage>,
    coordinator_id: Option<u32>,
    #[allow(dead_code)]
    leases: Vec<Lease>,
    peers: Vec<Peer>,
    local_role: ServerRole,
}

impl Server {
    /// Initialize shared state, and try connecting to peers.
    /// Failed handshakes are ignored, since they might be nodes that are starting later.
    /// After connecting to peers, you probably want to call [`Server::start`].
    pub fn connect(config: Config) -> Self {
        let coordinator_id = None;
        let leases = Vec::new();
        let mut peers = Vec::new();
        let local_role = ServerRole::Follower;

        let (tx, rx) = channel::<ServerThreadMessage>();

        for peer_address in &config.peers {
            match TcpStream::connect(peer_address) {
                Ok(stream) => {
                    let result = Self::start_handshake(stream, &config, tx.clone());
                    match result {
                        Ok(peer) => peers.push(peer),
                        Err(e) => eprintln!("{e:?}"),
                    }
                }
                Err(e) => eprintln!("{e:?}"),
            }
        }

        Self {
            config,
            rx: Some(rx),
            tx,
            coordinator_id,
            leases,
            peers,
            local_role,
        }
    }

    /// Start listening to incoming connections from server peers and DHCP clients.
    ///
    /// This function may return, but only in error situations. Error handling is TBD and WIP.
    /// Otherwise consider it as a blocking operation that loops and never returns control back to the caller.
    pub fn start(mut self, peer_listener: TcpListener) -> Result<(), Box<dyn Error>> {
        let server_tx = self.tx.clone();
        let peer_listener_thread =
            thread::spawn(move || Self::listen_nodes(peer_listener, server_tx));

        // TODO: start client listener thread. Using scoped threads here may make the code look nicer,
        // decide on that later.

        let rx = self.rx.take().unwrap();
        use Message::*;
        use ServerThreadMessage::*;
        for message in rx.iter() {
            #[allow(unused_variables)]
            match message {
                NewConnection(tcp_stream) => self.answer_handshake(tcp_stream),
                PeerLost(peer_id) => self.remove_peer(peer_id),
                ProtocolMessage(protocol_message) => match protocol_message {
                    Election => todo!(),
                    Okay => todo!(),
                    Coordinator => todo!(),
                    Add(lease) => todo!(),
                    Update(lease) => todo!(),
                    _ => panic!("Server received unexpected message {protocol_message:?}"),
                },
            }
        }

        peer_listener_thread.join().map_err(|e| {
            format!(
                "Node listener thread panicked, Err: {:?}",
                e.downcast_ref::<&str>()
            )
            .into()
        })
    }

    fn listen_nodes(listener: TcpListener, server_tx: Sender<ServerThreadMessage>) {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => server_tx
                    .send(ServerThreadMessage::NewConnection(stream))
                    .unwrap(),
                Err(e) => eprintln!("{e:?}"),
            }
        }
    }

    fn start_handshake(
        stream: TcpStream,
        config: &Config,
        server_tx: Sender<ServerThreadMessage>,
    ) -> Result<Peer, Box<dyn Error>> {
        let result = message::send(&stream, &Message::Join(config.id));
        match result {
            Ok(_) => {
                stream
                    .set_read_timeout(Some(config.heartbeat_timeout))
                    .unwrap();
                let message = message::receive(&stream).unwrap();
                stream.set_read_timeout(None).unwrap();

                match dbg!(message) {
                    Message::JoinAck(peer_id) => Ok(Peer::new(
                        stream,
                        peer_id,
                        server_tx,
                        config.heartbeat_timeout,
                    )),
                    _ => panic!("Peer responded to Join with something other than JoinAck"),
                }
            }
            Err(e) => {
                eprintln!("{e:?}");
                Err(e.into())
            }
        }
    }

    fn answer_handshake(&mut self, stream: TcpStream) {
        stream
            .set_read_timeout(Some(self.config.heartbeat_timeout))
            .unwrap();
        let message = message::receive(&stream).unwrap();
        stream.set_read_timeout(None).unwrap();

        match message {
            Message::Join(peer_id) => {
                let result = message::send(&stream, &Message::JoinAck(self.config.id));
                match result {
                    Ok(_) => {
                        self.peers.push(Peer::new(
                            stream,
                            peer_id,
                            self.tx.clone(),
                            self.config.heartbeat_timeout,
                        ));
                    }
                    Err(e) => eprintln!("{e:?}"),
                }
            }
            _ => panic!("First message of peer wasn't Join"),
        }
    }

    fn remove_peer(&mut self, peer_id: u32) {
        self.peers.retain(|peer| peer.id != peer_id);

        if Some(peer_id) == self.coordinator_id {
            self.start_election();
        }
    }

    /// Send [`Message::Election`] to all server peers with higher ids than this server.
    ///
    /// This function blocks until the bully algorithm timeout expires.
    fn start_election(&mut self) {
        // TODO Move the waiting into a separate thread so we don't block the main server thread
        use ServerRole::*;

        assert_eq!(self.local_role, Follower);
        self.local_role = WaitingForElection;

        for peer in &self.peers {
            if peer.id > self.config.id {
                peer.send_message(Message::Election);
            }
        }

        // Wait for peers to vote.
        thread::sleep(self.config.heartbeat_timeout);

        match self.local_role {
            WaitingForElection => {
                self.local_role = Coordinator;
                self.coordinator_id = Some(self.config.id);
                for peer in &self.peers {
                    peer.send_message(Message::Coordinator);
                }
            }
            Coordinator => unreachable!(),
            Follower => {
                println!("Received OK during election");
            }
        }
    }
}
