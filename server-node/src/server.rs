use crate::{
    config::Config,
    console,
    dhcp::{DhcpService, Lease},
    message::Message,
    peer::{HandshakeError, JoinSuccess, Peer, PeerId},
    ThreadJoin,
};
use protocol::{
    CborRecvError, DhcpClientMessage, DhcpOffer, DhcpServerMessage, MacAddr, RecvCbor, RecvError,
    SendCbor,
};
use std::{
    any::Any,
    collections::HashMap,
    fmt::Display,
    io::{self, ErrorKind},
    net::{Ipv4Addr, TcpListener, TcpStream, ToSocketAddrs},
    sync::{
        mpsc::{channel, Sender},
        Arc,
    },
    thread,
    time::{Duration, SystemTime},
};
use thiserror::Error;
use thread_pool::ThreadPool;

#[derive(Debug)]
pub struct LeaseOffer {
    lease: Lease,
    subnet_mask: u32,
}

type LeaseConfirmation = bool;

#[derive(Debug)]
pub enum ServerThreadMessage {
    IncomingPeerConnection(TcpStream),
    EstablishedPeerConnection(JoinSuccess),
    ConnectAttemptFinished,
    PeerLost(PeerId),
    ElectionTimeout,
    ProtocolMessage {
        sender_id: PeerId,
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
    tx: Sender<ServerThreadMessage>,
    thread_pool: Arc<ThreadPool>,
    coordinator_id: Option<PeerId>,
    dhcp_pool: DhcpService,
    peers: HashMap<PeerId, Peer>,
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
        let (tx, server_rx) = channel::<ServerThreadMessage>();
        let dhcp_pool = config.dhcp_pool.clone();
        let thread_count = config.thread_count;
        console::log!("Starting with {} workers", thread_count);
        let mut server = Self {
            config: Arc::new(config),
            tx,
            thread_pool: Arc::new(
                ThreadPool::new(
                    thread_count,
                    Box::new(|worker_id: usize, msg: Box<dyn Any>| {
                        console::warning!("Worker {worker_id} panicked");
                        // Try both &str and String, I don't know which type panic messages will occur in
                        if let Some(msg) = msg
                            .downcast_ref::<&str>()
                            .map(|s| s.to_string())
                            .or(msg.downcast_ref::<String>().cloned())
                        {
                            console::warning!("{}", msg);
                        }
                    }),
                )
                .map_err(Error::Spawn)?,
            ),
            coordinator_id: None,
            dhcp_pool,
            peers: HashMap::new(),
            local_role: ServerRole::Follower,
            majority: false,
            last_connect_attempt: ConnectAttempt::Never,
        };

        let peer_listener_thread = {
            let server_tx = server.tx.clone();
            thread::Builder::new()
                .name(format!("{}::peer_listener_thread", module_path!()))
                .spawn(move || Server::listen_nodes(peer_listener, server_tx))
                .map_err(Error::Spawn)?
        };

        let client_listener_thread = {
            let server_tx = server.tx.clone();
            let thread_pool = Arc::clone(&server.thread_pool);
            thread::Builder::new()
                .name(format!("{}::client_listener_thread", module_path!()))
                .spawn(move || Self::listen_clients(client_listener, server_tx, thread_pool))
                .map_err(Error::Spawn)?
        };

        use ServerThreadMessage::*;
        loop {
            // Render pretty text representation if running in a terminal
            console::update_state(format!("{server}"));

            // Receive the next message from other threads (peer I/O, listeners, timers etc.)
            match server_rx.recv_timeout(server.config.heartbeat_timeout) {
                Ok(IncomingPeerConnection(tcp_stream)) => server.answer_handshake(tcp_stream),
                Ok(EstablishedPeerConnection(JoinSuccess {
                    peer_id,
                    peer,
                    leases,
                })) => {
                    if let Ok(()) = server.add_peer(peer_id, peer) {
                        if server.dhcp_pool.leases.len() < leases.len() {
                            console::debug!("Updating leases");
                            server.dhcp_pool.leases = leases;
                        }
                        // Need to initiate election, cluster changed
                        server.start_election();
                    }
                }
                Ok(ConnectAttemptFinished) => {
                    server.last_connect_attempt = ConnectAttempt::Finished(SystemTime::now());
                    console::debug!("Connection attempt finished");
                }
                Ok(PeerLost(peer_id)) => server.remove_peer(peer_id),
                Ok(ElectionTimeout) => server.finish_election(),
                Ok(ProtocolMessage { sender_id, message }) => {
                    server.handle_protocol_message(sender_id, message)
                }
                Ok(LeaseRequest { mac_address, tx }) => server.handle_offer_lease(mac_address, tx),
                Ok(ConfirmRequest {
                    mac_address,
                    ip,
                    tx,
                }) => server.handle_confirm_lease(mac_address, ip, tx),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => { /* ignore */ }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    break;
                }
            };

            // Periodically check if server needs to connect to peers
            match server.last_connect_attempt {
                ConnectAttempt::Never => server.attempt_connect(),
                ConnectAttempt::Finished(at) => {
                    if at.elapsed().unwrap_or(Duration::ZERO) > server.config.heartbeat_timeout * 3
                    {
                        server.attempt_connect()
                    }
                }
                _ => {}
            };
        }

        peer_listener_thread.join_and_handle_panic();
        client_listener_thread.join_and_handle_panic();
        for peer in server.peers.into_values() {
            peer.disconnect();
        }
        Ok(())
    }

    fn connect_peers(config: Arc<Config>, server_tx: Sender<ServerThreadMessage>) {
        let timeout = config.peer_connection_timeout;
        for name in config.peers.iter() {
            console::debug!("Connecting to {name}");
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
                                    Arc::clone(&config),
                                    server_tx.clone(),
                                ) {
                                    Ok(success) => server_tx
                                        .send(ServerThreadMessage::EstablishedPeerConnection(
                                            success,
                                        ))
                                        .expect("Invariant violated: server_rx has been dropped before connect_peers has finished"),
                                    Err(
                                        e @ (HandshakeError::Recv(_) | HandshakeError::SendJoin(_)),
                                    ) => {
                                        // Expected errors, just use debug log
                                        console::debug!("Handshake failed: {e}")
                                    }
                                    Err(e) => {
                                        console::error!(&e, "Unexpected hanshake error with {name}")
                                    }
                                };
                            }
                            Err(e) => console::error!(&e, "Can't connect to peer {name}"),
                        }
                    }
                }
                Err(e) => console::error!(&e, "Name resolution failed for {name}"),
            }
        }
        server_tx
            .send(ServerThreadMessage::ConnectAttemptFinished)
            .expect(
                "Invariant violated: server_rx has been dropped before connect_peers has finished",
            );
    }

    fn attempt_connect(&mut self) {
        self.last_connect_attempt = ConnectAttempt::Running;
        console::debug!("Connection attempt started");

        // For now this is the only check stopping us from spamming
        // connection attempts to peers with whom we're already connected.
        // TODO: If we want to KNOW which peers exactly are disconnected,
        // we need to encode peer IDs along with the names/addresses in Config,
        // and modify the Server::connect_peers function to reason about
        // which peers are offline and which aren't.
        if self.peers.len() == self.config.peers.len() {
            self.tx
                .send(ServerThreadMessage::ConnectAttemptFinished)
                .expect(
                    "Invariant violated: server_rx has been dropped after calling attempt_connect",
                );
            return;
        }

        let server_tx = self.tx.clone();
        let config = Arc::clone(&self.config);
        self.thread_pool
            .execute(move || Self::connect_peers(config, server_tx))
            .unwrap();
    }

    fn serve_client(stream: TcpStream, server_tx: Sender<ServerThreadMessage>) {
        let result = DhcpClientMessage::recv(&stream);
        match result {
            Ok(DhcpClientMessage::Discover { mac_address }) => {
                Self::handle_discover(stream, server_tx, mac_address)
            }
            Ok(DhcpClientMessage::Request { mac_address, ip }) => {
                Self::handle_renew(stream, server_tx, mac_address, ip)
            }
            Err(e) => console::error!(&e, "Could not receive request from the client"),
        }
    }

    fn handle_discover(
        stream: TcpStream,
        server_tx: Sender<ServerThreadMessage>,
        mac_address: MacAddr,
    ) {
        let (tx, rx) = channel::<LeaseOffer>();
        server_tx
            .send(ServerThreadMessage::LeaseRequest { mac_address, tx })
            .expect("Invariant violated: server_rx has been dropped before joining client listener thread");

        // Wait for processing DHCP discover
        let LeaseOffer { lease, subnet_mask } = match rx.recv() {
            Ok(offer) => offer,
            Err(_) => {
                if let Err(e) = DhcpServerMessage::send(&stream, &DhcpServerMessage::Nack) {
                    console::error!(&e, "Could not reply with Nack to the client");
                }
                return;
            }
        };

        let ip = lease.lease_address;
        let lease_time = lease
            .expiry_timestamp
            .duration_since(SystemTime::now())
            .unwrap_or(Duration::ZERO)
            .as_secs() as u32;

        if let Err(e) = DhcpServerMessage::send(
            &stream,
            &DhcpServerMessage::Offer(DhcpOffer {
                ip,
                lease_time,
                subnet_mask,
            }),
        ) {
            console::error!(&e, "Could not send offer to the client");
            return;
        }

        let timeout = Duration::from_secs(10);
        match DhcpClientMessage::recv_timeout(&stream, timeout) {
            Ok(DhcpClientMessage::Request { mac_address, ip }) => {
                let (tx, rx) = channel::<LeaseConfirmation>();
                server_tx
                    .send(ServerThreadMessage::ConfirmRequest {
                        mac_address,
                        ip,
                        tx,
                    })
                    .expect("Invariant violated: server_rx has been dropped before joining client listener thread");
                // Wait for processing DHCP commit
                match rx.recv_timeout(timeout) {
                    Ok(committed) => {
                        if committed {
                            if let Err(e) =
                                DhcpServerMessage::send(&stream, &DhcpServerMessage::Ack)
                            {
                                console::error!(&e, "Could not send Ack to the client");
                            }
                        } else if let Err(e) =
                            DhcpServerMessage::send(&stream, &DhcpServerMessage::Nack)
                        {
                            console::error!(&e, "Could not send Nack to the client");
                        }
                    }
                    Err(_) => todo!(),
                }
            }
            Ok(message) => console::warning!(
                "Client didn't follow protocol!\nExpected: Request, got: {message:?}"
            ),
            Err(ref e) => match e {
                CborRecvError::Receive(RecvError::Io(io_error)) => match io_error.kind() {
                    ErrorKind::WouldBlock | ErrorKind::TimedOut => {
                        console::error!(
                            e,
                            "Client didn't follow Discover with Request within {timeout:?}"
                        )
                    }
                    _ => console::error!(e, "Could not receive client reply"),
                },
                _ => console::error!(e, "Could not receive client reply"),
            },
        }
    }

    fn handle_renew(
        stream: TcpStream,
        server_tx: Sender<ServerThreadMessage>,
        mac_address: MacAddr,
        ip: Ipv4Addr,
    ) {
        let (tx, rx) = channel::<LeaseConfirmation>();
        server_tx
            .send(ServerThreadMessage::ConfirmRequest {
                mac_address,
                ip,
                tx,
            })
            .expect("Invariant violated: server_rx has been dropped before joining client listener thread");
        if rx.recv().unwrap_or(false) {
            if let Err(e) = DhcpServerMessage::send(&stream, &DhcpServerMessage::Ack) {
                console::error!(&e, "Could not send Ack to the client");
            }
        } else if let Err(e) = DhcpServerMessage::send(&stream, &DhcpServerMessage::Nack) {
            console::error!(&e, "Could not send Nack to the client");
        }
    }

    fn handle_protocol_message(&mut self, sender_id: PeerId, message: Message) {
        use Message::*;
        match message {
            Heartbeat => console::debug!("Received heartbeat from {sender_id}"),
            Election => self.handle_election(sender_id, &message),
            Okay => self.handle_okay(sender_id, &message),
            Coordinator => self.handle_coordinator(sender_id),
            Lease(lease) => self.handle_add_lease(lease),
            SetPool(dhcp_pool) => self.handle_set_pool(dhcp_pool),
            SetMajority(majority) => self.handle_majority(majority),
            _ => panic!("Server received unexpected {message:?} from {sender_id}"),
        };
    }

    fn handle_set_pool(&mut self, dhcp_pool: DhcpService) {
        console::log!("Set pool to {}", dhcp_pool.range);
        self.dhcp_pool = dhcp_pool;
    }

    fn handle_election(&mut self, sender_id: PeerId, message: &Message) {
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

    fn handle_majority(&mut self, majority: bool) {
        if self.majority != majority {
            console::log!("{} majority", if majority { "Reached" } else { "Lost" });
            self.majority = majority;
        }
    }

    fn handle_add_lease(&mut self, lease: Lease) {
        self.dhcp_pool.add_lease(lease);
    }

    fn handle_offer_lease(&mut self, mac_address: MacAddr, tx: Sender<LeaseOffer>) {
        if !self.majority {
            return;
        }

        let lease = match self.dhcp_pool.discover_lease(mac_address) {
            Some(lease) => lease,
            None => return,
        };
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

    fn handle_confirm_lease(
        &mut self,
        mac_address: MacAddr,
        ip: Ipv4Addr,
        tx: Sender<LeaseConfirmation>,
    ) {
        if !self.majority {
            return;
        }

        match self.dhcp_pool.commit_lease(mac_address, ip) {
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

    fn listen_nodes(listener: TcpListener, server_tx: Sender<ServerThreadMessage>) {
        for stream in listener.incoming() {
            match stream {
                // TODO: Here we may need a mechanism to end this thread, unless we just decide to detach it?
                Ok(stream) => server_tx
                    .send(ServerThreadMessage::IncomingPeerConnection(stream))
                    .expect("Invariant violated: server_rx has been dropped before joining peer listener thread"),
                Err(e) => console::error!(&e, "Accepting new peer connection failed"),
            }
        }
    }

    fn listen_clients(
        listener: TcpListener,
        server_tx: Sender<ServerThreadMessage>,
        thread_pool: Arc<ThreadPool>,
    ) {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let tx = server_tx.clone();
                    thread_pool
                        .execute(move || Self::serve_client(stream, tx))
                        .unwrap();
                }
                Err(e) => console::error!(&e, "Accepting new client connection failed"),
            }
        }
    }

    fn answer_handshake(&mut self, stream: TcpStream) {
        // TODO: security: we should have a mechanism to authenticate peer, perhaps TLS
        let message = match Message::recv_timeout(&stream, self.config.heartbeat_timeout) {
            Ok(message) => message,
            Err(e) => {
                console::error!(&e, "Could not receive client handshake message");
                return;
            }
        };

        match message {
            Message::Join(peer_id) => {
                if self.peers.contains_key(&peer_id) {
                    // This reconnection is redundant, just close the stream early
                    console::debug!("Already have {peer_id}, closing connection");
                    return;
                } else {
                    let result = Message::send(
                        &stream,
                        &Message::JoinAck(self.config.id, self.dhcp_pool.leases.clone()),
                    );
                    match result {
                        Ok(_) => {
                            console::log!("Peer {peer_id} joined");

                            self.add_peer(
                                peer_id,
                                Peer::new(
                                    stream,
                                    peer_id,
                                    self.tx.clone(),
                                    self.config.heartbeat_timeout,
                                ),
                            ).expect("Invariant violated: Server::add_peer() failed when we don't have a stored connection");
                        }
                        Err(e) => console::error!(&e, "Answering handshake failed"),
                    }
                }
            }
            _ => console::error!(
                &HandshakeError::NoJoin(message),
                "Answering handshake failed"
            ),
        }

        // New peer joined, we want to inform it of the coordinator and reallocate the DHCP pool
        if self.local_role == ServerRole::Coordinator {
            self.become_coordinator();
        }
    }

    fn add_peer(&mut self, peer_id: PeerId, peer: Peer) -> Result<(), ()> {
        // This should prevent us from having simultaneous connections open
        if self.peers.contains_key(&peer_id) {
            console::debug!("Tried to add peer {peer_id}, but already had a connection");
            peer.disconnect(); // TODO: This might leave the read thread hanging
            console::debug!("peer.join() returned, stream should be closed now");
            return Err(());
        }
        if let Some(peer) = self.peers.insert(peer_id, peer) {
            console::debug!("Already had {peer_id}, dropping previous connection");
            peer.disconnect(); // TODO: This might leave the read thread hanging
            console::debug!("peer.join() returned, stream should be closed now");
        }
        console::debug!("Added peer {peer_id}");
        Ok(())
    }

    fn remove_peer(&mut self, peer_id: PeerId) {
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
        use ServerRole::*;

        self.local_role = WaitingForElection;

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
                if let Err(e) = server_tx.send(ServerThreadMessage::ElectionTimeout) {
                    console::warning!("Election timer thread can't notify the server");
                    console::error!(&e);
                }
            })
            .unwrap(); // Drop JoinHandle, detaching election thread from server thread
    }

    /// Inspect current server role. Become leader and send [`Message::Coordinator`] to all peers
    /// if no event has reset the server role back to [`ServerRole::Follower`].
    fn finish_election(&mut self) {
        use ServerRole::*;
        match self.local_role {
            WaitingForElection => {
                self.become_coordinator();
            }
            Coordinator => {
                console::log!("Already Coordinator when election ended");
            }
            Follower => {
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

        let pools = self
            .config
            .dhcp_pool
            .divide(self.peers.len() as u32 + 1, &self.dhcp_pool.leases); // +1 to account for the coordinator
        let mut pools_iter = pools.iter();

        // Set own pool
        self.handle_set_pool(
            pools_iter
                .next()
                .expect("Pools should always exist")
                .clone(),
        );

        for (pool, peer) in pools_iter.zip(self.peers.values()) {
            peer.send_message(Message::SetPool(pool.clone()));
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

        let mut ids = self.peers.keys().cloned().collect::<Vec<u32>>();
        ids.push(self.config.id);
        ids.sort();
        for (i, id) in ids.iter().enumerate() {
            if *id != self.config.id {
                write!(f, "{id}")?;
            } else {
                write!(f, "\x1B[1m{id}\x1B[0m")?;
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
        writeln!(f, "{}", self.dhcp_pool)?;

        writeln!(f, "{hline}")
    }
}
