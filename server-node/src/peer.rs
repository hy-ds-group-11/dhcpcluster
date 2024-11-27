use std::{
    sync::mpsc::{self, Receiver, Sender},
    thread::{self, JoinHandle},
    time::Duration,
};

use crate::{message::Message, MessageStream, SharedState};

/// Peer connection
pub struct Peer {
    _id: u32,
    _tx: Sender<Message>,
    _read_thread: JoinHandle<()>,
    _write_thread: JoinHandle<()>,
}

impl Peer {
    pub fn new(
        stream: impl MessageStream + 'static,
        id: u32,
        shared_state: SharedState,
        heartbeat_timeout: Duration,
    ) -> Self {
        println!("Started connection to peer with id {id}");
        let (tx, rx) = mpsc::channel::<Message>();

        let stream_read = stream.try_clone().unwrap();
        let stream_write = stream;
        Self {
            _id: id,
            _tx: tx,
            _read_thread: thread::spawn(move || Self::read_thread_fn(stream_read, shared_state)),
            _write_thread: thread::spawn(move || {
                Self::write_thread_fn(stream_write, rx, heartbeat_timeout)
            }),
        }
    }

    pub fn _send_message(&self, message: Message) {
        self._tx
            .send(message)
            .unwrap_or_else(|e| eprintln!("{e:?}"));
    }

    fn read_thread_fn(mut stream: impl MessageStream, shared_state: SharedState) {
        use Message::*;
        loop {
            let message = stream.receive_message().unwrap();
            dbg!(&message);
            match message {
                Join(_) => panic!("Peer tried to send Join message after handshake"),
                JoinAck(_) => panic!("Peer tried to send JoinAck message after handshake"),
                Heartbeat => println!("Heartbeat"),
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
    }

    fn write_thread_fn(mut stream: impl MessageStream, rx: Receiver<Message>, timeout: Duration) {
        loop {
            let receive = rx.recv_timeout(timeout);
            if let Ok(message) = receive {
                stream
                    .send_message(&message)
                    .unwrap_or_else(|e| eprintln!("{e:?}"));
            } else {
                let _ = stream.send_message(&Message::Heartbeat);
            }
        }
    }
}
