use super::Event;
use crate::{config::Config, console};
use protocol::{
    CborRecvError, DhcpClientMessage, DhcpServerMessage, MacAddr, RecvCbor, RecvError, SendCbor,
};
use std::{
    io::ErrorKind,
    net::{Ipv4Addr, TcpListener, TcpStream},
    sync::{
        mpsc::{self, Sender},
        Arc,
    },
};
use thread_pool::ThreadPool;

pub fn listen_clients(
    listener: &TcpListener,
    config: &Arc<Config>,
    server_tx: &Sender<Event>,
    thread_pool: &ThreadPool,
) {
    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                let config = Arc::clone(config);
                let tx = server_tx.clone();
                thread_pool
                    .execute(move || serve_client(&mut stream, &config, &tx))
                    .expect("Thread pool cannot spawn threads");
            }
            Err(e) => console::error!(&e, "Accepting new client connection failed"),
        }
    }
}

fn serve_client(stream: &mut TcpStream, config: &Arc<Config>, server_tx: &Sender<Event>) {
    stream
        .set_read_timeout(Some(config.client_timeout))
        .expect("Can't set stream read timeout");
    let result = stream.recv();
    match result {
        Ok(DhcpClientMessage::Discover { mac_address }) => {
            handle_discover(stream, server_tx, mac_address);
        }
        Ok(DhcpClientMessage::Request { mac_address, ip }) => {
            handle_renew(stream, server_tx, mac_address, ip);
        }
        Err(e) => console::error!(&e, "Could not receive request from the client"),
    }
}

fn handle_discover(stream: &mut TcpStream, server_tx: &Sender<Event>, mac_address: MacAddr) {
    #[cfg(debug_assertions)]
    assert!(matches!(stream.read_timeout(), Ok(Some(_))));

    let (tx, rx) = mpsc::channel();
    server_tx
        .send(Event::LeaseRequest { mac_address, tx })
        .expect(
            "Invariant violated: server_rx has been dropped before joining client listener thread",
        );

    // Wait for main thread to process DHCP discover
    let Ok(offer) = rx.recv() else {
        if let Err(e) = stream.send(&DhcpServerMessage::Nack) {
            console::error!(&e, "Could not reply with Nack to the client");
        }
        return;
    };

    // Send main thread's offer to the client
    if let Err(e) = stream.send(&DhcpServerMessage::Offer(offer.into())) {
        console::error!(&e, "Could not send offer to the client");
        return;
    }

    // Listen for client's confirmation. TODO: with real DHCP, this goes elsewhere
    match stream.recv() {
        Ok(DhcpClientMessage::Request { mac_address, ip }) => {
            let (tx, rx) = mpsc::channel();
            server_tx
                    .send(Event::ConfirmRequest {
                        mac_address,
                        ip,
                        tx,
                    })
                    .expect("Invariant violated: server_rx has been dropped before joining client listener thread");
            // Wait for processing DHCP commit
            match rx.recv() {
                Ok(committed) => {
                    if committed {
                        if let Err(e) = stream.send(&DhcpServerMessage::Ack) {
                            console::error!(&e, "Could not send Ack to the client");
                        }
                    } else if let Err(e) = stream.send(&DhcpServerMessage::Nack) {
                        console::error!(&e, "Could not send Nack to the client");
                    }
                }
                Err(e) => {
                    console::error!(&e, "Could not commit lease");
                }
            }
        }
        Ok(message) => {
            console::warning!(
                "Client didn't follow protocol!\nExpected: Request, got: {message:?}"
            );
        }
        Err(ref error) => {
            if let CborRecvError::Receive(RecvError::Io(io_error)) = error {
                match io_error.kind() {
                    ErrorKind::WouldBlock | ErrorKind::TimedOut => {
                        console::error!(
                            error,
                            "Client didn't follow Discover with Request within {:?}",
                            stream.read_timeout().ok().flatten().expect(
                                "handle_discover called without read timeout set on stream"
                            ),
                        );
                    }
                    _ => console::error!(error, "Could not receive client reply"),
                }
            } else {
                console::error!(error, "Could not receive client reply");
            }
        }
    }
}

fn handle_renew(
    stream: &mut TcpStream,
    server_tx: &Sender<Event>,
    mac_address: MacAddr,
    ip: Ipv4Addr,
) {
    #[cfg(debug_assertions)]
    assert!(matches!(stream.read_timeout(), Ok(Some(_))));

    let (tx, rx) = mpsc::channel();
    server_tx
        .send(Event::ConfirmRequest {
            mac_address,
            ip,
            tx,
        })
        .expect(
            "Invariant violated: server_rx has been dropped before joining client listener thread",
        );
    if rx.recv().unwrap_or(false) {
        if let Err(e) = stream.send(&DhcpServerMessage::Ack) {
            console::error!(&e, "Could not send Ack to the client");
        }
    } else if let Err(e) = stream.send(&DhcpServerMessage::Nack) {
        console::error!(&e, "Could not send Nack to the client");
    }
}
