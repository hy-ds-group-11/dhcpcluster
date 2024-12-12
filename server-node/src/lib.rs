//! # DHCP Cluster - Server Implementation
//!
//! This crate contains a distributed DHCP server implementation, with a custom protocol between nodes.
//!
//! For the protocol definition, look into the [`message`] module.
//!
//! The server architecture comprises of threads, which use blocking operations to communicate over [`std::net::TcpStream`]s.
//! There are two threads per active peer, one for receiving messages and one for sending messages.
//! There is also a server logic thread, handling bookkeeping for the peer- and client events.
//!
//! For the communication thread implementation, look into the [`peer`] module.

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]

use std::thread::{self, JoinHandle};

pub mod config;
pub mod console;
pub mod dhcp;
pub mod message;
pub mod peer;
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
                .map(|s| s.to_string())
                .or(msg.downcast_ref::<String>().cloned())
            {
                console::warning!("{}", msg);
            }
        }
    }
}

impl ThreadJoin for JoinHandle<()> {
    fn join(self) -> thread::Result<()> {
        self.join()
    }

    fn name(&self) -> String {
        self.thread().name().unwrap_or("<noname>").to_string()
    }
}
