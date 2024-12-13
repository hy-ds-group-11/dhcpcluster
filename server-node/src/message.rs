//! # Internal Server-to-Server Protocol
//!
//! This module contains traits for establishing connections and sending and receiving messages to server peers.
//!
//! The server communication style is message passing with multicast.
//!
//! Jump to [`Message`] for the server-to-server message definition.

use crate::{
    dhcp::{self, Lease},
    peer,
};
use protocol::{RecvCbor, SendCbor};
use serde::{Deserialize, Serialize};

/// # A server-to-server message
#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub enum Message {
    Join(peer::Id),
    JoinAck(peer::Id, Vec<Lease>),
    Heartbeat,
    Election,
    Okay,
    Coordinator,
    Lease(Lease),
    // TODO: this is BAD! Use dhcp::Pool and other required parameters, not service internals
    SetPool(dhcp::Service),
    SetMajority(bool),
}

impl RecvCbor for Message {}
impl SendCbor for Message {}
