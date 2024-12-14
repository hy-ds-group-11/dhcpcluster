pub mod client;
pub mod peer;

use crate::{
    config::Config,
    console,
    dhcp::{self, Ipv4Range, Lease, LeaseOffer},
    message::Message,
    ThreadJoin,
};
use peer::{HandshakeError, JoinSuccess, Peer};
use protocol::{MacAddr, RecvCbor, SendCbor};
use std::{
    any::Any,
    collections::HashMap,
    fmt::Display,
    net::{Ipv4Addr, TcpListener, TcpStream},
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc,
    },
    thread,
    time::{Duration, SystemTime},
};
use thread_pool::ThreadPool;

type LeaseConfirmation = bool;

#[derive(Debug)]
pub enum Event {
    IncomingPeerConnection(TcpStream),
    EstablishedPeerConnection(JoinSuccess),
    PeerLost(peer::Id),
    ElectionTimeout,
    PeerMessage {
        sender_id: peer::Id,
        message: Message,
    },
    LeaseRequest {
        mac_address: MacAddr,
        tx: Sender<LeaseOffer>,
    },
    ConfirmRequest {
        mac_address: MacAddr,
        ip: Ipv4Addr,
        tx: Sender<LeaseConfirmation>,
    },
}

#[derive(Debug, PartialEq)]
enum ElectionState {
    WaitingForElection,
    Coordinator,
    Follower,
}

// Don't spam (re)connection, keep track of last attempt time
#[derive(Debug)]
enum ConnectAttempt {
    Never,
    Running,
    Finished(SystemTime),
}

/// The distributed DHCP server
pub struct Server {
    config: Arc<Config>,
    tx: Sender<Event>,
    thread_pool: ThreadPool,
    coordinator_id: Option<peer::Id>,
    dhcp_service: dhcp::Service,
    peers: HashMap<peer::Id, Peer>,
    election_state: ElectionState,
    majority: bool,
    last_connect_attempt: ConnectAttempt,
}

impl Server {
    /// Start listening to incoming connections from server peers and DHCP clients.
    pub fn start(config: Config, peer_listener: TcpListener, client_listener: TcpListener) {
        // Create main thread channel
        let (tx, server_rx) = mpsc::channel();
        let dhcp_service = dhcp::Service::new(config.dhcp_pool.clone(), config.lease_time);
        let thread_count = config.thread_count;
        console::log!("Starting with {} workers", thread_count);
        let mut server = Self {
            config: Arc::new(config),
            tx,
            thread_pool: ThreadPool::new(
                thread_count,
                Box::new(|worker_id: usize, msg: Box<dyn Any>| {
                    console::warning!("Worker {worker_id} panicked");
                    // Try both &str and String, I don't know which type panic messages will occur in
                    if let Some(msg) = msg
                        .downcast_ref::<&str>()
                        .map(ToString::to_string)
                        .or(msg.downcast_ref::<String>().cloned())
                    {
                        console::warning!("{}", msg);
                    }
                }),
            )
            .unwrap_or_else(|e| {
                panic!("ThreadPool failed to initialize\n{e}");
            }),
            coordinator_id: None,
            dhcp_service,
            peers: HashMap::new(),
            election_state: ElectionState::Follower,
            majority: false,
            last_connect_attempt: ConnectAttempt::Never,
        };

        let peer_listener_thread = {
            let server_tx = server.tx.clone();
            thread::Builder::new()
                .name(format!("{}::peer_listener_thread", module_path!()))
                .spawn(move || Peer::listen(&peer_listener, &server_tx))
                .expect("Cannot spawn peer listener thread")
        };

        let client_listener_thread = {
            let config = Arc::clone(&server.config);
            let server_tx = server.tx.clone();
            let thread_pool = server.thread_pool.clone();
            thread::Builder::new()
                .name(format!("{}::client_listener_thread", module_path!()))
                .spawn(move || {
                    client::listen_clients(&client_listener, &config, &server_tx, &thread_pool);
                })
                .expect("Cannot spawn client listener thread")
        };

        // The main thread enters a loop, reacting to events from other threads
        server.run_event_loop(&server_rx);

        peer_listener_thread.join_and_handle_panic();
        client_listener_thread.join_and_handle_panic();
        for peer in server.peers.into_values() {
            drop(peer);
        }
    }

    fn run_event_loop(&mut self, server_rx: &Receiver<Event>) {
        loop {
            // Render pretty text representation if running in a terminal
            console::update_state(format!("{self}"));

            // Periodically check if server needs to connect to peers
            match self.last_connect_attempt {
                ConnectAttempt::Never => self.attempt_connect(),
                ConnectAttempt::Finished(at) => {
                    if at.elapsed().unwrap_or(Duration::ZERO) > self.config.heartbeat_timeout {
                        self.attempt_connect();
                    }
                }
                ConnectAttempt::Running => {}
            };

            // Receive the next message from other threads (peer I/O, listeners, timers etc.)
            let message = match server_rx.recv_timeout(self.config.heartbeat_timeout) {
                Ok(message) => message,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            };

            match message {
                Event::IncomingPeerConnection(tcp_stream) => {
                    self.answer_handshake(tcp_stream);
                }
                Event::EstablishedPeerConnection(JoinSuccess {
                    peer_id,
                    peer,
                    leases,
                }) => {
                    if let Ok(()) = self.add_peer(peer_id, peer) {
                        if self.dhcp_service.leases.len() < leases.len() {
                            console::debug!("Updating leases");
                            self.dhcp_service.leases = leases;
                        }
                        // Need to initiate election, cluster changed
                        self.start_election();
                    }
                }
                Event::PeerLost(peer_id) => self.remove_peer(peer_id),
                Event::ElectionTimeout => self.finish_election(),
                Event::PeerMessage { sender_id, message } => {
                    self.handle_peer_message(sender_id, message);
                }
                Event::LeaseRequest { mac_address, tx } => {
                    self.handle_lease_request(mac_address, &tx);
                }
                Event::ConfirmRequest {
                    mac_address,
                    ip,
                    tx,
                } => self.handle_confirm_request(mac_address, ip, &tx),
            }
        }
    }

    fn attempt_connect(&mut self) {
        self.last_connect_attempt = ConnectAttempt::Running;
        console::debug!("Connection attempt started");

        for (peer_id, name) in &self.config.peers {
            // Only start connections with peers we don't have a connection with
            // Always have the higher ID start connections to avoid race conditions with concurrent
            // handshakes
            if !self.peers.contains_key(peer_id) && self.config.id > *peer_id {
                let config = Arc::clone(&self.config);
                let peer_id = *peer_id;
                let name = name.to_owned();
                let server_tx = self.tx.clone();
                self.thread_pool
                    .execute(
                        move || match Peer::connect(&config, peer_id, &name, &server_tx) {
                            Ok(success) => {
                                server_tx
                                    .send(Event::EstablishedPeerConnection(success))
                                    .expect("Invariant violated: server_rx has been dropped");
                            }
                            Err(e) => {
                                console::error!(&e, "Can't connect to peer {peer_id} at {name}");
                            }
                        },
                    )
                    .expect("Thread pool cannot spawn treads");
            }
        }

        self.last_connect_attempt = ConnectAttempt::Finished(SystemTime::now());
        console::debug!("Connection threads spawned");
    }

    fn handle_peer_message(&mut self, sender_id: peer::Id, message: Message) {
        match message {
            Message::Heartbeat => console::debug!("Received heartbeat from {sender_id}"),
            Message::Election => self.handle_election(sender_id, &message),
            Message::Okay => self.handle_okay(sender_id, &message),
            Message::Coordinator => self.handle_coordinator(sender_id),
            Message::Lease(lease) => self.handle_add_lease(lease),
            Message::SetPool(update) => self.handle_set_pool(update),
            Message::SetMajority(majority) => self.handle_majority(majority),
            Message::Join(..) | Message::JoinAck { .. } => {
                console::warning!("Server received unexpected {message:?} from peer {sender_id}");
            }
        };
    }

    fn handle_set_pool(&mut self, pool: Ipv4Range) {
        console::log!("Set pool to {}", pool);
        self.dhcp_service.set_pool(pool);
    }

    fn handle_election(&mut self, sender_id: peer::Id, message: &Message) {
        console::log!("Peer {sender_id} invited {message:?}");
        if sender_id < self.config.id {
            console::log!("Received Election from lower id");
            self.peers
                .get(&sender_id)
                .expect("Invariant violated: server.peers must contain election message sender")
                .send_message(Message::Okay);
            self.start_election();
        }
    }

    fn handle_okay(&mut self, sender_id: peer::Id, message: &Message) {
        assert!(sender_id > self.config.id);
        console::log!("Peer {sender_id}: {message:?}, ");
        if self.election_state == ElectionState::WaitingForElection {
            console::log!("Stepping down to Follower");
            self.election_state = ElectionState::Follower;
        } else {
            console::log!("Already stepped down");
        }
    }

    fn handle_coordinator(&mut self, sender_id: peer::Id) {
        console::log!("Recognizing {sender_id} as Coordinator");
        self.coordinator_id = Some(sender_id);
        self.election_state = ElectionState::Follower;
        if sender_id < self.config.id {
            self.start_election();
        }
    }

    fn handle_majority(&mut self, majority: bool) {
        if self.majority != majority {
            console::log!("{} majority", if majority { "Reached" } else { "Lost" });
            self.majority = majority;
        }
    }

    fn handle_add_lease(&mut self, lease: Lease) {
        self.dhcp_service.add_lease(lease);
    }

    fn handle_lease_request(&mut self, mac_address: MacAddr, tx: &Sender<LeaseOffer>) {
        if !self.majority {
            return;
        }

        if let Some(lease) = self.dhcp_service.discover_lease(mac_address) {
            if let Err(e) = tx.send(LeaseOffer {
                lease,
                subnet_mask: self.config.prefix_length,
            }) {
                console::error!(
                    &e,
                    "Could not reply to client worker which requested a lease"
                );
            }
        }
    }

    fn handle_confirm_request(
        &mut self,
        mac_address: MacAddr,
        ip: Ipv4Addr,
        tx: &Sender<LeaseConfirmation>,
    ) {
        if !self.majority {
            return;
        }

        match self.dhcp_service.commit_lease(mac_address, ip) {
            Ok(lease) => {
                if let Err(e) = tx.send(true) {
                    console::error!(
                        &e,
                        "Could not send lease confirmation to the worker which requested it"
                    );
                }
                for peer in self.peers.values() {
                    peer.send_message(Message::Lease(lease.clone()));
                }
            }
            Err(e) => {
                console::error!(&e, "Can't confirm lease");
                if let Err(e) = tx.send(false) {
                    console::error!(
                        &e,
                        "Could not send lease denial to the worker which requested it"
                    );
                }
            }
        };
    }

    fn answer_handshake(&mut self, mut stream: TcpStream) {
        // TODO: security: we should have a mechanism to authenticate peer, perhaps TLS
        stream
            .set_read_timeout(Some(self.config.heartbeat_timeout * 3))
            .expect("Can't set stream read timeout");
        let message = match stream.recv() {
            Ok(message) => message,
            Err(e) => {
                console::error!(&e, "Could not receive peer handshake message");
                return;
            }
        };

        if let Message::Join(peer_id) = message {
            match stream.send(&Message::JoinAck {
                peer_id: self.config.id,
                leases: self.dhcp_service.leases.clone(),
            }) {
                Ok(()) => {
                    console::log!("Peer {peer_id} joined");

                    self.add_peer(
                            peer_id,
                            Peer::new(
                                stream,
                                peer_id,
                                self.tx.clone(),
                                self.config.heartbeat_timeout,
                            ),
                        ).expect("Invariant violated: Server::add_peer() failed when we shouldn't have a stored connection");
                }
                Err(e) => console::error!(&e, "Answering handshake failed"),
            }
        } else {
            console::error!(
                &HandshakeError::NoJoin(message),
                "Answering handshake failed"
            );
        }

        // New peer joined, we want to inform it of the coordinator and reallocate the DHCP pool
        if self.election_state == ElectionState::Coordinator {
            self.become_coordinator();
        }
    }

    fn add_peer(&mut self, peer_id: peer::Id, peer: Peer) -> Result<(), ()> {
        // This should prevent us from having simultaneous connections open
        if self.peers.contains_key(&peer_id) {
            console::debug!("Tried to add peer {peer_id}, but already had a connection");
            drop(peer);
            console::debug!("peer.join() returned, stream should be closed now");
            return Err(());
        }
        if let Some(peer) = self.peers.insert(peer_id, peer) {
            console::debug!("Already had {peer_id}, dropping previous connection");
            drop(peer);
            console::debug!("peer.join() returned, stream should be closed now");
        }
        console::debug!("Added peer {peer_id}");
        Ok(())
    }

    fn remove_peer(&mut self, peer_id: peer::Id) {
        self.peers.remove(&peer_id);

        // Peer left, we want to confirm the coordinator and reallocate the DHCP pool
        if self.election_state == ElectionState::Coordinator {
            self.become_coordinator();
        }

        if Some(peer_id) == self.coordinator_id {
            self.start_election();
        }
    }

    /// Send [`Message::Election`] to all server peers with higher ids than this server.
    ///
    /// This function starts a timer thread which sleeps until the bully algorithm timeout expires.
    /// The timer triggers a [`ServerThreadMessage::ElectionTimeout`] to the the server thread.
    fn start_election(&mut self) {
        self.election_state = ElectionState::WaitingForElection;

        for (peer_id, peer) in &self.peers {
            if *peer_id > self.config.id {
                peer.send_message(Message::Election);
            }
        }

        let dur = self.config.heartbeat_timeout;
        let server_tx = self.tx.clone();
        thread::Builder::new()
            .name(format!("{}::election_timer_thread", module_path!()))
            .spawn(move || {
                // Wait for peers to vote.
                console::log!("Starting election wait");
                thread::sleep(dur);
                console::log!("Election wait over");
                if let Err(e) = server_tx.send(Event::ElectionTimeout) {
                    console::warning!("Election timer thread can't notify the server");
                    console::error!(&e);
                }
            })
            .expect("Cannot spawn election timer thread"); // Drop JoinHandle, detaching election thread from server thread
    }

    /// Inspect current server role. Become leader and send [`Message::Coordinator`] to all peers
    /// if no event has reset the server role back to [`ServerRole::Follower`].
    fn finish_election(&mut self) {
        match self.election_state {
            ElectionState::WaitingForElection => {
                self.become_coordinator();
            }
            ElectionState::Coordinator => {
                console::log!("Already Coordinator when election ended");
            }
            ElectionState::Follower => {
                console::log!("Received OK during election");
            }
        }
    }

    fn become_coordinator(&mut self) {
        self.election_state = ElectionState::Coordinator;
        self.coordinator_id = Some(self.config.id);
        let majority = self.peers.len() + 1 > (self.config.peers.len() + 1) / 2;
        self.handle_majority(majority);

        for peer in self.peers.values() {
            peer.send_message(Message::Coordinator);
            peer.send_message(Message::SetMajority(self.majority));
        }

        #[allow(
            clippy::unwrap_used,
            reason = "Having more DHCP nodes than IPv4 addresses is nonsense"
        )]
        let pools = self
            .config
            .dhcp_pool
            // +1 to account for the coordinator
            .divide(u32::try_from(self.peers.len()).unwrap() + 1);

        let mut pools_iter = pools.into_iter();

        // Set own pool
        self.handle_set_pool(pools_iter.next().expect("Pools should always exist"));

        for (pool, peer) in pools_iter.zip(self.peers.values()) {
            peer.send_message(Message::SetPool(pool));
        }
    }
}

impl Display for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let write_label = |f: &mut std::fmt::Formatter<'_>, label| write!(f, "    {label:<16} ");

        let title = format!(
            "Server {} listening to peers on {}",
            self.config.id, self.config.address_private
        );
        let mut hline = title.chars().map(|_| '-').collect::<String>();
        hline = format!("\x1B[90m{hline}\x1B[0m");
        writeln!(f, "{hline}\n{title}\n")?;

        // Coordinator
        write_label(f, "Coordinator id")?;
        if let Some(coordinator) = self.coordinator_id {
            writeln!(f, "{coordinator}",)?;
        } else {
            writeln!(f, "Unknown",)?;
        }

        // Peers
        write_label(f, "Active peers")?;
        write!(f, "[ ")?;

        let mut ids = self.peers.keys().copied().collect::<Vec<u32>>();
        ids.push(self.config.id);
        ids.sort_unstable();
        for (i, id) in ids.iter().enumerate() {
            if *id == self.config.id {
                write!(f, "\x1B[1m{id}\x1B[0m")?;
            } else {
                write!(f, "{id}")?;
            }

            if i != ids.len() - 1 {
                write!(f, ", ")?;
            }
        }
        writeln!(f, " ]")?;

        // Role
        write_label(f, "Current role")?;
        writeln!(f, "{:?}", self.election_state)?;

        // Majority and dhcp address
        write_label(f, "Service")?;
        if self.majority {
            writeln!(f, "{}", self.config.dhcp_address)?;
        } else {
            writeln!(f, "No majority")?;
        }

        // Pool assignment
        write_label(f, "Assigned range")?;
        writeln!(f, "{}", self.dhcp_service)?;

        writeln!(f, "{hline}")
    }
}
