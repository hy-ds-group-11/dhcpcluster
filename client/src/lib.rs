use std::net::Ipv4Addr;

use protocol::DhcpServerMessage;

#[allow(unused)]
pub fn get_offer(mac_address: [u8; 6]) -> Option<DhcpServerMessage> {
    // TODO: Send message and get response
    unimplemented!()
}

#[allow(unused)]
pub fn get_ack(mac_address: Vec<u8>, ip: Ipv4Addr) -> Option<DhcpServerMessage> {
    // TODO: Send message and get response
    unimplemented!()
}
