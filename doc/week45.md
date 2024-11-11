## 2024-11-05

So far the group has settled on these initial (and aspirational) implementation details and architectural choices:

- [Rust](https://rust-lang.org)
- TCP for internal communication
- Identical nodes
- Address lease pool dynamically allocated between nodes
- Nodes elect a leader who decides the allocation of lease pool between nodes
- Node failure is detected by any node and handled by system re-electing leader
  - Holding a new election works in case of leader failure
- Custom DHCP Relay Agent that is aware of the distribution ("load balancer")
- (Ideally would) expose DHCP interface to users with distribution transparency
  - Low priority

## 2024-11-07

Added intial Cargo workspace structure.
