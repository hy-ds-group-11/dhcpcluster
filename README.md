# DHCP Cluster

A University of Helsinki **Distributed Systems** Course Project


https://github.com/user-attachments/assets/34c93146-e0de-4530-9dd1-dfa22c29f159


Fall 2024, Group 11

This repository contains a distributed DHCP server with shared state and a CLI for testing the server.
A DHCP Relay Agent, acting as a load balancer, has been designed but not implemented.

The software is currently _prototype quality_, it's not intended for production use.

## Documentation

[Design document :paperclip:](doc/design.md)

[Server documentation :books:](https://hy-ds-group-11.github.io/dhcpcluster/server_node/index.html)

[Client-server protocol documentation :books:](https://hy-ds-group-11.github.io/dhcpcluster/protocol/index.html)

### Weekly notes :notebook_with_decorative_cover:

| Week                | Summary                                                                           |
| :------------------ | :-------------------------------------------------------------------------------- |
| [45](doc/week45.md) | :busts_in_silhouette::speech_balloon: Initial planning and specification          |
| [46](doc/week46.md) | :closed_book: Design, documentation and server code                               |
| [47](doc/week47.md) | :white_check_mark: Server handshakes, testing and groundwork                      |
| [48](doc/week48.md) | :revolving_hearts: Peer heartbeat messages, thread shutdown                       |
| [49](doc/week49.md) | :1234::computer: Leader election, address pool distribution, client CLI           |
| [50](doc/week50.md) | :sparkles: DHCP functionality and client-server communication                     |
