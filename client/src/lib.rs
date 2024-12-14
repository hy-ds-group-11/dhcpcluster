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

pub mod config;

use protocol::{
    CborRecvError, CborSendError, DhcpClientMessage, DhcpOffer, DhcpServerMessage, MacAddr,
    RecvCbor, SendCbor,
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
    stream: &mut TcpStream,
    mac_address: MacAddr,
) -> Result<Option<DhcpOffer>, CommunicationError> {
    stream.send(&DhcpClientMessage::Discover { mac_address })?;
    let response = stream.recv()?;
    match response {
        DhcpServerMessage::Offer(offer) => Ok(Some(offer)),
        msg @ DhcpServerMessage::Ack => Err(CommunicationError::UnexpectedMessage(msg)),
        DhcpServerMessage::Nack => Ok(None),
    }
}

pub fn get_ack(
    stream: &mut TcpStream,
    mac_address: MacAddr,
    ip: Ipv4Addr,
) -> Result<bool, CommunicationError> {
    stream.send(&DhcpClientMessage::Request { mac_address, ip })?;
    let response = stream.recv()?;
    match response {
        msg @ DhcpServerMessage::Offer(_) => Err(CommunicationError::UnexpectedMessage(msg)),
        DhcpServerMessage::Ack => Ok(true),
        DhcpServerMessage::Nack => Ok(false),
    }
}
