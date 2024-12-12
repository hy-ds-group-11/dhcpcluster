use crate::{
    config::Config, console, dhcp::Lease, message::Message, server::ServerThreadMessage, ThreadJoin,
};
use protocol::{RecvCbor, SendCbor};
use std::{
    fmt::Display,
    net::TcpStream,
    sync::{
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
    pub peer_id: PeerId,
    pub peer: Peer,
    pub leases: Vec<Lease>,
}

enum SenderThreadMessage {
    Terminate,
    Relay(Message),
}

pub type PeerId = u32;

/// Peer connection
#[derive(Debug)]
pub struct Peer {
    id: PeerId,
    tx: Sender<SenderThreadMessage>,
    read_thread: JoinHandle<()>,
    write_thread: JoinHandle<()>,
}

impl Peer {
    pub fn new(
        stream: TcpStream,
        id: PeerId,
        server_tx: Sender<ServerThreadMessage>,
        heartbeat_timeout: Duration,
    ) -> Self {
        console::log!("Started connection to peer with id {id}");
        let (tx, rx) = mpsc::channel::<SenderThreadMessage>();

        let stream_read = stream.try_clone().unwrap();
        // Set read to timeout if three heartbeats are missed. Three was chosen arbitrarily
        stream_read
            .set_read_timeout(Some(3 * heartbeat_timeout))
            .expect("set_read_timeout call failed");

        let stream_write = stream;
        Self {
            id,
            tx: tx.clone(),
            read_thread: thread::Builder::new()
                .name(format!("{}::read_thread({})", module_path!(), id))
                .spawn(move || Self::read_thread_fn(stream_read, id, tx, server_tx))
                .unwrap(),
            write_thread: thread::Builder::new()
                .name(format!("{}::write_thread({})", module_path!(), id))
                .spawn(move || Self::write_thread_fn(stream_write, rx, heartbeat_timeout))
                .unwrap(),
        }
    }

    pub fn start_handshake(
        stream: TcpStream,
        config: Arc<Config>,
        server_tx: Sender<ServerThreadMessage>,
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
        self.tx
            .send(SenderThreadMessage::Relay(message))
            .unwrap_or_else(|e| console::warning!("{e:?}"));
    }

    pub fn disconnect(self) {
        std::mem::drop(self.tx);
        // TODO implement ^C events? Dropping the channel tx should be enough
        // to stop the threads, as seen in thread_pool
        self.read_thread.join_and_handle_panic();
        self.write_thread.join_and_handle_panic();
    }

    fn read_thread_fn(
        stream: TcpStream,
        peer_id: PeerId,
        tx: Sender<SenderThreadMessage>,
        server_tx: Sender<ServerThreadMessage>,
    ) {
        loop {
            let result = Message::recv(&stream);

            // Handle peer connection loss
            if result.is_err() {
                tx.send(SenderThreadMessage::Terminate)
                    .expect("Thread internal messaging failed!");

                server_tx
                    .send(ServerThreadMessage::PeerLost(peer_id))
                    .expect("Thread internal messaging failed!");

                break;
            }

            let message = result.unwrap();
            use Message::*;
            match message {
                Join(_) | JoinAck(..) => {
                    panic!("Peer {peer_id} tried to send {message:?} after handshake")
                }
                _ => server_tx
                    .send(ServerThreadMessage::ProtocolMessage {
                        sender_id: peer_id,
                        message,
                    })
                    .unwrap(),
            }
        }

        console::log!("Connection to peer {peer_id} lost");
    }

    fn write_thread_fn(stream: TcpStream, rx: Receiver<SenderThreadMessage>, timeout: Duration) {
        use SenderThreadMessage::*;
        loop {
            match rx.recv_timeout(timeout) {
                Ok(message) => match message {
                    Terminate => break,
                    Relay(message) => {
                        Message::send(&stream, &message).unwrap_or_else(|e| console::log!("{e:?}"));
                    }
                },
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    Message::send(&stream, &Message::Heartbeat).unwrap();
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    break;
                }
            }
        }
    }
}

impl Display for Peer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.id)
    }
}
