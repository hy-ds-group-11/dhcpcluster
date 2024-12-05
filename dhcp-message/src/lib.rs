//! This crate defines a custom client-server (relay agent to cluster) protocol.
//! In a final version of this software, this protocol should be replaced by the actual DHCP protocol.

use serde::{Deserialize, Serialize};
use std::{
    io::Error,
    net::{Ipv4Addr, TcpStream},
    time::Duration,
};

#[derive(Serialize, Deserialize, Debug)]
pub enum DhcpClientMessage {
    Discover { mac_address: [u8; 6] },
    Request { mac_address: [u8; 6], ip: Ipv4Addr },
}

#[derive(Serialize, Deserialize, Debug)]
pub enum DhcpServerMessage {
    Offer {
        ip: Ipv4Addr,
        lease_time: u32,
        subnet_mask: u32,
    },
    Ack,
    Nack,
}

impl DhcpClientMessage {
    /// # Read a message from a [`TcpStream`].
    /// This function can block the calling thread for the stream's current timeout setting (see [`TcpStream::set_read_timeout`]).
    pub fn recv(stream: &TcpStream) -> Result<Self, ciborium::de::Error<Error>> {
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
    ) -> Result<Self, ciborium::de::Error<Error>> {
        let previous_timeout = stream.read_timeout().unwrap();
        stream.set_read_timeout(Some(timeout)).unwrap();
        let result = Self::recv(stream);
        stream.set_read_timeout(previous_timeout).unwrap();
        result
    }

    /// # Send a message over a [`TcpStream`]
    pub fn send(stream: &TcpStream, message: &Self) -> Result<(), ciborium::ser::Error<Error>> {
        ciborium::into_writer(message, stream)
    }
}

impl DhcpServerMessage {
    /// # Read a message from a [`TcpStream`].
    /// This function can block the calling thread for the stream's current timeout setting (see [`TcpStream::set_read_timeout`]).
    pub fn recv(stream: &TcpStream) -> Result<Self, ciborium::de::Error<Error>> {
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
    ) -> Result<Self, ciborium::de::Error<Error>> {
        let previous_timeout = stream.read_timeout().unwrap();
        stream.set_read_timeout(Some(timeout)).unwrap();
        let result = Self::recv(stream);
        stream.set_read_timeout(previous_timeout).unwrap();
        result
    }

    /// # Send a message over a [`TcpStream`]
    pub fn send(stream: &TcpStream, message: &Self) -> Result<(), ciborium::ser::Error<Error>> {
        ciborium::into_writer(message, stream)
    }
}
