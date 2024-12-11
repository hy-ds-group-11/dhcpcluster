//! This crate defines a custom client-server (relay agent to cluster) protocol.
//! In a final version of this software, this protocol should be replaced by the actual DHCP protocol.

use serde::{Deserialize, Serialize};
use std::{
    fmt::Display,
    net::{Ipv4Addr, TcpStream},
    num::ParseIntError,
    str::FromStr,
    time::Duration,
};
use thiserror::Error;

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone, Copy)]
pub struct MacAddr([u8; 6]);

impl From<[u8; 6]> for MacAddr {
    fn from(value: [u8; 6]) -> Self {
        Self(value)
    }
}

#[derive(Error, Debug)]
pub enum MacAddrParseError {
    #[error("Failed to parse octet {index} in MAC address")]
    ParseOctet { index: usize, source: ParseIntError },
    #[error("MAC address too short, expected 6 octets, got {0}")]
    Short(usize),
    #[error("MAC address too long, expected 6 octets")]
    Long,
}

impl FromStr for MacAddr {
    type Err = MacAddrParseError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut bytes = Vec::with_capacity(6);
        let mut split = value.split(":");
        for i in 0..6 {
            match split.next() {
                Some(octet) => {
                    bytes.push(u8::from_str_radix(octet, 16).map_err(|e| {
                        MacAddrParseError::ParseOctet {
                            index: i,
                            source: e,
                        }
                    })?);
                }
                None => return Err(MacAddrParseError::Short(i)),
            }
        }

        if split.next().is_some() {
            return Err(MacAddrParseError::Long);
        }

        Ok(Self([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5],
        ]))
    }
}

impl Display for MacAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in self.0.iter().take(5) {
            write!(f, "{:0>2X}:", byte)?;
        }
        write!(f, "{:0>2X}", self.0[5])
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub enum DhcpClientMessage {
    Discover { mac_address: MacAddr },
    Request { mac_address: MacAddr, ip: Ipv4Addr },
}

#[derive(Serialize, Deserialize, Debug)]
pub struct DhcpOffer {
    pub ip: Ipv4Addr,
    pub lease_time: u32,
    pub subnet_mask: u32,
}

impl Display for DhcpOffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}/{} for {}s",
            self.ip, self.subnet_mask, self.lease_time,
        )
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub enum DhcpServerMessage {
    Offer(DhcpOffer),
    Ack,
    Nack,
}

pub type CborRecvError = ciborium::de::Error<std::io::Error>;
pub type CborSendError = ciborium::ser::Error<std::io::Error>;

pub trait RecvCbor: Sized + for<'a> Deserialize<'a> {
    /// # Read a message from a [`TcpStream`].
    /// This function can block the calling thread for the stream's current timeout setting (see [`TcpStream::set_read_timeout`]).
    fn recv(stream: &TcpStream) -> Result<Self, CborRecvError> {
        ciborium::from_reader(stream)
    }

    /// # Read a message from a [`TcpStream`], with a timeout.
    /// This function can block the calling thread for the specified timeout duration.
    /// ## Concurrency
    /// This function may not be used concurrently with a stream that has been shared between different threads.
    /// Doing so may result in unexpected changes to the stream's timeout.
    fn recv_timeout(stream: &TcpStream, timeout: Duration) -> Result<Self, CborRecvError> {
        let previous_timeout = stream.read_timeout().unwrap();
        stream.set_read_timeout(Some(timeout)).unwrap();
        let result = Self::recv(stream);
        stream.set_read_timeout(previous_timeout).unwrap();
        result
    }
}

pub trait SendCbor: Sized + Serialize {
    /// # Send a message over a [`TcpStream`]
    fn send(stream: &TcpStream, message: &Self) -> Result<(), CborSendError> {
        ciborium::into_writer(message, stream)
    }
}

impl RecvCbor for DhcpClientMessage {}
impl SendCbor for DhcpClientMessage {}
impl RecvCbor for DhcpServerMessage {}
impl SendCbor for DhcpServerMessage {}
