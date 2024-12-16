//! # Internal Server-to-Server Protocol
//!
//! This module contains traits for establishing connections and sending and receiving messages to server peers.
//!
//! The server communication style is message passing with multicast.
//!
//! Jump to [`Message`] for the server-to-server message definition.

use crate::{
    dhcp::{Ipv4Range, Lease},
    server::peer,
};
use protocol::{RecvCbor, SendCbor};
use serde::{Deserialize, Serialize};
use std::net::TcpStream;

/// # A server-to-server message
#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub enum Message {
    Join(peer::Id),
    JoinAck(peer::Id),
    Heartbeat,
    Election,
    Okay,
    Coordinator { majority: bool },
    Lease(Lease),
    LeaseTable(Vec<Lease>),
    SetPool(Ipv4Range),
}

impl RecvCbor<Message> for TcpStream {}
impl SendCbor<Message> for TcpStream {}
