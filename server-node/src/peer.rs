use std::{
    io::Read,
    net::TcpStream,
    sync::mpsc::{self, Receiver, Sender},
    thread::{self, sleep, JoinHandle},
};

use crate::message::Message;

/// Peer connection
pub struct Peer {
    id: Option<u32>,
    tx: Sender<Message>,
    read_thread: JoinHandle<()>,
    write_thread: JoinHandle<()>,
}

impl Peer {
    pub fn new(stream: TcpStream) -> Self {
        let stream_clone = stream.try_clone().unwrap();
        let (tx, rx) = mpsc::channel::<Message>();
        Self {
            id: None,
            tx,
            read_thread: thread::spawn(move || Self::read_thread_fn(stream_clone)),
            write_thread: thread::spawn(move || Self::write_thread_fn(stream, rx)),
        }
    }

    pub fn send_message(&self, message: Message) {
        self.tx.send(message).unwrap_or_else(|e| eprintln!("{e:?}"));
    }

    fn read_thread_fn(stream: TcpStream) {
        loop {
            let message = ciborium::from_reader::<Message, &TcpStream>(&stream);
            dbg!(message);
        }
    }

    fn write_thread_fn(stream: TcpStream, rx: Receiver<Message>) {
        for message in rx {
            ciborium::into_writer::<Message, &TcpStream>(&message, &stream)
                .unwrap_or_else(|e| eprintln!("{e:?}"));
        }
    }
}
