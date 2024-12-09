pub mod config;

use protocol::{DhcpClientMessage, DhcpOffer, DhcpServerMessage, RecvCbor, SendCbor};
use std::{
    error::Error,
    net::{Ipv4Addr, SocketAddr, TcpStream},
};

pub fn get_offer(
    server_addr: SocketAddr,
    mac_address: [u8; 6],
) -> Result<Option<DhcpOffer>, Box<dyn Error>> {
    let stream = TcpStream::connect(server_addr)?;
    DhcpClientMessage::send(&stream, &DhcpClientMessage::Discover { mac_address })?;
    let response = DhcpServerMessage::recv(&stream)?;
    match response {
        DhcpServerMessage::Offer(offer) => Ok(Some(offer)),
        DhcpServerMessage::Ack => Err("Unexpected Ack from server".into()),
        DhcpServerMessage::Nack => Ok(None),
    }
}

pub fn get_ack(
    server_addr: SocketAddr,
    mac_address: [u8; 6],
    ip: Ipv4Addr,
) -> Result<bool, Box<dyn Error>> {
    let stream = TcpStream::connect(server_addr)?;
    DhcpClientMessage::send(&stream, &DhcpClientMessage::Request { mac_address, ip })?;
    let response = DhcpServerMessage::recv(&stream)?;
    match response {
        DhcpServerMessage::Offer(_) => Err("Unexpected Offer from server".into()),
        DhcpServerMessage::Ack => Ok(true),
        DhcpServerMessage::Nack => Ok(false),
    }
}
