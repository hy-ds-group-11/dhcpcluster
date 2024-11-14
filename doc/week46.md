## 2024-11-14

We created a [design document](design.md) design document conforming to project description.

Some thoughts and ideas by yours truly (Lauri) (written while others are busy writing the design document):
- We could do integration testing by mocking TcpStream
  - Server logic needs to be generic, `T: Read + Write + ...`
- We __should__ do unit testing
- Automatic test coverage generation would be interesting but not required
- The actual protocol definition will just be some Rust `struct`s and `enum`s, decorated with [serde](https://serde.rs/) macros
- In a release version, the server-server protocol should probably have a version field... This can be done with an enum if we want to
