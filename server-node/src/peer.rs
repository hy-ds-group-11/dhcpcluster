use std::{
    sync::mpsc::{self, Receiver, Sender},
    thread::{self, JoinHandle},
};

use crate::{message::Message, Leases, MessageStream};

/// Peer connection
pub struct Peer {
    _id: u32,
    _tx: Sender<Message>,
    _read_thread: JoinHandle<()>,
    _write_thread: JoinHandle<()>,
}

impl Peer {
    pub fn new(stream: impl MessageStream + 'static, id: u32, leases: Leases) -> Self {
        println!("Started connection to peer with id {id}");
        let (tx, rx) = mpsc::channel::<Message>();

        let stream_read = stream.clone();
        let stream_write = stream;
        Self {
            _id: id,
            _tx: tx,
            _read_thread: thread::spawn(move || Self::read_thread_fn(stream_read, leases)),
            _write_thread: thread::spawn(move || Self::write_thread_fn(stream_write, rx)),
        }
    }

    pub fn _send_message(&self, message: Message) {
        self._tx
            .send(message)
            .unwrap_or_else(|e| eprintln!("{e:?}"));
    }

    fn read_thread_fn(mut stream: impl MessageStream, leases: Leases) {
        loop {
            let message = stream.receive_message().unwrap();
            dbg!(&message);
            match message {
                Message::Join(_) => panic!("Peer tried to send Join message after handshake"),
                Message::JoinAck(_) => panic!("Peer tried to send JoinAck message after handshake"),
                Message::Heartbeat => todo!(),
                Message::Election => todo!(),
                Message::Okay => todo!(),
                Message::Coordinator => todo!(),
                Message::Add(lease) => {
                    // TODO: checks needed for the lease
                    leases.lock().unwrap().push(lease);
                }
                Message::Update(_lease) => todo!(),
            }
        }
    }

    fn write_thread_fn(mut stream: impl MessageStream, rx: Receiver<Message>) {
        for message in rx {
            stream
                .send_message(&message)
                .unwrap_or_else(|e| eprintln!("{e:?}"));
        }
    }
}
