## 2024-11-26

The test harness is pretty much feature-complete now, with both `TcpStream` and
`TcpListener` mocked and repository test coverage at over 90% at the moment.
There is about 30% greater quantity of lines of test code than actual server
implementation, and the team needs to get all members introduced to the test
harness so that development can continue as a team effort.
The next session has been scheduled for tomorrow afternoon.

## 2024-11-27

To be able to focus on feature development, we removed all the test code from
the development branch for now. The idea would be to bring unit testing back
when the codebase has matured to more or less feature-complete.

After sorting out the finicky tests, we implemented heatbeat messages and
graceful peer communication thread shutdown when the hearbeats time out.

## 2024-11-28

During today's mob programming session (with a projector!), we tried to
implement the bully algorithm. but instead of managing to add that feature,
Rust decided to show us that our code wasn't quite ready for that.

So the groupwork session turned into an arduous refactoring, the results of
which can be seen
[here](https://github.com/hy-ds-group-11/dhcpcluster/commit/3b4e365a5c9a65ffc631631d4b798ff5d802184e)
and [here](https://github.com/hy-ds-group-11/dhcpcluster/commit/c61f22d460b0e3983185da83f0daf1b8cb894077).

The gist of the refactoring is that we decided to forgo the shared mutex-guarded
server state completely, opting for a new internal message passing channel
between threads. Message passing is a neat paradigm in Rust, because it allows
developers to avoid thinking about shared ownership and mutual exclusion,
and just have a single owner (a loop in a thread) for the "shared" data.
The owner can apply changes and perform logic on behalf of other threads, based
on internal message passing (within the server process).
