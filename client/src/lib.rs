pub mod config;

use protocol::{DhcpClientMessage, DhcpServerMessage, RecvCbor, SendCbor};
use std::{
    error::Error,
    net::{Ipv4Addr, SocketAddr, TcpStream},
};

pub fn get_offer(
    server_addr: SocketAddr,
    mac_address: [u8; 6],
) -> Result<Option<DhcpServerMessage>, Box<dyn Error>> {
    let stream = TcpStream::connect(server_addr)?;
    DhcpClientMessage::send(&stream, &DhcpClientMessage::Discover { mac_address })?;
    let response = DhcpServerMessage::recv(&stream)?;
    match response {
        offer @ DhcpServerMessage::Offer { .. } => Ok(Some(offer)),
        DhcpServerMessage::Ack => Err("Unexpected Ack from server".into()),
        DhcpServerMessage::Nack => Ok(None),
    }
}

pub fn get_ack(
    server_addr: SocketAddr,
    mac_address: [u8; 6],
    ip: Ipv4Addr,
) -> Result<Option<DhcpServerMessage>, Box<dyn Error>> {
    let stream = TcpStream::connect(server_addr)?;
    DhcpClientMessage::send(&stream, &DhcpClientMessage::Request { mac_address, ip })?;
    let response = DhcpServerMessage::recv(&stream)?;
    match response {
        DhcpServerMessage::Offer { .. } => Err("Unexpected Offer from server".into()),
        ack @ DhcpServerMessage::Ack => Ok(Some(ack)),
        DhcpServerMessage::Nack => Ok(None),
    }
}
