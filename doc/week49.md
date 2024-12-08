## 2024-12-08

It was a laborious week.
Short summary of what was accomplished:
- Bully algorithm
  - Implementation was made possible by the massive refactoring last week
- Text UI for visualizing server state in the console
- Address pool division between nodes by the coordinator
- Client listener system with a worker thread pool
  - Work in progress
- Client-server protocol
  - We opted for a custom pseudo-DHCP for now, proper DHCP can be done later
- Relentless refinements to the distributed systems algorithms, logging and UI

And finally today, we wrote the initial implementation of the CLI client for testing the server cluster.
Work is still underway to hook up the client-server functionality of the cluster, and that will be the focus next week.

Another thing I will try to get done, is recording a demo video both for showing in the GitHub readme,
but also as a fallback for when our live demo fails to work :)
The course's demo session will be on next Wednesday.
