use serde::{Deserialize, Serialize};
use std::{net::Ipv4Addr, time::SystemTime};

/// A DHCP Lease
#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct Lease {
    hardware_address: [u8; 6],    // Assume MAC address
    lease_address: Ipv4Addr,      // Assume IPv4 for now
    expiry_timestamp: SystemTime, // SystemTime as exact time is not critical, and we want a timestamp
}

/// IPv4 address pool to serve. `end` not included.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct DhcpPool {
    start: u32,
    end: u32,
}

impl DhcpPool {
    pub fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }

    pub fn from_cidr(ip_address: Ipv4Addr, prefix_length: u32) -> Self {
        let start = ip_address.to_bits();
        let end = start + 2_u32.pow(32 - prefix_length) - 1;
        Self { start, end }
    }

    pub fn divide(&self, parts: u32) -> Vec<DhcpPool> {
        let diff = self.end - self.start;
        let pool_size = diff / parts;
        let mut pools = Vec::new();

        for i in 0..parts {
            pools.push(DhcpPool::new(
                self.start + i * pool_size,
                self.start + (i + 1) * pool_size,
            ))
        }
        pools[parts as usize - 1].end += diff % parts;
        pools
    }
}
