pub mod message;

use crate::{
    config::{self, Config},
    console,
    server::Event,
    ThreadJoin,
};
use message::Message;
use protocol::{RecvCbor, SendCbor};
use std::{
    fmt::Display,
    io,
    net::{TcpListener, TcpStream, ToSocketAddrs},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
        Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum HandshakeError {
    #[error("Failed to receive reply during handshake")]
    Recv(#[from] protocol::CborRecvError),
    #[error("Failed to send Join during handshake")]
    SendJoin(#[from] protocol::CborSendError),
    #[error("Peer responded with {0:?} when expecting JoinAck")]
    NoJoinAck(Message),
    #[error("Peer initiated with {0:?} when expecting Join")]
    NoJoin(Message),
    #[error("Can't establish stream to peer {peer_id}")]
    Connect { peer_id: Id, source: io::Error },
    #[error("Can't resolve peer {peer_id} address ({name})")]
    NameResolution {
        peer_id: Id,
        name: String,
        source: io::Error,
    },
    #[error("Peer {peer_id} name {name} resolved to 0 addresses, can't reach peer")]
    Unreachable { peer_id: Id, name: String },
    #[error(
        "JoinAck mismatch! Expected id {expected}, received {received}.\nFix your configuration."
    )]
    IdMismatch { expected: Id, received: Id },
}

#[derive(Debug)]
pub struct JoinSuccess {
    pub peer_id: Id,
    pub peer: Peer,
}

pub type Id = u32;

/// Peer connection
#[derive(Debug)]
pub struct Peer {
    id: Id,
    tx: Option<Sender<Message>>,
    terminated: Arc<AtomicBool>,
    read_thread: Option<JoinHandle<()>>,
    write_thread: Option<JoinHandle<()>>,
}

impl Peer {
    /// Blocking incoming peer connection accept loop
    pub fn listen(listener: &TcpListener, server_tx: &Sender<Event>) {
        // TODO: Here we may need a mechanism to end this loop
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => server_tx
                    .send(Event::IncomingPeerConnection(stream))
                    .expect("Invariant violated: server_rx has been dropped before joining peer listener thread"),
                Err(e) => console::error!(&e, "Accepting new peer connection failed"),
            }
        }
    }

    /// Initiate peer connection
    pub fn connect(
        config: &Arc<Config>,
        peer: &config::Peer,
        server_tx: &Sender<Event>,
    ) -> Result<JoinSuccess, HandshakeError> {
        let timeout = config.connect_timeout;
        let peer_id = peer.id;
        let name = &peer.host;

        console::debug!("Connecting to {peer_id} at {name}");

        // Resolve address
        let mut addrs = peer
            .host
            .to_socket_addrs()
            .map_err(|e| HandshakeError::NameResolution {
                peer_id,
                name: name.clone(),
                source: e,
            })?
            .peekable();

        // Try every result
        while let Some(addr) = addrs.next() {
            let stream = match TcpStream::connect_timeout(&addr, timeout) {
                Ok(stream) => stream,
                Err(e) => match addrs.peek() {
                    Some(_) => continue, // Retry next result if available
                    None => return Err(HandshakeError::Connect { peer_id, source: e }),
                },
            };

            return Self::start_handshake(stream, &Arc::clone(config), peer_id, server_tx.clone());
        }
        Err(HandshakeError::Unreachable {
            peer_id,
            name: name.to_owned(),
        })
    }

    /// Start peer IO threads
    #[must_use]
    pub fn new(
        mut stream: TcpStream,
        id: Id,
        server_tx: Sender<Event>,
        heartbeat_timeout: Duration,
    ) -> Self {
        console::log!("Started connection to peer with id {id}");
        let (tx, rx) = mpsc::channel();
        let terminated = Arc::new(AtomicBool::new(false));

        let mut stream_read = stream.try_clone().expect("Cannot clone TcpStream!");
        // Set read to timeout if three heartbeats are missed. Three was chosen arbitrarily
        stream_read
            .set_read_timeout(Some(3 * heartbeat_timeout))
            .expect("set_read_timeout call failed");

        let read_thread = {
            let terminated = Arc::clone(&terminated);
            thread::Builder::new()
                .name(format!("{}::read_thread({})", module_path!(), id))
                .spawn(move || Self::read_thread_fn(&mut stream_read, id, &server_tx, &terminated))
                .expect("Cannot spawn peer read thread")
        };

        let write_thread = thread::Builder::new()
            .name(format!("{}::write_thread({})", module_path!(), id))
            .spawn(move || Self::write_thread_fn(&mut stream, id, &rx, heartbeat_timeout))
            .expect("Cannot spawn peer write thread");

        Self {
            id,
            tx: Some(tx),
            read_thread: Some(read_thread),
            write_thread: Some(write_thread),
            terminated,
        }
    }

    pub fn start_handshake(
        mut stream: TcpStream,
        config: &Arc<Config>,
        expected_id: Id,
        server_tx: Sender<Event>,
    ) -> Result<JoinSuccess, HandshakeError> {
        stream
            .set_read_timeout(Some(config.heartbeat_timeout * 3))
            .expect("Can't set stream read timeout");

        stream.send(&Message::Join(config.id))?;

        match stream.recv()? {
            Message::JoinAck(peer_id) => {
                if peer_id == expected_id {
                    console::log!("Connected to peer {peer_id}");
                    Ok(JoinSuccess {
                        peer_id,
                        peer: Self::new(stream, peer_id, server_tx, config.heartbeat_timeout),
                    })
                } else {
                    Err(HandshakeError::IdMismatch {
                        expected: expected_id,
                        received: peer_id,
                    })
                }
            }
            message => Err(HandshakeError::NoJoinAck(message)),
        }
    }

    pub fn send_message(&self, message: Message) {
        if let Err(e) = self
            .tx
            .as_ref()
            .expect("Invariant violated: tx should exist before Peer::drop")
            .send(message)
        {
            console::error!(&e, "Cannot relay messages to peer {} write thread", self.id);
        }
    }

    fn read_thread_fn(
        stream: &mut TcpStream,
        peer_id: Id,
        server_tx: &Sender<Event>,
        terminated: &Arc<AtomicBool>,
    ) {
        // This will not block forever, it will eventually time out,
        // as set_read_timeout was called in Peer::new
        while let Ok(message) = stream.recv() {
            match message {
                Message::Join(_) | Message::JoinAck { .. } => {
                    console::warning!("Peer {peer_id} tried to send {message:?} after handshake");
                    break;
                }
                _ => server_tx
                    .send(Event::PeerMessage {
                        sender_id: peer_id,
                        message,
                    })
                    .expect("Invariant violated: server_rx dropped before peer::disconnect"),
            }
            if terminated.load(Ordering::Relaxed) {
                console::warning!("Connection to {peer_id} was terminated by us");
                break;
            }
        }

        server_tx
            .send(Event::PeerLost(peer_id))
            .expect("Main thread messaging failed!");

        console::log!("Connection to peer {peer_id} lost");
    }

    fn write_thread_fn(
        stream: &mut TcpStream,
        peer_id: Id,
        rx: &Receiver<Message>,
        timeout: Duration,
    ) {
        loop {
            match rx.recv_timeout(timeout) {
                Ok(message) => {
                    if let Err(e) = stream.send(&message) {
                        console::error!(&e, "Failed to send {message:?} to peer {peer_id}");
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if let Err(e) = stream.send(&Message::Heartbeat) {
                        console::error!(&e, "Failed to send heartbeat to peer {peer_id}");
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    break;
                }
            }
        }
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        drop(self.tx.take());
        self.terminated.store(true, Ordering::Relaxed);
        if let Some(read_thread) = self.read_thread.take() {
            read_thread.join_and_handle_panic();
        }
        if let Some(write_thread) = self.write_thread.take() {
            write_thread.join_and_handle_panic();
        }
    }
}

impl Display for Peer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.id)
    }
}
