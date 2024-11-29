//! # Internal Server-to-Server Protocol
//!
//! This module contains traits for establishing connections and sending and receiving messages to server peers.
//!
//! The server communication style is message passing with multicast.
//!
//! Jump to [`Message`] for the server-to-server message definition.

use crate::Lease;
use serde::{Deserialize, Serialize};
use std::{
    io::{self},
    net::TcpStream,
    time::Duration,
};

/// # A server-to-server message
#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub enum Message {
    Join(u32),
    JoinAck(u32),
    Heartbeat,
    Election,
    Okay,
    Coordinator,
    Add(Lease),
    Update(Lease),
}

pub fn recv(stream: &TcpStream) -> Result<Message, ciborium::de::Error<io::Error>> {
    ciborium::from_reader(stream)
}

pub fn recv_timeout(
    stream: &TcpStream,
    timeout: Duration,
) -> Result<Message, ciborium::de::Error<io::Error>> {
    stream.set_read_timeout(Some(timeout)).unwrap();
    let result = recv(stream);
    stream.set_read_timeout(None).unwrap();
    result
}

pub fn send(stream: &TcpStream, message: &Message) -> Result<(), ciborium::ser::Error<io::Error>> {
    ciborium::into_writer(message, stream)
}
