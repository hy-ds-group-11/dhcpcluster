## 2024-11-19

Over the weekend, we have been pondering our options for choosing the mechanism which would allow us to concurrently receive messages from N-1 `TcpStream`s.

In summary, the options are:
- Have N-1 threads that each own one `TcpStream`, and using blocking reads on the streams
- Have a single-threaded solution, and use nonblocking reads
- Have a single-threaded solution, and use nonblocking reads with `epoll` to block when there are no incoming messages. This requires the `libc` crate and is specific to Linux.
- Use `tokio` and `async` Rust

There is a lot to write about each option, but ChatGPT summarized the pros and cons very well:

### 1. **One thread per `TcpStream` (blocking reads)**
   - **Pros:**
     - Simplicity: Each thread handles its own connection with a blocking read, letting the OS manage scheduling. This aligns with Rust's ownership model, making it easy to reason about and implement.
     - Concurrency: The OS can handle concurrency efficiently by context-switching threads when they block on I/O.
     - No busy-waiting or polling, so CPU utilization will be efficient.

   - **Cons:**
     - Overhead: With a large number of nodes (and thus many threads), the cost of spawning and maintaining many threads can become significant. The OS may also impose limits on the number of threads, and you may experience issues with stack memory or scheduling overhead if `N` is very large.
     - Scaling: On a large distributed system, context-switching overhead could grow, and having too many threads might lead to inefficiency.

   - **When it's appropriate:**
     - If `N` (the number of nodes) is relatively small and performance is acceptable, this could be a simple and effective solution. Rust's threading model makes this easier to implement.

### 2. **One thread for all `TcpStream`s (non-blocking reads in round-robin)**
   - **Pros:**
     - Lower memory overhead: Since you're using fewer threads (just one), you save memory and avoid context-switching overhead.
     - Fine-grained control: With `epoll` (or `kqueue` on BSD/macOS), you can efficiently wait on multiple sockets and react as soon as any data is available, which can scale well to many connections.

   - **Cons:**
     - Complexity: Manually managing non-blocking reads and handling `epoll` or a similar mechanism is more complex than the multithreaded approach.
     - CPU usage: Without `epoll`, polling in a round-robin manner will likely result in wasted CPU cycles due to busy-waiting, especially if there's a long wait between messages.

   - **When it's appropriate:**
     - For large-scale systems where `N` is large and memory/threading overhead is a concern, this is the preferred approach, as `epoll` is efficient for multiplexing I/O over many sockets.

### Recommended approach:
For performance and scalability, I'd recommend using a library that abstracts over the non-blocking I/O complexity and lets you focus on the logic rather than low-level socket management.

- **Tokio (for async I/O in Rust)**: It provides an async runtime built on top of `epoll` (on Linux) or equivalent mechanisms (such as `kqueue` on macOS). This allows you to write non-blocking code without manually managing the low-level system calls, while maintaining good performance and scalability. Each `TcpStream` would be managed asynchronously in the same thread, with minimal overhead.
  - Pros: The runtime takes care of scheduling and waiting efficiently on I/O events, and it's well-integrated with Rust's async/await syntax.
  - Cons: Requires adopting async programming patterns, which might be more complex depending on your existing design.

### Conclusion:
- If you want a simple, easier-to-maintain solution and `N` is small, the one-thread-per-connection approach might work well for you.
- For better scalability, consider leveraging async I/O with a library like `Tokio` rather than manually managing `epoll`. It will reduce complexity and improve performance, especially as the number of nodes grows.

## 2024-11-21

We had an intensive mob programming session, where we settled on proceeding with the threaded solution.
It turns out, that we want to spawn two threads per peer, one for blocking reads and
the other for blocking writes! This way the message transmissions to other peers will not
halt, even if one of the peers is slow to respond or a timeout must be reached in the event that
a peer crashes. Passing messages to the sender thread was accomplished with a standard Rust mpsc channel.

We also added some actual server to server communication, so the first messages
(handshakes and acknowledgements) were exchanged.
There are no immediate roadblocks in the horizon right now.

Later in the evening, I (Lauri) got inspired to try to write tests for the server.
It was difficult and time consuming to first make the implementation generic over
a "TcpStream" implementation, and then mock the TcpStream, but the end result is
a working initial implementation for a test suite.

For the mocking to work, I had to use some Rust features that I have only learned very recently,
such as the Any-trait and interior mutability with RefCell.
