use std::net::Ipv4Addr;

use dhcp_message::{DhcpClientMessage, DhcpServerMessage};

pub fn get_offer(mac_address: Vec<u8>) -> Option<DhcpServerMessage> {
    let message = DhcpClientMessage::Discover { mac_address };
    let _data = message.encode();

    // TODO: Send message and get response
    unimplemented!()
}

pub fn get_ack(mac_address: Vec<u8>, ip: Ipv4Addr) -> Option<DhcpServerMessage> {
    let message = DhcpClientMessage::Request { mac_address, ip };
    let _data = message.encode();

    // TODO: Send message and get response
    unimplemented!()
}
