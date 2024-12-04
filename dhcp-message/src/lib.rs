use ciborium::{de, ser};
use serde::{Deserialize, Serialize};
use std::{io::Read, net::Ipv4Addr};

#[derive(Serialize, Deserialize, Debug)]
pub enum DHCPRequest {
    Discover {
        mac_address: Vec<u8>,
    },
    Offer {
        ip: Ipv4Addr,
        lease_time: u32,
        subnet_mask: Ipv4Addr,
    },
    Request {
        mac_address: Vec<u8>,
        ip: Ipv4Addr,
    },
    Ack,
    Nak,
}

#[derive(Debug)]
pub struct DhcpMessage {
    pub request: DHCPRequest,
}

impl DhcpMessage {
    pub fn new(request: DHCPRequest) -> Self {
        Self { request }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut data = Vec::new();
        ser::into_writer(&self.request, &mut data).unwrap();

        data
    }

    pub fn decode(bytes: impl Read) -> Self {
        let request = de::from_reader(bytes).unwrap();

        Self { request }
    }
}
