#![cfg_attr(coverage_nightly, feature(coverage_attribute))]

pub mod config;
mod message;
mod peer;

use crate::{config::Config, peer::Peer};
use message::Message;
use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    io::{self, Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream, ToSocketAddrs},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, SystemTime},
};

// TODO move these traits to message module?

/// A trait for listening to incoming connections
pub trait MessageListener: Send + Sized {
    type Stream: MessageStream;

    fn bind(addr: impl ToSocketAddrs) -> io::Result<Self>;

    fn incoming(&self) -> impl Iterator<Item = io::Result<Self::Stream>>;
}

// Disable coverage for TcpListener, testing is done with [`test::MockListener`]
#[cfg_attr(coverage_nightly, coverage(off))]
impl MessageListener for TcpListener {
    type Stream = TcpStream;

    fn bind(addr: impl ToSocketAddrs) -> io::Result<Self> {
        Self::bind(addr)
    }

    fn incoming(&self) -> impl Iterator<Item = io::Result<Self::Stream>> {
        self.incoming()
    }
}

/// A trait for receiving and sending [`Message`]s.
pub trait MessageStream: Read + Write + Send + Sized {
    fn connect(addr: impl ToSocketAddrs) -> io::Result<Self>;

    fn set_read_timeout(&self, dur: Option<Duration>) -> io::Result<()>;

    fn try_clone(&self) -> io::Result<Self>;

    /// Reads and deserializes a message
    fn receive_message(&mut self) -> Result<Message, ciborium::de::Error<io::Error>> {
        ciborium::from_reader::<Message, &mut Self>(self)
    }

    /// Serializes and writes a message
    fn send_message(&mut self, message: &Message) -> Result<(), ciborium::ser::Error<io::Error>> {
        ciborium::into_writer::<Message, &mut Self>(message, self)
    }
}

// Disable coverage for TcpStream, testing is done with [`test::MockStream`]
#[cfg_attr(coverage_nightly, coverage(off))]
impl MessageStream for TcpStream {
    fn connect(addr: impl ToSocketAddrs) -> io::Result<Self> {
        Self::connect(addr)
    }

    fn set_read_timeout(&self, dur: Option<Duration>) -> io::Result<()> {
        self.set_read_timeout(dur)
    }

    fn try_clone(&self) -> io::Result<Self> {
        self.try_clone()
    }
}

type Leases = Arc<Mutex<Vec<Lease>>>;

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct Lease {
    hardware_address: [u8; 6],    // Assume MAC address
    lease_address: Ipv4Addr,      // Assume IPv4 for now
    expiry_timestamp: SystemTime, // SystemTime as exact time is not critical, and we want a timestamp
}

pub struct Cluster {
    server: Server,
    peers: Vec<Peer>,
    #[allow(dead_code)]
    coordinator_id: Option<u32>,
}

impl Cluster {
    /// Initialize cluster, and try connecting to peers.
    /// Failed handshakes are ignored, since they might be nodes that are starting later.
    /// After connecting to peers, `run_server(stream)` needs to be called.
    pub fn connect<S: MessageStream + 'static>(config: Config) -> Self {
        let mut peers = Vec::new();
        let server = Server::new(&config);

        for peer_address in config.peers {
            match S::connect(peer_address) {
                Ok(stream) => {
                    let result = Self::start_handshake(stream, config.id, &server);
                    match result {
                        Ok(peer) => peers.push(peer),
                        Err(e) => eprintln!("{e:?}"),
                    }
                }
                Err(e) => eprintln!("{e:?}"),
            }
        }

        Self {
            server,
            peers,
            coordinator_id: None,
        }
    }

    fn start_handshake(
        mut stream: impl MessageStream + 'static,
        id: u32,
        server: &Server,
    ) -> Result<Peer, Box<dyn Error>> {
        let result = stream.send_message(&Message::Join(id));
        match result {
            Ok(_) => {
                stream
                    .set_read_timeout(Some(Duration::from_millis(500)))
                    .unwrap();
                let message = stream.receive_message().unwrap();
                stream.set_read_timeout(None).unwrap();

                match dbg!(message) {
                    Message::JoinAck(peer_id) => {
                        Ok(Peer::new(stream, peer_id, Arc::clone(&server.leases)))
                    }
                    _ => panic!("Peer responded to Join with something other than JoinAck"),
                }
            }
            Err(e) => {
                eprintln!("{e:?}");
                Err(e.into())
            }
        }
    }

    fn answer_handshake(&mut self, mut stream: impl MessageStream + 'static) {
        stream
            .set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        let message = stream.receive_message().unwrap();
        stream.set_read_timeout(None).unwrap();

        match message {
            Message::Join(id) => {
                let result = stream.send_message(&Message::JoinAck(self.server.id));
                match result {
                    Ok(_) => {
                        self.peers
                            .push(Peer::new(stream, id, Arc::clone(&self.server.leases)))
                    }
                    Err(e) => eprintln!("{e:?}"),
                }
            }
            _ => panic!("First message of peer wasn't Join"),
        }
    }

    fn listen_nodes(&mut self, listener: impl MessageListener + 'static) {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => self.answer_handshake(stream),
                Err(e) => eprintln!("{e:?}"),
            }
        }
    }

    pub fn start_server(
        mut self,
        peer_listener: impl MessageListener + 'static,
    ) -> Result<(), Box<dyn Error>> {
        let peer_listener_thread = thread::spawn(move || self.listen_nodes(peer_listener));

        // TODO

        peer_listener_thread.join().map_err(|e| {
            format!(
                "Node listener thread panicked, Err: {:?}",
                e.downcast_ref::<&str>()
            )
            .into()
        })
    }
}

pub struct Server {
    id: u32,
    leases: Leases,
}

impl Server {
    pub fn new(config: &Config) -> Self {
        Self {
            id: config.id,
            leases: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::VecDeque, io, iter, net::SocketAddr, sync::MutexGuard};

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
                let mut stream = MockStream {
                    peer_addr: seq.peer_addr,
                    inner: Default::default(),
                };
                stream.send_replies(&seq);
                streams.push(stream);
            }
        })
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
        let peer_1_addr = "123.123.123.123:8909".parse().unwrap();

        push_replies(vec![MessageSequence {
            peer_addr: peer_1_addr,
            messages: vec![Message::JoinAck(1)],
        }]);

        let config = Config {
            address_private: ([0u8; 4], 1234).into(),
            peers: vec![peer_1_addr],
            id: 0,
        };
        let cluster = Cluster::connect::<MockStream>(config);

        assert!(cluster.coordinator_id.is_none());
        assert!(cluster.server.leases.lock().unwrap().is_empty());
        assert_eq!(cluster.server.id, 0);
        with_streams(|streams| {
            assert_eq!(streams[0].get_messages(), vec![Message::Join(0)]);
            assert_eq!(streams.len(), 1);
            let stream = streams[0].inner();
            assert!(stream.connected);
        });
    }

    #[test]
    fn cluster_connect_to_two_peers() {
        let peer_1_addr = "123.123.123.123:8909".parse().unwrap();
        let peer_2_addr = "1.1.1.1:22".parse().unwrap();

        push_replies(vec![
            MessageSequence {
                peer_addr: peer_1_addr,
                messages: vec![Message::JoinAck(1)],
            },
            MessageSequence {
                peer_addr: peer_2_addr,
                messages: vec![Message::JoinAck(2)],
            },
        ]);

        let config = Config {
            address_private: ([0u8; 4], 1234).into(),
            peers: vec![peer_1_addr, peer_2_addr],
            id: 0,
        };
        let cluster = Cluster::connect::<MockStream>(config);

        assert!(cluster.coordinator_id.is_none());
        assert!(cluster.server.leases.lock().unwrap().is_empty());
        assert_eq!(cluster.server.id, 0);
        with_streams(|streams| {
            assert_eq!(streams.len(), 2);
        });
        with_stream_of_peer(peer_1_addr, |stream| {
            assert_eq!(stream.get_messages(), vec![Message::Join(0)]);
            assert!(stream.inner().connected);
        });
        with_stream_of_peer(peer_2_addr, |stream| {
            assert_eq!(stream.get_messages(), vec![Message::Join(0)]);
            assert!(stream.inner().connected);
        });
    }

    // TODO refactor repeated test logic to function

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
        let mut seq1 = MockStream {
            peer_addr,
            inner: Default::default(),
        };
        seq1.send_replies(&MessageSequence {
            peer_addr,
            messages: vec![],
        });
        let mut listener = MockListener::bind(server_addr).unwrap();
        listener.push_streams(vec![(Duration::from_millis(100), Ok(seq1))].into_iter());

        let config = Config {
            address_private: server_addr,
            peers: vec![peer_addr],
            id: 0,
        };
        let cluster = Cluster::connect::<MockStream>(config);
        cluster.start_server(listener).unwrap();
    }

    #[test]
    fn cluster_handle_incoming_handshake_from_one_peer() {
        let server_addr = "127.0.0.1:1234".parse().unwrap();
        let peer_addr = "127.0.0.2:4321".parse().unwrap();
        let mut seq1 = MockStream {
            peer_addr,
            inner: Default::default(),
        };
        seq1.send_replies(&MessageSequence {
            peer_addr: server_addr,
            messages: vec![Message::Join(1)],
        });
        let mut listener = MockListener::bind(server_addr).unwrap();
        listener.push_streams(vec![(Duration::from_millis(100), Ok(seq1))].into_iter());

        let config = Config {
            address_private: server_addr,
            peers: vec![peer_addr],
            id: 0,
        };
        let cluster = Cluster::connect::<MockStream>(config);
        cluster.start_server(listener.clone()).unwrap();

        let mut handled = listener.handled_streams.lock().unwrap();
        let mut server_replies = handled.pop().unwrap().get_messages();
        assert_eq!(server_replies.len(), 1);
        assert_eq!(server_replies.pop().unwrap(), Message::JoinAck(0));
    }
}
