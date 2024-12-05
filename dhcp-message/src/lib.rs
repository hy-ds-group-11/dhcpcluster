//! This crate defines a custom client-server (relay agent to cluster) protocol.
//! In a final version of this software, this protocol should be replaced by the actual DHCP protocol.

use ciborium::{de, ser};
use serde::{Deserialize, Serialize};
use std::{io::Read, net::Ipv4Addr};

#[derive(Serialize, Deserialize, Debug)]
pub enum DhcpClientMessage {
    Discover { mac_address: Vec<u8> },
    Request { mac_address: Vec<u8>, ip: Ipv4Addr },
}

#[derive(Serialize, Deserialize, Debug)]
pub enum DhcpServerMessage {
    Offer {
        ip: Ipv4Addr,
        lease_time: u32,
        subnet_mask: Ipv4Addr,
    },
    Ack,
    Nak,
}

impl DhcpClientMessage {
    pub fn encode(&self) -> Vec<u8> {
        let mut data = Vec::new();
        ser::into_writer(&self, &mut data).unwrap();

        data
    }

    pub fn decode(bytes: impl Read) -> Self {
        de::from_reader(bytes).unwrap()
    }
}

impl DhcpServerMessage {
    pub fn encode(&self) -> Vec<u8> {
        let mut data = Vec::new();
        ser::into_writer(&self, &mut data).unwrap();

        data
    }

    pub fn decode(bytes: impl Read) -> Self {
        de::from_reader(bytes).unwrap()
    }
}
