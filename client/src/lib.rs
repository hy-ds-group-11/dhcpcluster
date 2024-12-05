use std::net::Ipv4Addr;

use dhcp_message::DhcpMessage;

pub fn send_discover(mac_address: Vec<u8>) -> DhcpMessage {
    let message = DhcpMessage::Discover { mac_address };
    let _data = message.encode();

    // TODO: Send message and get response
    unimplemented!()
}

pub fn send_request(mac_address: Vec<u8>, ip: Ipv4Addr) -> DhcpMessage {
    let message = DhcpMessage::Request { mac_address, ip };
    let _data = message.encode();

    // TODO: Send message and get response
    unimplemented!()
}
