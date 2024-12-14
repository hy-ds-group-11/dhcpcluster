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

//! # DHCP Cluster - Server Implementation
//!
//! This crate contains a distributed DHCP server implementation, with a custom protocol between nodes.
//!
//! For the protocol definition, look into the [`server::peer::message`] module.
//!
//! The server architecture comprises of threads, which use blocking operations to communicate over [`std::net::TcpStream`]s.
//! There are two threads per active peer, one for receiving messages and one for sending messages.
//! There is also a server logic thread, handling bookkeeping for the peer- and client events.
//!
//! For the communication thread implementation, look into the [`server::peer`] module.

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]

use std::thread::{self, JoinHandle};

pub mod config;
pub mod console;
pub mod dhcp;
pub mod server;

pub trait ThreadJoin: Sized {
    fn join(self) -> thread::Result<()>;

    fn name(&self) -> String;

    fn join_and_handle_panic(self) {
        let name = self.name();
        if let Err(msg) = self.join() {
            console::warning!("Thread {name} panicked");
            if let Some(msg) = msg
                .downcast_ref::<&str>()
                .map(ToString::to_string)
                .or(msg.downcast_ref::<String>().cloned())
            {
                console::warning!("{}", msg);
            }
        }
    }
}

impl ThreadJoin for JoinHandle<()> {
    fn join(self) -> thread::Result<()> {
        JoinHandle::join(self)
    }

    fn name(&self) -> String {
        self.thread().name().unwrap_or("<noname>").to_string()
    }
}
