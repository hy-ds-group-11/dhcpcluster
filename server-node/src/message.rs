//! # Internal Server-to-Server Protocol
//!
//! This module contains traits for establishing connections and sending and receiving messages to server peers.
//!
//! The server communication style is message passing with multicast.
//!
//! Jump to [`Message`] for the server-to-server message definition.

use crate::{
    dhcp::{Ipv4Range, Lease},
    peer,
};
use protocol::{RecvCbor, SendCbor};
use serde::{Deserialize, Serialize};

/// # A server-to-server message
#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub enum Message {
    Join(peer::Id),
    JoinAck {
        peer_id: peer::Id,
        leases: Vec<Lease>,
    },
    Heartbeat,
    Election,
    Okay,
    Coordinator,
    Lease(Lease),
    SetPool(Ipv4Range),
    SetMajority(bool),
}

impl RecvCbor for Message {}
impl SendCbor for Message {}
