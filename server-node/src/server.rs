use crate::{
    config::Config,
    console,
    dhcp::{DhcpPool, Lease},
    message::{self, Message},
    peer::{Peer, PeerId},
};
use std::{
    error::Error,
    fmt::Display,
    net::{TcpListener, TcpStream},
    sync::mpsc::{channel, Receiver, Sender},
    thread,
    time::SystemTime,
};

#[derive(Debug)]
pub enum ServerThreadMessage {
    NewConnection(TcpStream),
    PeerLost(PeerId),
    ElectionTimeout,
    ProtocolMessage { sender_id: PeerId, message: Message },
}

#[derive(Debug, PartialEq)]
enum ServerRole {
    WaitingForElection,
    Coordinator,
    Follower,
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
    dhcp_pool: DhcpPool,
    peers: Vec<Peer>,
    local_role: ServerRole,
    majority: bool,
}

impl Server {
    /// Initialize shared state, and try connecting to peers.
    /// Failed handshakes are ignored, since they might be nodes that are starting later.
    /// After connecting to peers, you probably want to call [`Server::start`].
    pub fn connect(config: Config) -> Self {
        let start_time = SystemTime::now();
        let coordinator_id = None;
        let leases = Vec::new();
        let dhcp_pool = config.dhcp_pool.clone();
        let mut peers = Vec::new();
        let local_role = ServerRole::Follower;

        let (tx, rx) = channel::<ServerThreadMessage>();

        for peer_address in &config.peers {
            match TcpStream::connect(peer_address) {
                Ok(stream) => {
                    let result = Peer::start_handshake(stream, &config, tx.clone());
                    match result {
                        Ok(peer) => peers.push(peer),
                        Err(e) => console::warning!("{e:?}"),
                    }
                }
                Err(e) => console::warning!("Connection refused to peer {peer_address}\n{e:?}"),
            }
        }

        Self {
            config,
            rx: Some(rx),
            tx,
            coordinator_id,
            leases,
            dhcp_pool,
            peers,
            local_role,
            start_time,
            majority: false,
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

        // Only node in cluster, become coordinator
        if self.peers.is_empty() {
            self.become_coordinator();
        }

        // Prevent program seemingly hanging when waiting for first message
        console::render(self.start_time, &format!("{self}"));

        let rx = self.rx.take().unwrap();
        use ServerThreadMessage::*;
        for message in rx.iter() {
            match message {
                NewConnection(tcp_stream) => self.answer_handshake(tcp_stream),
                PeerLost(peer_id) => self.remove_peer(peer_id),
                ElectionTimeout => self.finish_election(),
                ProtocolMessage { sender_id, message } => {
                    self.handle_protocol_message(sender_id, message)
                }
            }
            console::render(self.start_time, &format!("{self}"));
        }

        peer_listener_thread.join().map_err(|e| {
            format!(
                "Node listener thread panicked, Err: {:?}",
                e.downcast_ref::<&str>()
            )
            .into()
        })
    }

    fn handle_protocol_message(&mut self, sender_id: PeerId, message: Message) {
        use Message::*;
        #[allow(unused_variables)]
        match message {
            Heartbeat => console::debug!("Received heartbeat from {sender_id}"),
            Election => self.handle_election(sender_id, &message),
            Okay => self.handle_okay(sender_id, &message),
            Coordinator => self.handle_coordinator(sender_id),
            Add(lease) => todo!(),
            Update(lease) => todo!(),
            SetPool(dhcp_pool) => {
                self.dhcp_pool = dhcp_pool;
            }
            SetMajority(majority) => {
                if self.majority != majority {
                    console::log!("{} majority", if majority { "Reached" } else { "Lost" });
                    self.majority = majority;
                }
            }
            _ => panic!("Server received unexpected {message:?} from {sender_id}"),
        };
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

        // New peer joined, we want to inform it of the coordinator and reallocate the DHCP pool
        if self.local_role == ServerRole::Coordinator {
            self.become_coordinator();
        }
    }

    fn remove_peer(&mut self, peer_id: PeerId) {
        self.peers.retain(|peer| peer.id != peer_id);

        // Peer left, we want to confirm the coordinator and reallocate the DHCP pool
        if self.local_role == ServerRole::Coordinator {
            self.become_coordinator();
        }

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
        self.majority = self.peers.len() + 1 > (self.config.peers.len() + 1) / 2;
        console::log!(
            "{} majority",
            if self.majority { "Reached" } else { "Lost" }
        );

        for peer in &self.peers {
            peer.send_message(Message::Coordinator);
            peer.send_message(Message::SetMajority(self.majority));
        }

        let pools = self.config.dhcp_pool.divide(self.peers.len() as u32 + 1); // +1 to account for the coordinator
        let mut pools_iter = pools.iter();

        // Set own pool
        self.dhcp_pool = pools_iter
            .next()
            .expect("Pools should always exist")
            .clone();

        for (pool, peer) in pools_iter.zip(self.peers.iter()) {
            peer.send_message(Message::SetPool(pool.clone()));
        }
    }
}

impl Display for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let write_label = |f: &mut std::fmt::Formatter<'_>, label| write!(f, "    {label:<16} ");

        let title = format!(
            "Server {} listening on {}",
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

        let mut ids = self.peers.iter().map(|item| item.id).collect::<Vec<_>>();
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

        writeln!(f, "{hline}")
    }
}
