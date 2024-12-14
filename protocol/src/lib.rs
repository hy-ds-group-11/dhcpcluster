#![deny(clippy::unwrap_used, clippy::allow_attributes_without_reason)]
#![warn(clippy::perf, clippy::complexity, clippy::pedantic, clippy::suspicious)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    reason = "We're not going to write comprehensive docs"
)]
#![allow(
    clippy::cast_precision_loss,
    reason = "There are no sufficient floating point types"
)]

//! This crate defines a custom client-server (relay agent to cluster) protocol.
//! In a final version of this software, this protocol should be replaced by the actual DHCP protocol.

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    fmt::Display,
    io::{Read, Write},
    net::{Ipv4Addr, TcpStream},
    num::ParseIntError,
    str::FromStr,
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
        let mut bytes = [0; 6];
        let mut split = value.split(':');
        for (i, byte) in bytes.iter_mut().enumerate() {
            match split.next() {
                Some(octet) => {
                    *byte = u8::from_str_radix(octet, 16).map_err(|e| {
                        MacAddrParseError::ParseOctet {
                            index: i,
                            source: e,
                        }
                    })?;
                }
                None => return Err(MacAddrParseError::Short(i)),
            }
        }

        if split.next().is_some() {
            return Err(MacAddrParseError::Long);
        }

        Ok(Self(bytes))
    }
}

impl Display for MacAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in self.0.iter().take(5) {
            write!(f, "{byte:0>2X}:")?;
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

pub type RecvError = ciborium::de::Error<std::io::Error>;
pub type SendError = ciborium::ser::Error<std::io::Error>;

#[derive(Error, Debug)]
pub enum CborRecvError {
    #[error("Failed to receive incoming CBOR")]
    Receive(#[from] RecvError),
    #[error("Failed to set stream read timeout")]
    SetTimeout(#[source] std::io::Error),
    #[error("Failed to access stream read timeout")]
    GetTimeout(#[source] std::io::Error),
}

#[derive(Error, Debug)]
pub enum CborSendError {
    #[error("Failed to send CBOR")]
    Send(#[from] SendError),
}

pub trait RecvCbor<M: DeserializeOwned>: Sized + Read {
    /// # Receive a message from self
    /// This function can block the calling thread for the stream's current read timeout setting
    /// (see [`TcpStream::set_read_timeout`]).
    fn recv(&mut self) -> Result<M, CborRecvError> {
        Ok(ciborium::from_reader(self)?)
    }
}

pub trait SendCbor<M: Serialize>: Sized + Write {
    /// # Send a message over self
    fn send(&mut self, message: &M) -> Result<(), CborSendError> {
        Ok(ciborium::into_writer(message, self)?)
    }
}

impl RecvCbor<DhcpClientMessage> for TcpStream {}
impl SendCbor<DhcpClientMessage> for TcpStream {}
impl RecvCbor<DhcpServerMessage> for TcpStream {}
impl SendCbor<DhcpServerMessage> for TcpStream {}
