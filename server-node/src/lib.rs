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
pub mod console;
pub mod message;
pub mod peer;

use crate::{config::Config, peer::Peer};
use message::Message;
use peer::PeerId;
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

#[derive(Debug)]
pub enum ServerThreadMessage {
    NewConnection(TcpStream),
    PeerLost(PeerId),
    ElectionTimeout,
    ProtocolMessage { sender_id: PeerId, message: Message },
}

/// The distributed DHCP server
#[derive(Debug)]
pub struct Server {
    pub start_time: SystemTime,
    config: Config,
    rx: Option<Receiver<ServerThreadMessage>>,
    tx: Sender<ServerThreadMessage>,
    coordinator_id: Option<PeerId>,
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
        let start_time = SystemTime::now();
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
                        Err(e) => console::warning!("{e:?}"),
                    }
                }
                Err(e) => {
                    console::warning!("Connection refused to peer {peer_address}\n{e:?}");
                }
            }
        }

        Self {
            start_time,
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
        // Cluster configuration change, start election (TODO, properly consider when first election should be held)
        self.start_election();

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
                ElectionTimeout => self.finish_election(),
                ProtocolMessage { sender_id, message } => match message {
                    Heartbeat => console::debug!("Received heartbeat from {sender_id}"),
                    Election => self.handle_election(sender_id, &message),
                    Okay => self.handle_okay(sender_id, &message),
                    Coordinator => self.handle_coordinator(sender_id),
                    Add(lease) => todo!(),
                    Update(lease) => todo!(),
                    _ => panic!("Server received unexpected {message:?} from {sender_id}"),
                },
            }
            console::render(&self);
        }

        peer_listener_thread.join().map_err(|e| {
            format!(
                "Node listener thread panicked, Err: {:?}",
                e.downcast_ref::<&str>()
            )
            .into()
        })
    }

    fn handle_election(&mut self, sender_id: PeerId, message: &Message) {
        console::log!("Peer {sender_id} invited {message:?}");
        if sender_id < self.config.id {
            console::log!("Received Election from lower id");
            self.peer(sender_id).unwrap().send_message(Message::Okay);
            self.start_election();
        }
    }

    fn handle_okay(&mut self, sender_id: PeerId, message: &Message) {
        assert!(sender_id > self.config.id);
        console::log!("Peer {sender_id}: {message:?}, ");
        if self.local_role == ServerRole::WaitingForElection {
            console::log!("Stepping down to Follower");
            self.local_role = ServerRole::Follower;
        } else {
            console::log!("Already stepped down");
        }
    }

    fn handle_coordinator(&mut self, sender_id: PeerId) {
        console::log!("Recognizing {sender_id} as Coordinator");
        self.coordinator_id = Some(sender_id);
        self.local_role = ServerRole::Follower;
        if sender_id < self.config.id {
            self.start_election();
        }
    }

    fn listen_nodes(listener: TcpListener, server_tx: Sender<ServerThreadMessage>) {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => server_tx
                    .send(ServerThreadMessage::NewConnection(stream))
                    .unwrap(),
                Err(e) => console::warning!("{e:?}"),
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
                let message = message::recv_timeout(&stream, config.heartbeat_timeout).unwrap();

                match message {
                    Message::JoinAck(peer_id) => {
                        console::log!("Connected to peer {peer_id}");
                        Ok(Peer::new(
                            stream,
                            peer_id,
                            server_tx,
                            config.heartbeat_timeout,
                        ))
                    }
                    _ => panic!("Peer responded to Join with something other than JoinAck"),
                }
            }
            Err(e) => Err(format!("Handshake failed! {e:?}").into()),
        }
    }

    fn answer_handshake(&mut self, stream: TcpStream) {
        let message = message::recv_timeout(&stream, self.config.heartbeat_timeout).unwrap();

        match message {
            Message::Join(peer_id) => {
                let result = message::send(&stream, &Message::JoinAck(self.config.id));
                match result {
                    Ok(_) => {
                        console::log!("Peer {peer_id} joined");
                        self.peers.push(Peer::new(
                            stream,
                            peer_id,
                            self.tx.clone(),
                            self.config.heartbeat_timeout,
                        ));
                    }
                    Err(e) => console::warning!("{e:?}"),
                }
            }
            _ => panic!("First message of peer wasn't Join"),
        }
    }

    fn remove_peer(&mut self, peer_id: PeerId) {
        self.peers.retain(|peer| peer.id != peer_id);

        if Some(peer_id) == self.coordinator_id {
            self.start_election();
        }
    }

    fn peer(&self, peer_id: PeerId) -> Option<&Peer> {
        self.peers.iter().find(|peer| peer.id == peer_id)
    }

    /// Send [`Message::Election`] to all server peers with higher ids than this server.
    ///
    /// This function starts a timer thread which sleeps until the bully algorithm timeout expires.
    /// The timer triggers a [`ServerThreadMessage::ElectionTimeout`] to the the server thread.
    fn start_election(&mut self) {
        use ServerRole::*;

        self.local_role = WaitingForElection;

        for peer in &self.peers {
            if peer.id > self.config.id {
                peer.send_message(Message::Election);
            }
        }

        let dur = self.config.heartbeat_timeout;
        let tx = self.tx.clone();
        thread::spawn(move || {
            // Wait for peers to vote.
            console::log!("Starting election wait");
            thread::sleep(dur);
            console::log!("Election wait over");
            tx.send(ServerThreadMessage::ElectionTimeout).unwrap();
        }); // Drop JoinHandle, detaching election thread from server thread
    }

    /// Inspect current server role. Become leader and send [`Message::Coordinator`] to all peers
    /// if no event has reset the server role back to [`ServerRole::Follower`].
    fn finish_election(&mut self) {
        use ServerRole::*;
        match self.local_role {
            WaitingForElection => {
                self.local_role = Coordinator;
                self.coordinator_id = Some(self.config.id);
                for peer in &self.peers {
                    peer.send_message(Message::Coordinator);
                }
            }
            Coordinator => {
                console::log!("Already Coordinator when election ended");
            }
            Follower => {
                console::log!("Received OK during election");
            }
        }
    }
}
