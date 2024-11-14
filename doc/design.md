# DHCP Cluster (Group 11)

Project by: Lauri Gustafsson, Juho Röyskö, Vili Sinervä and Simo Soini

## Overview

The project idea is a distributed DHCP cluster, which would provide fault tolerance and geographic distribution as compared to a general DHCP server.
All nodes will run the same software, with leader election to choose a node to make centralized decisions when needed.
The cluster will have a single configuration for the DHCP ranges, with subsets assigned to individual nodes dynamically.
The leases are a shared state between all nodes, which allows the cluster to recover from the loss of a node.
Choosing the best node to connect to will be done by a custom DHCP relay agent.

## Diagram of nodes and client

Primary messages listed for each connection. Leader also sends the range allocations to each node.

```mermaid
flowchart TD
    A(Client) <-->|Ping/DHCP| B(Node 1)
    B <-->|Elections/Leases| C(Node 2)
    A <-->|Ping/DHCP| C
    C <-->|Elections/Leases| D(Node 3
    leader
    )
    A <-->|Ping/DHCP| D
    B <-->|Elections/Leases| D
```

## Design details
The system is designed to use odd number of nodes to guarantee that majority can always be reached.
The demo system will have 3 nodes, which means that the cluster can continue working with one crashed node.
The elected leader node has the responsibility to make sure that the majority of nodes can communicate with each other.
If not, the cluster should not work, as the leader could be the only one who cannot communicate with others.
In this case, the other nodes have already selected a new leader, and this mechanism makes sure that there can never be two competing parts of the cluster working.

The leader also divides the allocated IP-address pool between the nodes.
This division has to be redone if the amount of active nodes changes, or if some nodes are running low on available addresses.
By dividing the pool between the nodes, there is no need for complex communication within the cluster for every single lease offered.
Each node can just offer a lease from their share of the pool, and then sync the lease to the other nodes after it is finished.
Every node should have a consistent lease table state.
Then any node can renew a lease even if it was initially given out by some other node in the cluster.

Configuration syncing is also done by the leader node.
The nodes need to have the other nodes included in the configuration to know how to contact them and to know how many nodes there should be in total.
Static leases are also a thing that we might want to be included in the configuration system.

The client is the one distributing the queries to the nodes according to latency.
For testing and demo purposes, the test client will be able to generate DHCP traffic, and distribute it in different ways.

## Messages

The order of the messages is not critical for this application. Each lease is granted and updated by a single node, so there is no concurrent access. The timescales that DHCP operates at are in the range of several minutes at least. The bully algorithm also works without a strict ordering of messages. We also don't need to worry about clocks being out of sync, as long as all nodes are using an NTP server. The exactness of the expiry timestamps is not critical.

### Leader election (bully algorithm)

These messages are very simple and don't have much of a syntax. They are sent between all nodes as necessary. The meanings of the messages are as they are in the generic bully algorithm.
- Election
- Okay
- Coordinator

### Client-Server

- PING (ICMP echo) - used for RTT estimation, which the client uses to select the node to connect to. Use the existing implementation, instead of writing our own.
- DHCPDISCOVER, DHCPOFFER, DHCPREQUEST, DHCPACK, DHCPNACK - these are standard DHCP messages, so are not explained further here. Might use an existing implementation/library.

### Custom

The format of these message is a serialized binary representation of Rust enums/structs.
- Add/Update lease messages (two separate message types) sent every time a lease is added or updated. If one is not sent at least every X seconds, an empty one is sent as a heartbeat. The heartbeat is used to detect missing nodes. These messages are multicast to all nodes. We expect traffic to be low enough that sending each update individually should not be a problem. This includes at least the MAC address, IP address and expiry timestamp.
- DHCP range allocations are sent by the leader to every other node individually when necessary (leader changes, a node is running out of addresses, a node is dropped etc). This makes sure the DHCP range is fully utilized without overlap. This includes the starts and lengths of the ranges that the node allocates from.
- DHCP config is multicast to all nodes by the leader when a leader is elected. This makes sure that all nodes are using the same version of the configuration, even if one has an outdated local copy of the configuration file.
- Cluster status is a simple message which the leader uses to let the other nodes know, if the cluster has the majority required to continue operating.
