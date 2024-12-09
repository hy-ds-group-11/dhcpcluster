# DHCP Cluster

A University of Helsinki **Distributed Systems** Course Project

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
| [45](doc/week45.md) | Initial planning and specification :busts_in_silhouette::speech_balloon:          |
| [46](doc/week46.md) | Design, documentation :closed_book: and server code :rocket:                      |
| [47](doc/week47.md) | Server handshakes :raised_hands:, testing :white_check_mark: and groundwork       |
| [48](doc/week48.md) | Peer heartbeat messages :revolving_hearts:, thread shutdown :arrow_heading_down:  |
| [49](doc/week49.md) | Leader election :trophy:, address pool distribution :1234:, client CLI :computer: |
| [50](doc/week50.md) | Under construction :construction:                                                 |
