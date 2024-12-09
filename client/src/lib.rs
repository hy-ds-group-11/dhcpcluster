pub mod config;

use protocol::{
    CborRecvError, CborSendError, DhcpClientMessage, DhcpOffer, DhcpServerMessage, RecvCbor,
    SendCbor,
};
use std::net::{Ipv4Addr, TcpStream};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CommunicationError {
    #[error("Failed to receive message from server")]
    ReceiveMessage(#[from] CborRecvError),
    #[error("Failed to send message to server")]
    SendMessage(#[from] CborSendError),
    #[error("Unexpected {0:?} from server")]
    UnexpectedMessage(DhcpServerMessage),
}

pub fn get_offer(
    stream: &TcpStream,
    mac_address: [u8; 6],
) -> Result<Option<DhcpOffer>, CommunicationError> {
    DhcpClientMessage::send(stream, &DhcpClientMessage::Discover { mac_address })?;
    let response = DhcpServerMessage::recv(stream)?;
    match response {
        DhcpServerMessage::Offer(offer) => Ok(Some(offer)),
        msg @ DhcpServerMessage::Ack => Err(CommunicationError::UnexpectedMessage(msg)),
        DhcpServerMessage::Nack => Ok(None),
    }
}

pub fn get_ack(
    stream: &TcpStream,
    mac_address: [u8; 6],
    ip: Ipv4Addr,
) -> Result<bool, CommunicationError> {
    DhcpClientMessage::send(stream, &DhcpClientMessage::Request { mac_address, ip })?;
    let response = DhcpServerMessage::recv(stream)?;
    match response {
        msg @ DhcpServerMessage::Offer(_) => Err(CommunicationError::UnexpectedMessage(msg)),
        DhcpServerMessage::Ack => Ok(true),
        DhcpServerMessage::Nack => Ok(false),
    }
}
