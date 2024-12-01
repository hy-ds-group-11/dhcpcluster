use crate::{
    console,
    message::{self, Message},
    ServerThreadMessage,
};
use std::{
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
    #[allow(dead_code)]
    read_thread: JoinHandle<()>,
    #[allow(dead_code)]
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
            read_thread: thread::spawn(move || {
                Self::read_thread_fn(stream_read, id, tx, server_tx)
            }),
            write_thread: thread::spawn(move || {
                Self::write_thread_fn(stream_write, rx, heartbeat_timeout)
            }),
        }
    }

    pub fn send_message(&self, message: Message) {
        self.tx
            .send(SenderThreadMessage::Relay(message))
            .unwrap_or_else(|e| console::log!("{e:?}"));
    }

    fn read_thread_fn(
        stream: TcpStream,
        peer_id: PeerId,
        tx: Sender<SenderThreadMessage>,
        server_tx: Sender<ServerThreadMessage>,
    ) {
        loop {
            let result = message::recv(&stream);

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
                message::send(&stream, &Message::Heartbeat).unwrap();
                continue;
            }

            match receive.unwrap() {
                Relay(message) => {
                    message::send(&stream, &message).unwrap_or_else(|e| console::log!("{e:?}"));
                }
                Terminate => break,
            }
        }
    }
}
