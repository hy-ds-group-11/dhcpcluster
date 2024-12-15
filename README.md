# DHCP Cluster

A University of Helsinki **Distributed Systems** Course Project


https://github.com/user-attachments/assets/bd09bf65-d828-42df-9e15-c071cbb4e9fd


![Demo video](doc/demo.mp4)

Fall 2024, Group 11

This repository contains a distributed DHCP server with shared state and a CLI for testing the server.
A DHCP Relay Agent, acting as a load balancer, has been designed but **not implemented**.

The software is currently **prototype quality**, it's not intended for production use.

### Features:
- Written in Rust, blazingly fast :fire:
- Multithreaded :rocket:
- Shared state (all nodes know all leases)
- Fault tolerance via redundancy
- Leader election with bully algorithm
  - Leader decides which node gets to give offers from which part of the address pool
- Custom protocol, **does not actually implement DHCP** ([yet](https://github.com/hy-ds-group-11/dhcpcluster/issues/4))
- Probably not completely reliable, but it's just DHCP
- No `async` (for no particular reason)

## Documentation

[Design document :paperclip:](doc/design.md)

[Server documentation :books:](https://hy-ds-group-11.github.io/dhcpcluster/server_node/index.html)

### Weekly notes :notebook_with_decorative_cover:

| Week                | Summary                                                                           |
| :------------------ | :-------------------------------------------------------------------------------- |
| [45](doc/week45.md) | :busts_in_silhouette::speech_balloon: Initial planning and specification          |
| [46](doc/week46.md) | :closed_book: Design, documentation and server code                               |
| [47](doc/week47.md) | :white_check_mark: Server handshakes, testing and groundwork                      |
| [48](doc/week48.md) | :revolving_hearts: Peer heartbeat messages, thread shutdown                       |
| [49](doc/week49.md) | :1234::computer: Leader election, address pool distribution, client CLI           |
| [50](doc/week50.md) | :sparkles: DHCP functionality and client-server communication                     |

## Downloads :floppy_disk:

[See Releases](https://github.com/hy-ds-group-11/dhcpcluster/releases)
