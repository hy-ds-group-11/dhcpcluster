use std::{
    net::TcpStream,
    sync::mpsc::{self, Receiver, Sender},
    thread::{self, JoinHandle},
};

use crate::message::Message;

/// Peer connection
pub struct Peer {
    id: u32,
    tx: Sender<Message>,
    read_thread: JoinHandle<()>,
    write_thread: JoinHandle<()>,
}

impl Peer {
    pub fn new(stream: TcpStream, id: u32) -> Self {
        println!("Started connection to peer with id {id}");
        let stream_clone = stream.try_clone().unwrap();
        let (tx, rx) = mpsc::channel::<Message>();

        Self {
            id,
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
            let message = ciborium::from_reader::<Message, &TcpStream>(&stream).unwrap();
            dbg!(&message);
            match message {
                Message::Join(_) => panic!("Peer tried to send Join message after handshake"),
                Message::JoinAck(_) => panic!("Peer tried to send JoinAck message after handshake"),
                Message::Heartbeat => todo!(),
                Message::Election => todo!(),
                Message::Okay => todo!(),
                Message::Coordinator => todo!(),
                Message::Add(lease) => todo!(),
                Message::Update(lease) => todo!(),
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
