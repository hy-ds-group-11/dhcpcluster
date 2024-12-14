use crate::{
    config::Config,
    console,
    dhcp::{self, Ipv4Range, Lease, LeaseOffer},
    message::Message,
    peer::{self, HandshakeError, JoinSuccess, Peer},
    ThreadJoin,
};
use protocol::{
    CborRecvError, DhcpClientMessage, DhcpServerMessage, MacAddr, RecvCbor, RecvError, SendCbor,
};
use std::{
    any::Any,
    collections::HashMap,
    fmt::Display,
    io::{self, ErrorKind},
    net::{Ipv4Addr, TcpListener, TcpStream, ToSocketAddrs},
    sync::{
        mpsc::{self, Sender},
        Arc,
    },
    thread,
    time::{Duration, SystemTime},
};
use thiserror::Error;
use thread_pool::ThreadPool;

type LeaseConfirmation = bool;

#[derive(Debug)]
pub enum MainThreadMessage {
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
enum ServerRole {
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

#[derive(Error, Debug)]
pub enum Error {
    #[error("Failed to spawn threads")]
    Spawn(#[source] io::Error),
}

/// The distributed DHCP server
pub struct Server {
    config: Arc<Config>,
    tx: Sender<MainThreadMessage>,
    thread_pool: ThreadPool,
    coordinator_id: Option<peer::Id>,
    dhcp_service: dhcp::Service,
    peers: HashMap<peer::Id, Peer>,
    local_role: ServerRole,
    majority: bool,
    last_connect_attempt: ConnectAttempt,
}

impl Server {
    /// Start listening to incoming connections from server peers and DHCP clients.
    pub fn start(
        config: Config,
        peer_listener: TcpListener,
        client_listener: TcpListener,
    ) -> Result<(), Error> {
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
            .map_err(Error::Spawn)?,
            coordinator_id: None,
            dhcp_service,
            peers: HashMap::new(),
            local_role: ServerRole::Follower,
            majority: false,
            last_connect_attempt: ConnectAttempt::Never,
        };

        let peer_listener_thread = {
            let server_tx = server.tx.clone();
            thread::Builder::new()
                .name(format!("{}::peer_listener_thread", module_path!()))
                .spawn(move || Server::listen_nodes(&peer_listener, &server_tx))
                .map_err(Error::Spawn)?
        };

        let client_listener_thread = {
            let config = Arc::clone(&server.config);
            let server_tx = server.tx.clone();
            let thread_pool = server.thread_pool.clone();
            thread::Builder::new()
                .name(format!("{}::client_listener_thread", module_path!()))
                .spawn(move || {
                    Self::listen_clients(&client_listener, &config, &server_tx, &thread_pool);
                })
                .map_err(Error::Spawn)?
        };

        loop {
            // Render pretty text representation if running in a terminal
            console::update_state(format!("{server}"));

            // Periodically check if server needs to connect to peers
            match server.last_connect_attempt {
                ConnectAttempt::Never => server.attempt_connect(),
                ConnectAttempt::Finished(at) => {
                    if at.elapsed().unwrap_or(Duration::ZERO) > server.config.heartbeat_timeout {
                        server.attempt_connect();
                    }
                }
                ConnectAttempt::Running => {}
            };

            // Receive the next message from other threads (peer I/O, listeners, timers etc.)
            let message = match server_rx.recv_timeout(server.config.heartbeat_timeout) {
                Ok(message) => message,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            };

            match message {
                MainThreadMessage::IncomingPeerConnection(tcp_stream) => {
                    server.answer_handshake(tcp_stream);
                }
                MainThreadMessage::EstablishedPeerConnection(JoinSuccess {
                    peer_id,
                    peer,
                    leases,
                }) => {
                    if let Ok(()) = server.add_peer(peer_id, peer) {
                        if server.dhcp_service.leases.len() < leases.len() {
                            console::debug!("Updating leases");
                            server.dhcp_service.leases = leases;
                        }
                        // Need to initiate election, cluster changed
                        server.start_election();
                    }
                }
                MainThreadMessage::PeerLost(peer_id) => server.remove_peer(peer_id),
                MainThreadMessage::ElectionTimeout => server.finish_election(),
                MainThreadMessage::PeerMessage { sender_id, message } => {
                    server.handle_peer_message(sender_id, message);
                }
                MainThreadMessage::LeaseRequest { mac_address, tx } => {
                    server.handle_lease_request(mac_address, &tx);
                }
                MainThreadMessage::ConfirmRequest {
                    mac_address,
                    ip,
                    tx,
                } => server.handle_confirm_request(mac_address, ip, &tx),
            }
        }

        peer_listener_thread.join_and_handle_panic();
        client_listener_thread.join_and_handle_panic();
        for peer in server.peers.into_values() {
            drop(peer);
        }
        Ok(())
    }

    fn connect_peer(
        config: &Arc<Config>,
        server_tx: &Sender<MainThreadMessage>,
        peer_index: usize,
    ) {
        let (peer_id, name) = &config.peers[peer_index];
        let timeout = config.peer_connection_timeout;
        console::debug!("Connecting to {peer_id} at {name}");
        match name.to_socket_addrs() {
            Ok(addrs) => {
                for addr in addrs {
                    let result = if let Some(timeout) = timeout {
                        TcpStream::connect_timeout(&addr, timeout)
                    } else {
                        TcpStream::connect(addr)
                    };

                    match result {
                        Ok(stream) => {
                            match Peer::start_handshake(
                                stream,
                                &Arc::clone(config),
                                server_tx.clone(),
                            ) {
                                Ok(success) => server_tx
                                    .send(MainThreadMessage::EstablishedPeerConnection(
                                        success,
                                    ))
                                    .expect("Invariant violated: server_rx has been dropped before connect_peers has finished"),
                                Err(
                                    e @ (HandshakeError::Recv(_) | HandshakeError::SendJoin(_)),
                                ) => {
                                    // Expected errors, just use debug log
                                    console::debug!("Handshake failed: {e}");
                                }
                                Err(e) => {
                                    console::error!(
                                        &e,
                                        "Unexpected hanshake error with {peer_id} at {name}"
                                    );
                                }
                            };
                        }
                        Err(e) => {
                            console::error!(&e, "Can't connect to peer {peer_id} at {name}");
                        }
                    }
                }
            }
            Err(e) => console::error!(&e, "Name resolution failed for {peer_id} at {name}"),
        }
    }

    fn attempt_connect(&mut self) {
        self.last_connect_attempt = ConnectAttempt::Running;
        console::debug!("Connection attempt started");

        // Using enumerate is a dirty hack which simplifies how much we need to pass to the thread
        // It already has access to an unchanging Config, so this avoids adding lifetimes etc.
        // TODO: Implement a better, more Rust-like solution
        for (index, (peer_id, _)) in self.config.peers.iter().enumerate() {
            // Only start connections with peers we don't have a connection with
            // Always have the higher ID start connections to avoid race conditions with concurrent
            // handshakes
            if !self.peers.contains_key(peer_id) && self.config.id > *peer_id {
                let server_tx = self.tx.clone();
                let config = Arc::clone(&self.config);
                self.thread_pool
                    .execute(move || Self::connect_peer(&config, &server_tx, index))
                    .expect("Thread pool cannot spawn treads");
            }
        }

        self.last_connect_attempt = ConnectAttempt::Finished(SystemTime::now());
        console::debug!("Connection threads spawned");
    }

    fn serve_client(
        stream: &mut TcpStream,
        config: &Arc<Config>,
        server_tx: &Sender<MainThreadMessage>,
    ) {
        stream
            .set_read_timeout(Some(config.client_timeout))
            .expect("Can't set stream read timeout");
        let result = stream.recv();
        match result {
            Ok(DhcpClientMessage::Discover { mac_address }) => {
                Self::handle_discover(stream, server_tx, mac_address);
            }
            Ok(DhcpClientMessage::Request { mac_address, ip }) => {
                Self::handle_renew(stream, server_tx, mac_address, ip);
            }
            Err(e) => console::error!(&e, "Could not receive request from the client"),
        }
    }

    fn handle_discover(
        stream: &mut TcpStream,
        server_tx: &Sender<MainThreadMessage>,
        mac_address: MacAddr,
    ) {
        #[cfg(debug_assertions)]
        assert!(matches!(stream.read_timeout(), Ok(Some(_))));

        let (tx, rx) = mpsc::channel();
        server_tx
            .send(MainThreadMessage::LeaseRequest { mac_address, tx })
            .expect("Invariant violated: server_rx has been dropped before joining client listener thread");

        // Wait for main thread to process DHCP discover
        let Ok(offer) = rx.recv() else {
            if let Err(e) = stream.send(&DhcpServerMessage::Nack) {
                console::error!(&e, "Could not reply with Nack to the client");
            }
            return;
        };

        // Send main thread's offer to the client
        if let Err(e) = stream.send(&DhcpServerMessage::Offer(offer.into())) {
            console::error!(&e, "Could not send offer to the client");
            return;
        }

        // Listen for client's confirmation. TODO: with real DHCP, this goes elsewhere
        match stream.recv() {
            Ok(DhcpClientMessage::Request { mac_address, ip }) => {
                let (tx, rx) = mpsc::channel();
                server_tx
                    .send(MainThreadMessage::ConfirmRequest {
                        mac_address,
                        ip,
                        tx,
                    })
                    .expect("Invariant violated: server_rx has been dropped before joining client listener thread");
                // Wait for processing DHCP commit
                match rx.recv() {
                    Ok(committed) => {
                        if committed {
                            if let Err(e) = stream.send(&DhcpServerMessage::Ack) {
                                console::error!(&e, "Could not send Ack to the client");
                            }
                        } else if let Err(e) = stream.send(&DhcpServerMessage::Nack) {
                            console::error!(&e, "Could not send Nack to the client");
                        }
                    }
                    Err(e) => {
                        console::error!(&e, "Could not commit lease");
                    }
                }
            }
            Ok(message) => console::warning!(
                "Client didn't follow protocol!\nExpected: Request, got: {message:?}"
            ),
            Err(ref error) => {
                if let CborRecvError::Receive(RecvError::Io(io_error)) = error {
                    match io_error.kind() {
                        ErrorKind::WouldBlock | ErrorKind::TimedOut => {
                            console::error!(
                                error,
                                "Client didn't follow Discover with Request within {:?}",
                                stream.read_timeout().ok().flatten().expect(
                                    "handle_discover called without read timeout set on stream"
                                ),
                            );
                        }
                        _ => console::error!(error, "Could not receive client reply"),
                    }
                } else {
                    console::error!(error, "Could not receive client reply");
                }
            }
        }
    }

    fn handle_renew(
        stream: &mut TcpStream,
        server_tx: &Sender<MainThreadMessage>,
        mac_address: MacAddr,
        ip: Ipv4Addr,
    ) {
        #[cfg(debug_assertions)]
        assert!(matches!(stream.read_timeout(), Ok(Some(_))));

        let (tx, rx) = mpsc::channel();
        server_tx
            .send(MainThreadMessage::ConfirmRequest {
                mac_address,
                ip,
                tx,
            })
            .expect("Invariant violated: server_rx has been dropped before joining client listener thread");
        if rx.recv().unwrap_or(false) {
            if let Err(e) = stream.send(&DhcpServerMessage::Ack) {
                console::error!(&e, "Could not send Ack to the client");
            }
        } else if let Err(e) = stream.send(&DhcpServerMessage::Nack) {
            console::error!(&e, "Could not send Nack to the client");
        }
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
            _ => panic!("Server received unexpected {message:?} from {sender_id}"),
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
        if self.local_role == ServerRole::WaitingForElection {
            console::log!("Stepping down to Follower");
            self.local_role = ServerRole::Follower;
        } else {
            console::log!("Already stepped down");
        }
    }

    fn handle_coordinator(&mut self, sender_id: peer::Id) {
        console::log!("Recognizing {sender_id} as Coordinator");
        self.coordinator_id = Some(sender_id);
        self.local_role = ServerRole::Follower;
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

    fn listen_nodes(listener: &TcpListener, server_tx: &Sender<MainThreadMessage>) {
        for stream in listener.incoming() {
            match stream {
                // TODO: Here we may need a mechanism to end this thread, unless we just decide to detach it?
                Ok(stream) => server_tx
                    .send(MainThreadMessage::IncomingPeerConnection(stream))
                    .expect("Invariant violated: server_rx has been dropped before joining peer listener thread"),
                Err(e) => console::error!(&e, "Accepting new peer connection failed"),
            }
        }
    }

    fn listen_clients(
        listener: &TcpListener,
        config: &Arc<Config>,
        server_tx: &Sender<MainThreadMessage>,
        thread_pool: &ThreadPool,
    ) {
        for stream in listener.incoming() {
            match stream {
                Ok(mut stream) => {
                    let config = Arc::clone(config);
                    let tx = server_tx.clone();
                    thread_pool
                        .execute(move || Self::serve_client(&mut stream, &config, &tx))
                        .expect("Thread pool cannot spawn threads");
                }
                Err(e) => console::error!(&e, "Accepting new client connection failed"),
            }
        }
    }

    fn answer_handshake(&mut self, mut stream: TcpStream) {
        // TODO: security: we should have a mechanism to authenticate peer, perhaps TLS
        stream
            .set_read_timeout(Some(self.config.heartbeat_timeout * 3))
            .expect("Can't set stream read timeout");
        let message = match stream.recv() {
            Ok(message) => message,
            Err(e) => {
                console::error!(&e, "Could not receive client handshake message");
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
        if self.local_role == ServerRole::Coordinator {
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
        if self.local_role == ServerRole::Coordinator {
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
        self.local_role = ServerRole::WaitingForElection;

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
                if let Err(e) = server_tx.send(MainThreadMessage::ElectionTimeout) {
                    console::warning!("Election timer thread can't notify the server");
                    console::error!(&e);
                }
            })
            .expect("Cannot spawn election timer thread"); // Drop JoinHandle, detaching election thread from server thread
    }

    /// Inspect current server role. Become leader and send [`Message::Coordinator`] to all peers
    /// if no event has reset the server role back to [`ServerRole::Follower`].
    fn finish_election(&mut self) {
        match self.local_role {
            ServerRole::WaitingForElection => {
                self.become_coordinator();
            }
            ServerRole::Coordinator => {
                console::log!("Already Coordinator when election ended");
            }
            ServerRole::Follower => {
                console::log!("Received OK during election");
            }
        }
    }

    fn become_coordinator(&mut self) {
        self.local_role = ServerRole::Coordinator;
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
        writeln!(f, "{:?}", self.local_role)?;

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
