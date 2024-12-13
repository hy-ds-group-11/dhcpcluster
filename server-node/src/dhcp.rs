use protocol::MacAddr;
use serde::{Deserialize, Serialize};
use std::{
    fmt::Display,
    net::Ipv4Addr,
    time::{Duration, SystemTime},
};
use thiserror::Error;

/// A DHCP Lease
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Lease {
    pub hardware_address: MacAddr,    // Assume MAC address
    pub address: Ipv4Addr,            // Assume IPv4 for now
    pub expiry_timestamp: SystemTime, // SystemTime as exact time is not critical, and we want a timestamp
}

/// IPv4 address pool to serve. `end` not included.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Ipv4Range {
    start: u32,
    end: u32,
}

impl Display for Ipv4Range {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} - {}",
            Ipv4Addr::from_bits(self.start),
            Ipv4Addr::from_bits(self.end)
        )
    }
}

impl Ipv4Range {
    #[must_use]
    pub fn new(start: u32, end: u32) -> Self {
        assert!(start <= end);
        Self { start, end }
    }

    #[must_use]
    pub fn start(&self) -> Ipv4Addr {
        Ipv4Addr::from_bits(self.start)
    }

    #[must_use]
    pub fn end(&self) -> Ipv4Addr {
        Ipv4Addr::from_bits(self.end)
    }

    #[must_use]
    pub fn len(&self) -> u32 {
        self.end - self.start
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    pub fn iter_starting_at(&self, at: Ipv4Addr) -> impl Iterator<Item = Ipv4Addr> {
        let at = at.to_bits();
        (at..self.end)
            .chain(self.start..at)
            .map(Ipv4Addr::from_bits)
    }

    #[must_use]
    pub fn from_cidr(ip_address: Ipv4Addr, prefix_length: u32) -> Self {
        let start = ip_address.to_bits();
        let end = start + 2_u32.pow(32 - prefix_length) - 1;
        Self::new(start, end)
    }

    #[must_use]
    pub fn divide(&self, parts: u32) -> Vec<Self> {
        let pool_size = self.len() / parts;
        let mut pools = Vec::with_capacity(parts as usize);

        for i in 0..parts {
            pools.push(Self::new(
                self.start + i * pool_size,
                self.start + (i + 1) * pool_size,
            ));
        }
        pools[parts as usize - 1].end += self.len() % parts;

        pools
    }
}

#[derive(Error, Debug)]
pub enum CommitError {
    #[error("No lease found for ({mac_addr}, {ip_addr})")]
    LeaseNotFound {
        mac_addr: MacAddr,
        ip_addr: Ipv4Addr,
    },
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Service {
    pub pool: Ipv4Range,
    pub lease_time: Duration,
    pub leases: Vec<Lease>,
    pub pending_leases: Vec<Lease>,
    next_candidate_address: Ipv4Addr,
}

impl Service {
    #[must_use]
    pub fn new_with_leases(pool: Ipv4Range, lease_time: Duration, leases: &[Lease]) -> Self {
        let next_candidate_address = pool.start();
        Self {
            pool,
            lease_time,
            leases: leases.into(),
            pending_leases: Vec::new(),
            next_candidate_address,
        }
    }

    #[must_use]
    pub fn new(pool: Ipv4Range, lease_time: Duration) -> Self {
        Self::new_with_leases(pool, lease_time, &[])
    }

    pub fn set_pool(&mut self, pool: Ipv4Range) {
        self.pool = pool;
    }

    pub fn add_lease(&mut self, new_lease: Lease) {
        // TODO: do some sanity checks
        if let Some(lease) = self.leases.iter_mut().find(|l| {
            l.hardware_address == new_lease.hardware_address && l.address == new_lease.address
        }) {
            lease.expiry_timestamp = new_lease.expiry_timestamp;
        } else {
            self.leases.push(new_lease);
        }
    }

    pub fn discover_lease(&mut self, mac_address: MacAddr) -> Option<Lease> {
        // Check if MAC Address already has an IP
        if let Some(lease) = self
            .leases
            .iter_mut()
            .find(|l| l.hardware_address == mac_address)
        {
            return Some(lease.clone());
        }

        // Prune leases
        let now = SystemTime::now();
        self.leases.retain(|l| l.expiry_timestamp > now);
        self.pending_leases.retain(|l| l.expiry_timestamp > now);

        // First check a range starting from the last address given
        // Fall back to checking from the start of the full range
        for ip_addr in self.pool.iter_starting_at(self.next_candidate_address) {
            // TODO: Use HashMap or other more efficient collection
            if self
                .leases
                .iter()
                .chain(self.pending_leases.iter())
                .any(|l| l.address == ip_addr)
            {
                continue;
            }

            // This won't overflow the range end
            // because the maximum value of ip_addr is range.end - 1
            self.next_candidate_address = Ipv4Addr::from_bits(ip_addr.to_bits() + 1);

            let lease = Lease {
                hardware_address: mac_address,
                address: ip_addr,
                expiry_timestamp: self.fresh_timestamp(),
            };
            self.pending_leases.push(lease.clone());
            return Some(lease);
        }

        None
    }

    pub fn commit_lease(
        &mut self,
        mac_addr: MacAddr,
        ip_addr: Ipv4Addr,
    ) -> Result<Lease, CommitError> {
        let fresh_timestamp = self.fresh_timestamp();
        if let Some(pending_index) = self
            .pending_leases
            .iter()
            .position(|l| l.hardware_address == mac_addr && l.address == ip_addr)
        {
            let mut lease = self.pending_leases.remove(pending_index);
            lease.expiry_timestamp = fresh_timestamp;
            self.leases.push(lease.clone());
            return Ok(lease);
        } else if let Some(lease) = self
            .leases
            .iter_mut()
            .find(|l| l.hardware_address == mac_addr && l.address == ip_addr)
        {
            lease.expiry_timestamp = fresh_timestamp;
            return Ok(lease.clone());
        }
        Err(CommitError::LeaseNotFound { mac_addr, ip_addr })
    }

    fn fresh_timestamp(&self) -> SystemTime {
        SystemTime::now()
            .checked_add(self.lease_time)
            .expect("Time overflow")
    }
}

impl Display for Lease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MAC {} has IP {}", self.hardware_address, self.address,)?;
        if let Ok(dur) = self.expiry_timestamp.duration_since(SystemTime::now()) {
            write!(f, ", expiring in {dur:<9.0?}")?;
        }
        Ok(())
    }
}

impl Display for Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{}", self.pool)?;
        if !self.leases.is_empty() {
            writeln!(f)?;
        }
        for lease in self.leases.iter().take(3) {
            writeln!(f, "{lease}")?;
        }
        if self.leases.len() > 3 {
            let remaining = self.leases.len() - 3;
            writeln!(f, "And {remaining} more")?;
        }
        Ok(())
    }
}
