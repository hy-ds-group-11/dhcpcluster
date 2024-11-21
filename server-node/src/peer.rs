use std::{net::TcpStream, thread, thread::JoinHandle};

/// Peer connection
pub struct Peer {
    id: Option<u32>,
    receive_thread: JoinHandle<()>,
    send_thread: JoinHandle<()>,
}

impl Peer {
    fn handle_read(stream: TcpStream) {}
    fn handle_write(stream: TcpStream) {}

    pub fn new(stream: TcpStream) -> Self {
        let stream_clone = stream.try_clone().unwrap();
        Self {
            id: None,
            receive_thread: thread::spawn(move || Self::handle_read(stream_clone)),
            send_thread: thread::spawn(move || Self::handle_write(stream)),
        }
    }
}
