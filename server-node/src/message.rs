//! # Internal Server-to-Server Protocol
//!
//! This module contains traits for establishing connections and sending and receiving messages to server peers.
//!
//! The server communication style is message passing with multicast.
//!
//! Jump to [`Message`] for the server-to-server message definition.

use crate::{
    dhcp::{DhcpPool, Lease},
    peer::PeerId,
};
use serde::{Deserialize, Serialize};
use std::{
    io::{self},
    net::TcpStream,
    time::Duration,
};

/// # A server-to-server message
#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub enum Message {
    Join(PeerId),
    JoinAck(PeerId),
    Heartbeat,
    Election,
    Okay,
    Coordinator,
    Add(Lease),
    Update(Lease),
    SetPool(DhcpPool),
}

/// # Read a message from a [`TcpStream`].
/// This function can block the calling thread for the stream's current timeout setting (see [`TcpStream::set_read_timeout`]).
pub fn recv(stream: &TcpStream) -> Result<Message, ciborium::de::Error<io::Error>> {
    ciborium::from_reader(stream)
}

/// # Read a message from a [`TcpStream`], with a timeout.
/// This function can block the calling thread for the specified timeout duration.
/// ## Concurrency
/// This function may not be used concurrently with a stream that has been shared between different threads.
/// Doing so may result in unexpected changes to the stream's timeout.
pub fn recv_timeout(
    stream: &TcpStream,
    timeout: Duration,
) -> Result<Message, ciborium::de::Error<io::Error>> {
    let previous_timeout = stream.read_timeout().unwrap();
    stream.set_read_timeout(Some(timeout)).unwrap();
    let result = recv(stream);
    stream.set_read_timeout(previous_timeout).unwrap();
    result
}

/// # Send a message over a [`TcpStream`]
pub fn send(stream: &TcpStream, message: &Message) -> Result<(), ciborium::ser::Error<io::Error>> {
    ciborium::into_writer(message, stream)
}
