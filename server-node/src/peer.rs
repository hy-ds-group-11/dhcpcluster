use std::{
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use crate::{message::Message, MessageStream, SharedState};

enum InternalMessage {
    Terminate,
    Relay(Message),
}

/// Peer connection
#[allow(dead_code)]
pub struct Peer {
    id: u32,
    tx: Sender<InternalMessage>,
    read_thread: JoinHandle<()>,
    write_thread: JoinHandle<()>,
}

impl Peer {
    pub fn new(
        stream: impl MessageStream + 'static,
        id: u32,
        shared_state: Arc<SharedState>,
        heartbeat_timeout: Duration,
    ) -> Self {
        println!("Started connection to peer with id {id}");
        let (tx, rx) = mpsc::channel::<InternalMessage>();

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
                Self::read_thread_fn(stream_read, id, tx, shared_state)
            }),
            write_thread: thread::spawn(move || {
                Self::write_thread_fn(stream_write, rx, heartbeat_timeout)
            }),
        }
    }

    #[allow(dead_code)]
    pub fn send_message(&self, message: Message) {
        self.tx
            .send(InternalMessage::Relay(message))
            .unwrap_or_else(|e| eprintln!("{e:?}"));
    }

    fn read_thread_fn(
        mut stream: impl MessageStream,
        id: u32,
        tx: Sender<InternalMessage>,
        shared_state: Arc<SharedState>,
    ) {
        use Message::*;
        loop {
            let result = stream.receive_message();
            dbg!(&result);

            // Handle peer connection loss
            if result.is_err() {
                tx.send(InternalMessage::Terminate)
                    .expect("Thread internal messaging failed!");
                shared_state
                    .peers
                    .lock()
                    .unwrap()
                    .retain(|peer| peer.id != id);
                break;
            }

            let message = result.unwrap();
            match message {
                Join(_) | JoinAck(_) => {
                    panic!("Peer {id} tried to send {message:?} after handshake")
                }
                Heartbeat => println!("Received heartbeat from {id}"),
                Election => todo!(),
                Okay => todo!(),
                Coordinator => todo!(),
                Add(lease) => {
                    // TODO: checks needed for the lease
                    shared_state.leases.lock().unwrap().push(lease);
                }
                Update(_lease) => todo!(),
            }
        }

        println!("Connection to peer {id} lost");
    }

    fn write_thread_fn(
        mut stream: impl MessageStream,
        rx: Receiver<InternalMessage>,
        timeout: Duration,
    ) {
        use InternalMessage::*;
        loop {
            let receive = rx.recv_timeout(timeout);

            if receive.is_err() {
                let _ = stream.send_message(&Message::Heartbeat);
                continue;
            }

            match receive.unwrap() {
                Relay(message) => {
                    stream
                        .send_message(&message)
                        .unwrap_or_else(|e| eprintln!("{e:?}"));
                }
                Terminate => break,
            }
        }
    }
}
