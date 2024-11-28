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
    io::{self, Read, Write},
    net::{TcpListener, TcpStream, ToSocketAddrs},
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

/// # A trait for listening to incoming connections
///
/// You probably want to use [`std::net::TcpListener`] in production code (implementation provided in this crate),
/// or implement the trait for any alternative communication channel with applicable semantics.
pub trait MessageListener: Send + Sized {
    type Stream: MessageStream;

    fn bind(addr: impl ToSocketAddrs) -> io::Result<Self>;

    fn incoming(&self) -> impl Iterator<Item = io::Result<Self::Stream>>;
}

// Disable coverage for TcpListener, testing is done with [`test::MockListener`]
#[cfg_attr(coverage_nightly, coverage(off))]
impl MessageListener for TcpListener {
    type Stream = TcpStream;

    fn bind(addr: impl ToSocketAddrs) -> io::Result<Self> {
        Self::bind(addr)
    }

    fn incoming(&self) -> impl Iterator<Item = io::Result<Self::Stream>> {
        self.incoming()
    }
}

/// # A trait for receiving and sending [`Message`]s
///
/// You probably want to use [`std::net::TcpStream`] in production code (implementation provided in this crate),
/// or implement the trait for any alternative communication channel with applicable semantics.
pub trait MessageStream: Read + Write + Send + Sized {
    fn connect(addr: impl ToSocketAddrs) -> io::Result<Self>;

    fn set_read_timeout(&self, dur: Option<Duration>) -> io::Result<()>;

    fn try_clone(&self) -> io::Result<Self>;

    /// Reads and deserializes a message
    fn receive_message(&mut self) -> Result<Message, ciborium::de::Error<io::Error>> {
        ciborium::from_reader::<Message, &mut Self>(self)
    }

    /// Serializes and writes a message
    fn send_message(&mut self, message: &Message) -> Result<(), ciborium::ser::Error<io::Error>> {
        ciborium::into_writer::<Message, &mut Self>(message, self)
    }
}

// Disable coverage for TcpStream, testing is done with [`test::MockStream`]
#[cfg_attr(coverage_nightly, coverage(off))]
impl MessageStream for TcpStream {
    fn connect(addr: impl ToSocketAddrs) -> io::Result<Self> {
        Self::connect(addr)
    }

    fn set_read_timeout(&self, dur: Option<Duration>) -> io::Result<()> {
        self.set_read_timeout(dur)
    }

    fn try_clone(&self) -> io::Result<Self> {
        self.try_clone()
    }
}
