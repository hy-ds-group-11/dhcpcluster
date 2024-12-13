use crate::{
    config::Config, console, dhcp::Lease, message::Message, server::MainThreadMessage, ThreadJoin,
};
use protocol::{RecvCbor, SendCbor};
use std::{
    fmt::Display,
    net::TcpStream,
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
}

#[derive(Debug)]
pub struct JoinSuccess {
    pub peer_id: Id,
    pub peer: Peer,
    pub leases: Vec<Lease>,
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
    #[must_use]
    pub fn new(
        stream: TcpStream,
        id: Id,
        server_tx: Sender<MainThreadMessage>,
        heartbeat_timeout: Duration,
    ) -> Self {
        console::log!("Started connection to peer with id {id}");
        let (tx, rx) = mpsc::channel();
        let terminated = Arc::new(AtomicBool::new(false));

        let stream_read = stream.try_clone().expect("Cannot clone TcpStream!");
        // Set read to timeout if three heartbeats are missed. Three was chosen arbitrarily
        stream_read
            .set_read_timeout(Some(3 * heartbeat_timeout))
            .expect("set_read_timeout call failed");

        let read_thread = {
            let terminated = Arc::clone(&terminated);
            thread::Builder::new()
                .name(format!("{}::read_thread({})", module_path!(), id))
                .spawn(move || Self::read_thread_fn(&stream_read, id, &server_tx, &terminated))
                .expect("Cannot spawn peer read thread")
        };

        let write_thread = thread::Builder::new()
            .name(format!("{}::write_thread({})", module_path!(), id))
            .spawn(move || Self::write_thread_fn(&stream, id, &rx, heartbeat_timeout))
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
        stream: TcpStream,
        config: &Arc<Config>,
        server_tx: Sender<MainThreadMessage>,
    ) -> Result<JoinSuccess, HandshakeError> {
        Message::send(&stream, &Message::Join(config.id))?;
        match Message::recv_timeout(&stream, config.heartbeat_timeout)? {
            Message::JoinAck(peer_id, leases) => {
                console::log!("Connected to peer {peer_id}");
                Ok(JoinSuccess {
                    peer_id,
                    peer: Self::new(stream, peer_id, server_tx, config.heartbeat_timeout),
                    leases,
                })
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
        stream: &TcpStream,
        peer_id: Id,
        server_tx: &Sender<MainThreadMessage>,
        terminated: &Arc<AtomicBool>,
    ) {
        // This will not block forever, it will eventually time out,
        // as set_read_timeout was called in Peer::new
        while let Ok(message) = Message::recv(stream) {
            match message {
                Message::Join(_) | Message::JoinAck(..) => {
                    console::warning!("Peer {peer_id} tried to send {message:?} after handshake");
                    break;
                }
                _ => server_tx
                    .send(MainThreadMessage::PeerMessage {
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
            .send(MainThreadMessage::PeerLost(peer_id))
            .expect("Main thread messaging failed!");

        console::log!("Connection to peer {peer_id} lost");
    }

    fn write_thread_fn(stream: &TcpStream, peer_id: Id, rx: &Receiver<Message>, timeout: Duration) {
        loop {
            match rx.recv_timeout(timeout) {
                Ok(message) => {
                    if let Err(e) = Message::send(stream, &message) {
                        console::error!(&e, "Failed to send {message:?} to peer {peer_id}");
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if let Err(e) = Message::send(stream, &Message::Heartbeat) {
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
