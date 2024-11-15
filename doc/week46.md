## 2024-11-14

We created a [design document](design.md) conforming to project description.

Some thoughts and ideas by yours truly (Lauri) (written while others are busy writing the design document):
- We could do integration testing by mocking TcpStream
  - Server logic needs to be generic, `T: Read + Write + ...`
- We __should__ do unit testing
- Automatic test coverage generation would be interesting but not required
- The actual protocol definition will just be some Rust `struct`s and `enum`s, decorated with [serde](https://serde.rs/) macros
- In a release version, the server-server protocol should probably have a version field... This can be done with an enum if we want to

## 2024-11-15

We broke ground and wrote first code for the server node, including configuration file parsing and server peer listener thread.
It was mostly straightforward, but [joining threads](https://doc.rust-lang.org/std/thread/struct.JoinHandle.html) had us thinking for a while.

What we learned, was that Rust's thread JoinHandle's join-method returns a `Result`, whose:
- `Ok`-variant contains the return value from the function/closure which the thread ran
  - This could be a nested `Result<T, E>`!
- `Err`-variant contains the value passed to panic (usually a string reference)
  - But it's provided to us through a `Box<dyn Any + Send + 'static>>`-pointer!
  - This is because `panic!(...)` doesn't have to be called with a `&str`, some applications need a different type to return from a paniced thread
  - But in our case it's going to be a `&str` if anything, so we call `downcast_ref::<&str>()` on it to get an `Option<&str>` that can be debug-printed

The session itself was conducted on Discord with voice communication and a desktop share stream. This seems to work pretty well.
