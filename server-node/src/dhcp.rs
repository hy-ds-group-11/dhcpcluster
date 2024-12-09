use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    fmt::Display,
    net::Ipv4Addr,
    time::{Duration, SystemTime},
};

/// A DHCP Lease
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Lease {
    pub hardware_address: [u8; 6],    // Assume MAC address
    pub lease_address: Ipv4Addr,      // Assume IPv4 for now
    pub expiry_timestamp: SystemTime, // SystemTime as exact time is not critical, and we want a timestamp
}

/// IPv4 address pool to serve. `end` not included.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct DhcpService {
    pub start: u32,
    pub end: u32,
    pub lease_time: Duration,
    pub leases: Vec<Lease>,
    pub pending_leases: Vec<Lease>,
}

impl DhcpService {
    pub fn new_with_leases(start: u32, end: u32, lease_time: Duration, leases: &[Lease]) -> Self {
        Self {
            start,
            end,
            lease_time,
            leases: leases.into(),
            pending_leases: Vec::new(),
        }
    }

    pub fn new(start: u32, end: u32, lease_time: Duration) -> Self {
        Self::new_with_leases(start, end, lease_time, &[])
    }

    pub fn from_cidr(ip_address: Ipv4Addr, prefix_length: u32, lease_time: Duration) -> Self {
        let start = ip_address.to_bits();
        let end = start + 2_u32.pow(32 - prefix_length) - 1;
        Self {
            start,
            end,
            lease_time,
            leases: Vec::new(),
            pending_leases: Vec::new(),
        }
    }

    pub fn divide(&self, parts: u32, leases: &[Lease]) -> Vec<DhcpService> {
        let diff = self.end - self.start;
        let pool_size = diff / parts;
        let mut pools = Vec::new();

        for i in 0..parts {
            pools.push(DhcpService::new_with_leases(
                self.start + i * pool_size,
                self.start + (i + 1) * pool_size,
                self.lease_time,
                leases,
            ))
        }
        pools[parts as usize - 1].end += diff % parts;
        pools
    }

    fn fresh_timestamp(&self) -> SystemTime {
        SystemTime::now().checked_add(self.lease_time).unwrap()
    }

    pub fn add_lease(&mut self, new_lease: Lease) {
        // TODO: do some sanity checks
        if let Some(lease) = self.leases.iter_mut().find(|l| {
            l.hardware_address == new_lease.hardware_address
                && l.lease_address == new_lease.lease_address
        }) {
            lease.expiry_timestamp = new_lease.expiry_timestamp;
        } else {
            self.leases.push(new_lease);
        }
    }

    pub fn discover_lease(&mut self, mac_address: [u8; 6]) -> Option<Lease> {
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

        for ip_addr in self.start..self.end {
            // TODO: Use HashMap or other more efficient collection
            if self
                .leases
                .iter()
                .chain(self.pending_leases.iter())
                .any(|l| l.lease_address.to_bits() == ip_addr)
            {
                continue;
            }

            let lease = Lease {
                hardware_address: mac_address,
                lease_address: Ipv4Addr::from_bits(ip_addr),
                expiry_timestamp: self.fresh_timestamp(),
            };
            self.pending_leases.push(lease.clone());
            return Some(lease);
        }

        None
    }

    pub fn commit_lease(
        &mut self,
        mac_address: [u8; 6],
        ip_address: Ipv4Addr,
    ) -> Result<Lease, Box<dyn Error>> {
        let fresh_timestamp = self.fresh_timestamp();
        if let Some(pending_index) = self
            .pending_leases
            .iter()
            .position(|l| l.hardware_address == mac_address && l.lease_address == ip_address)
        {
            let mut lease = self.pending_leases.remove(pending_index);
            lease.expiry_timestamp = fresh_timestamp;
            self.leases.push(lease.clone());
            return Ok(lease);
        } else if let Some(lease) = self
            .leases
            .iter_mut()
            .find(|l| l.hardware_address == mac_address && l.lease_address == ip_address)
        {
            lease.expiry_timestamp = fresh_timestamp;
            return Ok(lease.clone());
        }
        Err("Can't find matching lease".into())
    }

    pub fn fmt_range(&self) -> String {
        format!(
            "{} - {}",
            Ipv4Addr::from_bits(self.start),
            Ipv4Addr::from_bits(self.end)
        )
    }
}

impl Display for Lease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "MAC {} has IP {}",
            self.hardware_address
                .iter()
                .map(|d| format!("{d:0>2X}"))
                .collect::<Vec<_>>()
                .join(":"),
            self.lease_address,
        )?;
        if let Ok(dur) = self.expiry_timestamp.duration_since(SystemTime::now()) {
            write!(f, ", expiring in {:<9.0?}", dur)?;
        }
        Ok(())
    }
}

impl Display for DhcpService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let range = self.fmt_range();
        writeln!(f, "{range}")?;
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
