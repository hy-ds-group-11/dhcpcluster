#![cfg_attr(coverage_nightly, feature(coverage_attribute))]

pub mod config;
mod message;
mod peer;

use crate::{config::Config, peer::Peer};
use message::Message;
use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    io::{Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream, ToSocketAddrs},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, SystemTime},
};

/// Describes all essential features that a message carrier such as [`std::net::TcpStream`] must have.
///
/// This trait is needed for mocking in tests.
pub trait MessageStream: Read + Write + Send + Sized {
    fn connect(addr: impl ToSocketAddrs) -> std::io::Result<Self>;

    fn set_read_timeout(&self, dur: Option<Duration>) -> std::io::Result<()>;

    fn clone(&self) -> Self;

    /// Convenience method which reads and deserializes a message
    fn receive_message(&mut self) -> Result<Message, ciborium::de::Error<std::io::Error>> {
        ciborium::from_reader::<Message, &mut Self>(self)
    }

    /// Convenience method which serializes a message and writes it
    fn send_message(
        &mut self,
        message: &Message,
    ) -> Result<(), ciborium::ser::Error<std::io::Error>> {
        ciborium::into_writer::<Message, &mut Self>(message, self)
    }
}

// Disable coverage for TcpStream, testing is done with [`test::MockStream`]
#[cfg_attr(coverage_nightly, coverage(off))]
impl MessageStream for TcpStream {
    fn connect(addr: impl ToSocketAddrs) -> std::io::Result<Self> {
        Self::connect(addr)
    }

    fn set_read_timeout(&self, dur: Option<Duration>) -> std::io::Result<()> {
        self.set_read_timeout(dur)
    }

    fn clone(&self) -> Self {
        self.try_clone().unwrap()
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
                    // When testing, store established streams for inspection
                    #[cfg(test)]
                    tests::push_stream(stream.clone());

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

    fn listen_nodes(&mut self, listener: TcpListener) {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => self.answer_handshake(stream),
                Err(e) => eprintln!("{e:?}"),
            }
        }
    }

    pub fn start_server(mut self, peer_listener: TcpListener) -> Result<(), Box<dyn Error>> {
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
    use std::{
        any::Any, cell::RefCell, collections::VecDeque, iter, net::SocketAddr, sync::MutexGuard,
    };

    struct MessageSequence {
        peer_addr: SocketAddr,
        messages: Vec<Message>,
    }

    struct MockStreamRaw {
        connected: Option<SocketAddr>,
        read_timeout: RefCell<Option<Duration>>,
        sent_data: VecDeque<u8>,
        reply_data: VecDeque<u8>,
    }

    pub struct MockStream {
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
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.inner().sent_data.write(buf)
        }

        fn flush(&mut self) -> std::io::Result<()> {
            self.inner().sent_data.flush()
        }
    }

    impl Read for MockStream {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.inner().reply_data.read(buf)
        }
    }

    impl MessageStream for MockStream {
        fn connect(addr: impl ToSocketAddrs) -> std::io::Result<Self> {
            // Parse the address
            let addr = addr.to_socket_addrs()?.next();

            // Check if a message sequence has been defined for the peer.
            // Construct a MockStream if defined, return ConnectionRefused, if not.
            // TODO for an even fancier solution, we should develop a mechanism
            // to support multiple message sequences per peer address, i.e.
            // if connect() is called repeatedly.
            with_replies(
                |replies| match replies.iter().find(|seq| Some(seq.peer_addr) == addr) {
                    Some(replies) => {
                        let mut stream = Self {
                            inner: Arc::new(Mutex::new(MockStreamRaw {
                                connected: addr,
                                read_timeout: RefCell::new(None),
                                sent_data: VecDeque::new(),
                                reply_data: VecDeque::new(),
                            })),
                        };
                        stream.send_replies(replies);
                        Ok(stream)
                    }
                    None => Err(std::io::Error::new(
                        std::io::ErrorKind::ConnectionRefused,
                        "Connection refused by peer",
                    )),
                },
            )
        }

        fn set_read_timeout(&self, dur: Option<Duration>) -> std::io::Result<()> {
            *self.inner().read_timeout.borrow_mut() = dur;
            Ok(())
        }

        fn clone(&self) -> Self {
            Self {
                inner: Arc::clone(&self.inner),
            }
        }
    }

    thread_local! {
        // This global is used for gathering MockStreams from the server initialization during tests
        static STREAMS: RefCell<Vec<MockStream>> = const {RefCell::new(Vec::new())};

        // In tests where the MockStreams should reply to the server,
        // you must push the desired message sequences to this global.
        // See [`push_replies`] for setting up message sequences.
        static REPLIES: RefCell<Vec<MessageSequence>> = const {RefCell::new(Vec::new())};
    }

    fn with_stream_of_peer<T>(key: SocketAddr, f: impl FnOnce(&MockStream) -> T) -> T {
        with_streams(|streams| -> T {
            let stream = streams
                .iter()
                .find(|stream| stream.inner().connected == Some(key))
                .expect("Tried to inspect stream with undefined peer in test");
            f(stream)
        })
    }

    fn with_streams<T>(f: impl FnOnce(&mut Vec<MockStream>) -> T) -> T {
        STREAMS.with_borrow_mut(f)
    }

    fn with_replies<T>(f: impl FnOnce(&mut Vec<MessageSequence>) -> T) -> T {
        REPLIES.with_borrow_mut(f)
    }

    pub fn push_stream(stream: impl MessageStream + 'static) {
        with_streams(|streams| {
            let stream: Box<dyn Any> = Box::new(stream.clone());
            let stream = *(stream
                .downcast::<MockStream>()
                .expect("Non-mock MessageStream used in tests"));
            streams.push(stream);
        })
    }

    fn push_replies(sequences: impl IntoIterator<Item = MessageSequence>) {
        with_replies(|replies| replies.extend(sequences));
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
            assert_eq!(stream.connected, Some(peer_1_addr));
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
            assert_eq!(stream.inner().connected, Some(peer_1_addr));
        });
        with_stream_of_peer(peer_2_addr, |stream| {
            assert_eq!(stream.get_messages(), vec![Message::Join(0)]);
            assert_eq!(stream.inner().connected, Some(peer_2_addr));
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
}
