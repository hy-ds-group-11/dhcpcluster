use crate::{config::Config, console, message::Message, server::ServerThreadMessage, ThreadJoin};
use protocol::{RecvCbor, SendCbor};
use std::{
    error::Error,
    fmt::Display,
    net::TcpStream,
    sync::mpsc::{self, Receiver, Sender},
    thread::{self, JoinHandle},
    time::Duration,
};

enum SenderThreadMessage {
    Terminate,
    Relay(Message),
}

pub type PeerId = u32;

/// Peer connection
#[derive(Debug)]
pub struct Peer {
    pub id: PeerId,
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
        config: &Config,
        server_tx: Sender<ServerThreadMessage>,
    ) -> Result<Peer, Box<dyn Error>> {
        let result = Message::send(&stream, &Message::Join(config.id));
        match result {
            Ok(_) => {
                let message = Message::recv_timeout(&stream, config.heartbeat_timeout).unwrap();

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

    pub fn send_message(&self, message: Message) {
        self.tx
            .send(SenderThreadMessage::Relay(message))
            .unwrap_or_else(|e| console::warning!("{e:?}"));
    }

    pub fn join(self) -> Result<(), String> {
        // TODO implement ^C events? Dropping the channel tx should be enough
        // to stop the threads, as seen in thread_pool
        self.read_thread.join_and_format_error()?;
        self.write_thread.join_and_format_error()?;
        Ok(())
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
                Join(_) | JoinAck(_) => {
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
            let receive = rx.recv_timeout(timeout);

            if receive.is_err() {
                Message::send(&stream, &Message::Heartbeat).unwrap();
                continue;
            }

            match receive.unwrap() {
                Relay(message) => {
                    Message::send(&stream, &message).unwrap_or_else(|e| console::log!("{e:?}"));
                }
                Terminate => break,
            }
        }
    }
}

impl Display for Peer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.id)
    }
}
