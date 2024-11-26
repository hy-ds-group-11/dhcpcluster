//! This is the main tests module for the server, with mock implementations for
//! [`TcpStream`] and [`TcpListener`].

#![cfg(test)]

use super::*;
use std::{
    cell::RefCell,
    collections::VecDeque,
    io::{self, Read, Write},
    iter,
    net::{SocketAddr, SocketAddrV4, ToSocketAddrs},
    sync::MutexGuard,
};

struct MessageSequence {
    peer_addr: SocketAddr,
    messages: Vec<Message>,
}

type IncomingConnection = (Duration, Result<MockStream, io::ErrorKind>);

pub struct MockListener {
    #[allow(dead_code)]
    local_addr: SocketAddr,
    incoming_streams: Arc<Mutex<Vec<IncomingConnection>>>,
    handled_streams: Arc<Mutex<Vec<MockStream>>>,
}

impl MockListener {
    fn push_streams(
        &mut self,
        streams: impl Iterator<Item = (Duration, Result<MockStream, io::ErrorKind>)>,
    ) {
        self.incoming_streams.lock().unwrap().extend(streams);
    }
}

impl Clone for MockListener {
    fn clone(&self) -> Self {
        Self {
            local_addr: self.local_addr,
            incoming_streams: Arc::clone(&self.incoming_streams),
            handled_streams: Arc::clone(&self.handled_streams),
        }
    }
}

impl MessageListener for MockListener {
    type Stream = MockStream;

    fn bind(addr: impl ToSocketAddrs) -> io::Result<Self> {
        let local_addr = addr.to_socket_addrs()?.next().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::AddrNotAvailable, "No addresses found")
        })?;
        Ok(Self {
            local_addr,
            incoming_streams: Arc::new(Mutex::new(Vec::new())),
            handled_streams: Arc::new(Mutex::new(Vec::new())),
        })
    }

    fn incoming(&self) -> impl Iterator<Item = io::Result<Self::Stream>> {
        let streams = Arc::clone(&self.incoming_streams);
        std::iter::from_fn(move || {
            let mut streams = streams.lock().unwrap();
            if let Some((delay, result)) = streams.pop() {
                thread::sleep(delay);
                if let Ok(stream) = &result {
                    self.handled_streams.lock().unwrap().push(stream.clone());
                }
                Some(result.map_err(|kind| io::Error::new(kind, "Mock error")))
            } else {
                None
            }
        })
    }
}

#[derive(Default)]
struct MockStreamRaw {
    connected: bool,
    read_timeout: RefCell<Option<Duration>>,
    sent_data: VecDeque<u8>,
    reply_data: VecDeque<u8>,
}

pub struct MockStream {
    peer_addr: SocketAddr,
    inner: Arc<Mutex<MockStreamRaw>>,
}

impl MockStream {
    fn new(seq: &MessageSequence) -> Self {
        let mut stream = Self {
            peer_addr: seq.peer_addr,
            inner: Default::default(),
        };
        stream.send_replies(seq);
        stream
    }

    fn inner(&self) -> MutexGuard<MockStreamRaw> {
        self.inner.lock().unwrap()
    }

    fn send_replies(&mut self, sequence: &MessageSequence) {
        for message in &sequence.messages {
            ciborium::into_writer::<Message, &mut VecDeque<u8>>(
                message,
                &mut self.inner().reply_data,
            )
            .unwrap();
        }
    }

    fn get_messages(&self) -> Vec<Message> {
        let mut inner = self.inner();
        iter::from_fn(|| ciborium::from_reader(&mut inner.sent_data).ok()).collect()
    }
}

impl Write for MockStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner().sent_data.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner().sent_data.flush()
    }
}

impl Read for MockStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner().reply_data.read(buf)
    }
}

impl MessageStream for MockStream {
    fn connect(addr: impl ToSocketAddrs) -> io::Result<Self> {
        // Parse the address
        let addr = addr.to_socket_addrs()?.next().unwrap();

        // Check if a stream has been defined for the peer.
        // Return the MockStream if defined, return ConnectionRefused, if not.
        // TODO for an even fancier solution, we should develop a mechanism
        // to support multiple message sequences per peer address, i.e.
        // when connect() is called repeatedly.
        with_streams(
            |streams| match streams.iter().find(|stream| stream.peer_addr == addr) {
                Some(stream) => {
                    assert!(!stream.inner().connected);
                    stream.inner().connected = true;
                    Ok(stream.clone())
                }
                None => Err(io::Error::new(
                    io::ErrorKind::ConnectionRefused,
                    "Connection refused by peer",
                )),
            },
        )
    }

    fn set_read_timeout(&self, dur: Option<Duration>) -> io::Result<()> {
        *self.inner().read_timeout.borrow_mut() = dur;
        Ok(())
    }

    fn try_clone(&self) -> Result<MockStream, io::Error> {
        Ok(Self {
            peer_addr: self.peer_addr,
            inner: Arc::clone(&self.inner),
        })
    }
}

impl Clone for MockStream {
    fn clone(&self) -> Self {
        MessageStream::try_clone(self).unwrap()
    }
}

thread_local! {
    // This global is used for gathering MockStreams from the server initialization during tests
    // See [`push_replies`] for setting up message sequences to the streams.
    static STREAMS: RefCell<Vec<MockStream>> = const {RefCell::new(Vec::new())};
}

fn with_stream_of_peer<T>(key: SocketAddr, f: impl FnOnce(&MockStream) -> T) -> T {
    with_streams(|streams| -> T {
        let stream = streams
            .iter()
            .find(|stream| stream.peer_addr == key)
            .expect("Tried to inspect stream with undefined peer in test");
        f(stream)
    })
}

fn with_streams<T>(f: impl FnOnce(&mut Vec<MockStream>) -> T) -> T {
    STREAMS.with_borrow_mut(f)
}

fn push_replies(sequences: impl IntoIterator<Item = MessageSequence>) {
    with_streams(|streams| {
        for seq in sequences {
            streams.push(MockStream::new(&seq));
        }
    })
}

fn generate_localhost_addresses(n: u32) -> Vec<SocketAddr> {
    (0..n)
        .into_iter()
        .map(|nth| {
            let ip = Ipv4Addr::from_bits(std::net::Ipv4Addr::LOCALHOST.to_bits() + nth);
            SocketAddr::V4(SocketAddrV4::new(ip, 4321))
        })
        .collect()
}

fn cluster_connect_to_n_peers(n: u32) {
    let peer_addrs = generate_localhost_addresses(n);
    for (id, peer_addr) in peer_addrs.iter().enumerate() {
        push_replies(vec![MessageSequence {
            peer_addr: *peer_addr,
            messages: vec![Message::JoinAck(id as u32 + 1)],
        }]);
    }

    let config = Config {
        address_private: ([0u8; 4], 1234).into(),
        peers: peer_addrs.clone(),
        id: 0,
    };
    let cluster = Cluster::connect::<MockStream>(config);

    assert!(cluster.coordinator_id.is_none());
    assert!(cluster.server.leases.lock().unwrap().is_empty());
    assert_eq!(cluster.server.id, 0);
    with_streams(|streams| {
        assert_eq!(streams.len(), n.try_into().unwrap());
    });
    for peer_addr in peer_addrs {
        with_stream_of_peer(peer_addr, |stream| {
            assert_eq!(stream.get_messages(), vec![Message::Join(0)]);
            assert!(stream.inner().connected);
        });
    }
}

#[test]
fn cluster_connect_to_no_peers() {
    let config = Config {
        address_private: ([0u8; 4], 1234).into(),
        peers: vec![],
        id: 0,
    };
    let cluster = Cluster::connect::<MockStream>(config);
    assert!(cluster.peers.is_empty());
    assert!(cluster.coordinator_id.is_none());
    assert!(cluster.server.leases.lock().unwrap().is_empty());
    assert_eq!(cluster.server.id, 0);
    with_streams(|streams| assert!(streams.is_empty()));
}

#[test]
fn cluster_connect_to_one_peer() {
    cluster_connect_to_n_peers(1);
}

#[test]
fn cluster_connect_to_two_peers() {
    cluster_connect_to_n_peers(2);
}

#[test]
fn cluster_connect_to_thousand_peers() {
    cluster_connect_to_n_peers(1024);
}

#[test]
fn cluster_connect_to_offline_peer() {
    let peer_1_addr = "123.123.123.123:8909".parse().unwrap();

    // push_replies omitted

    let config = Config {
        address_private: ([0u8; 4], 1234).into(),
        peers: vec![peer_1_addr],
        id: 0,
    };
    let cluster = Cluster::connect::<MockStream>(config);

    assert!(cluster.peers.is_empty());
    with_streams(|streams| assert!(streams.is_empty()));
}

#[test]
#[should_panic(expected = "Node listener thread panicked")]
fn cluster_handle_incoming_peer_no_messages() {
    let server_addr = "127.0.0.1:1234".parse().unwrap();
    let peer_addr = "127.0.0.2:4321".parse().unwrap();
    let stream1 = MockStream::new(&MessageSequence {
        peer_addr,
        messages: vec![],
    });
    let mut listener = MockListener::bind(server_addr).unwrap();
    listener.push_streams(vec![(Duration::from_millis(100), Ok(stream1))].into_iter());

    let config = Config {
        address_private: server_addr,
        peers: vec![peer_addr],
        id: 0,
    };
    let cluster = Cluster::connect::<MockStream>(config);
    cluster.start_server(listener).unwrap();
}

fn cluster_handle_incoming_handshake_from_n_peers(n: u32) {
    let server_addr = "127.0.0.1:1234".parse().unwrap();
    let peer_addresses = generate_localhost_addresses(n);
    let streams: Vec<MockStream> = peer_addresses
        .iter()
        .enumerate()
        .map(|(id, peer_addr)| {
            MockStream::new(&MessageSequence {
                peer_addr: *peer_addr,
                messages: vec![Message::Join(id as u32 + 1)],
            })
        })
        .collect();
    let mut listener = MockListener::bind(server_addr).unwrap();
    listener.push_streams(
        streams
            .into_iter()
            .map(|stream| (Duration::from_millis(0), Ok(stream))),
    );
    let config = Config {
        address_private: server_addr,
        peers: peer_addresses,
        id: 0,
    };
    let cluster = Cluster::connect::<MockStream>(config);
    cluster.start_server(listener.clone()).unwrap();

    let handled = listener.handled_streams.lock().unwrap();
    assert_eq!(handled.len(), n.try_into().unwrap());
    for server_replies in handled.iter().map(|stream| stream.get_messages()) {
        assert_eq!(server_replies.len(), 1);
        assert_eq!(server_replies[0], Message::JoinAck(0));
    }
}

#[test]
fn cluster_handle_incoming_handshake_from_one_peer() {
    cluster_handle_incoming_handshake_from_n_peers(1);
}

#[test]
fn cluster_handle_incoming_handshake_from_thousand_peers() {
    cluster_handle_incoming_handshake_from_n_peers(1024);
}
