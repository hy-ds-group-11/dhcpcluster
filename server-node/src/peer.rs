use std::{
    net::TcpStream,
    sync::mpsc::{self, Receiver, Sender},
    thread::{self, JoinHandle},
};

use crate::message::Message;

/// Peer connection
pub struct Peer {
    _id: u32,
    _tx: Sender<Message>,
    _read_thread: JoinHandle<()>,
    _write_thread: JoinHandle<()>,
}

impl Peer {
    pub fn new(stream: TcpStream, id: u32) -> Self {
        println!("Started connection to peer with id {id}");
        let stream_clone = stream.try_clone().unwrap();
        let (tx, rx) = mpsc::channel::<Message>();

        Self {
            _id: id,
            _tx: tx,
            _read_thread: thread::spawn(move || Self::read_thread_fn(stream_clone)),
            _write_thread: thread::spawn(move || Self::write_thread_fn(stream, rx)),
        }
    }

    pub fn _send_message(&self, message: Message) {
        self._tx
            .send(message)
            .unwrap_or_else(|e| eprintln!("{e:?}"));
    }

    fn read_thread_fn(stream: TcpStream) {
        loop {
            let message = ciborium::from_reader::<Message, &TcpStream>(&stream).unwrap();
            dbg!(&message);
            match message {
                Message::Join(_) => panic!("Peer tried to send Join message after handshake"),
                Message::JoinAck(_) => panic!("Peer tried to send JoinAck message after handshake"),
                Message::Heartbeat => todo!(),
                Message::Election => todo!(),
                Message::Okay => todo!(),
                Message::Coordinator => todo!(),
                Message::Add(_lease) => todo!(),
                Message::Update(_lease) => todo!(),
            }
        }
    }

    fn write_thread_fn(stream: TcpStream, rx: Receiver<Message>) {
        for message in rx {
            ciborium::into_writer::<Message, &TcpStream>(&message, &stream)
                .unwrap_or_else(|e| eprintln!("{e:?}"));
        }
    }
}
