#![deny(clippy::unwrap_used, clippy::allow_attributes_without_reason)]
#![warn(clippy::perf, clippy::complexity, clippy::pedantic, clippy::suspicious)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    reason = "We're not going to write comprehensive docs"
)]
#![allow(
    clippy::cast_precision_loss,
    reason = "There are no sufficient floating point types"
)]

use std::{
    any::Any,
    convert::TryInto,
    error::Error,
    io,
    num::NonZero,
    sync::{
        mpsc::{self, Receiver, RecvError, Sender},
        Arc, Mutex, PoisonError, RwLock,
    },
    thread::{self, JoinHandle},
};

type PanicHandler = Box<dyn Fn(usize, Box<dyn Any>) + Send + Sync>;
enum Job {
    Quit,
    Execute(Box<dyn FnOnce() + Send + 'static>),
}
type Rx = Arc<Mutex<Receiver<Job>>>;
type Tx = Sender<Job>;

pub struct ThreadPool {
    inner: Arc<RwLock<Inner>>,
}

struct Inner {
    panic_handler: PanicHandler,
    workers: Vec<Worker>,
    size: usize,
    next_id: usize,
    rx: Option<Rx>,
    tx: Option<Tx>,
}

fn new_poison_err(_: impl Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "ThreadPool lock poisoned")
}

impl ThreadPool {
    /// Create a new `ThreadPool`.
    ///
    /// The size is the number of threads in the pool.
    pub fn new<P>(size: NonZero<usize>, panic_handler: P) -> io::Result<ThreadPool>
    where
        P: Fn(usize, Box<dyn Any>) + Send + Sync + 'static,
    {
        let (tx, rx) = mpsc::channel();
        let rx = Arc::new(Mutex::new(rx));
        let tx = Some(tx);
        let size = size.into();

        let mut workers = Vec::with_capacity(size);
        for id in 0..size {
            workers.push(Worker::new(id, Arc::clone(&rx))?);
        }

        Ok(ThreadPool {
            inner: Arc::new(RwLock::new(Inner {
                panic_handler: Box::new(panic_handler),
                workers,
                size,
                next_id: size,
                rx: Some(rx),
                tx,
            })),
        })
    }

    pub fn execute<F>(&self, f: F) -> io::Result<()>
    where
        F: FnOnce() + Send + 'static,
    {
        self.inner.write().map_err(new_poison_err)?.respawn()?;
        self.execute_unchecked(f)
    }

    pub fn set_size(&self, size: NonZero<usize>) -> io::Result<isize> {
        self.inner.write().map_err(new_poison_err)?.set_size(size)
    }

    pub fn size(&self) -> io::Result<usize> {
        let size = self.inner.read().map_err(new_poison_err)?.size;
        Ok(size)
    }

    fn execute_unchecked<F>(&self, f: F) -> io::Result<()>
    where
        F: FnOnce() + Send + 'static,
    {
        let job = Box::new(f);
        self.inner
            .read()
            .map_err(new_poison_err)?
            .tx
            .as_ref()
            .expect("Invariant violated: sender should exist before ThreadPool::drop")
            .send(Job::Execute(job))
            .expect("Invariant violated: channel should always function before ThreadPool::drop");
        Ok(())
    }
}

impl Clone for ThreadPool {
    fn clone(&self) -> Self {
        ThreadPool {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl Inner {
    fn partition_quit_workers(&mut self) -> usize {
        self.workers.sort_unstable_by_key(Worker::has_quit);
        self.workers.partition_point(|worker| !worker.has_quit())
    }

    fn respawn(&mut self) -> io::Result<()> {
        let i = self.partition_quit_workers();

        // Extract quit workers and join them
        let quit_workers = self.workers.split_off(i);
        for mut worker in quit_workers {
            worker.join(&self.panic_handler);
        }

        // Spawn new workers up to missing
        let need = self.size.saturating_sub(self.workers.len());
        for _ in 0..need {
            self.workers.push(Worker::new(
                self.next_id,
                Arc::clone(
                    self.rx.as_ref().expect(
                        "Invariant violated: receiver should exist before ThreadPool::drop",
                    ),
                ),
            )?);
            self.next_id = self.next_id.wrapping_add(1);
        }

        Ok(())
    }

    fn set_size(&mut self, size: NonZero<usize>) -> io::Result<isize> {
        let size: usize = size.into();
        let diff = TryInto::<isize>::try_into(size).expect("Too many threads")
            - TryInto::<isize>::try_into(self.size).expect("Too many threads");

        // Stop extra workers
        if diff < 0 {
            for _ in 0..diff.abs() {
                self
                    .tx
                    .as_ref()
                    .expect("Invariant violated: sender should exist before ThreadPool::drop")
                    .send(Job::Quit)
                    .expect("Invariant violated: channel should always function before ThreadPool::drop");
            }
        }

        // Update size
        self.size = size;

        // Join old workers and spawn new workers if needed
        self.respawn()?;

        Ok(diff)
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        drop(self.tx.take());
        drop(self.rx.take());

        for worker in &mut self.workers {
            worker.join(&self.panic_handler);
        }
    }
}

#[derive(Debug)]
struct Worker {
    id: usize,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    fn new(id: usize, rx: Rx) -> io::Result<Worker> {
        let thread = thread::Builder::new()
            .name(format!("{}::Worker({})", module_path!(), id))
            .spawn(move || loop {
                let message = match rx.lock() {
                    Ok(guard) => guard,
                    Err(poisoned @ PoisonError { .. }) => {
                        // Receiver mutex poisoned by other thread panic should not affect us
                        poisoned.into_inner()
                    }
                }
                .recv();
                match message {
                    Ok(Job::Execute(job)) => job(),
                    Ok(Job::Quit) | Err(RecvError { .. }) => break,
                }
            })?;

        Ok(Worker {
            id,
            thread: Some(thread),
        })
    }

    fn join(&mut self, panic_handler: &PanicHandler) {
        if let Some(thread) = self.thread.take() {
            if let Err(message) = thread.join() {
                (panic_handler)(self.id, message);
            }
        }
    }

    fn has_quit(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "Unwrap is ok in test code")]
mod tests {
    use std::time::Duration;

    use super::*;

    fn new(size: usize) -> ThreadPool {
        ThreadPool::new(size.try_into().unwrap(), |_, _| {}).unwrap()
    }

    fn inner(pool: &ThreadPool) -> std::sync::RwLockReadGuard<'_, Inner> {
        pool.inner.read().unwrap()
    }

    fn inner_mut(pool: &ThreadPool) -> std::sync::RwLockWriteGuard<'_, Inner> {
        pool.inner.write().unwrap()
    }

    fn wait() {
        std::thread::sleep(Duration::from_secs(1));
    }

    fn partition(pool: ThreadPool, quit: usize) -> ThreadPool {
        let (i, n) = {
            let i = inner_mut(&pool).partition_quit_workers();
            (i, inner(&pool).workers.len())
        };

        for worker in &inner(&pool).workers {
            println!("{}: {}", worker.id, worker.has_quit());
        }
        println!();

        assert_eq!(i, n - quit);
        {
            let mut pool = inner_mut(&pool);
            let quit_workers = pool.workers.split_off(i);
            for worker in &quit_workers {
                assert!(worker.has_quit());
            }
            assert_eq!(quit_workers.len(), quit);
            for worker in &pool.workers {
                assert!(!worker.has_quit());
            }
        }
        pool
    }

    #[test]
    fn partitioning_logic() {
        let pool = new(8);
        let pool = partition(pool, 0);

        pool.execute_unchecked(|| {
            panic!();
        })
        .unwrap();
        wait();

        let pool = partition(pool, 1);

        for _ in 0..3 {
            pool.execute_unchecked(|| {
                panic!();
            })
            .unwrap();
        }
        wait();

        partition(pool, 3);
    }

    #[test]
    fn grow() {
        let pool = new(1);
        assert_eq!(pool.size().unwrap(), 1);
        assert_eq!(inner(&pool).workers.len(), 1);

        let diff = pool.set_size(10.try_into().unwrap()).unwrap();
        assert_eq!(diff, 9);
        assert_eq!(pool.size().unwrap(), 10);
        assert_eq!(inner(&pool).workers.len(), 10);

        let diff = pool.set_size(32.try_into().unwrap()).unwrap();
        assert_eq!(diff, 22);
        assert_eq!(pool.size().unwrap(), 32);
        assert_eq!(inner(&pool).workers.len(), 32);
    }

    #[test]
    fn shrink() {
        let pool = new(40);
        assert_eq!(pool.size().unwrap(), 40);
        assert_eq!(inner(&pool).workers.len(), 40);

        let diff = pool.set_size(10.try_into().unwrap()).unwrap();
        assert_eq!(diff, -30);
        assert_eq!(pool.size().unwrap(), 10);
        wait();
        inner_mut(&pool).respawn().unwrap();
        assert_eq!(inner(&pool).workers.len(), 10);

        let diff = pool.set_size(3.try_into().unwrap()).unwrap();
        assert_eq!(diff, -7);
        assert_eq!(pool.size().unwrap(), 3);
        wait();
        inner_mut(&pool).respawn().unwrap();
        assert_eq!(inner(&pool).workers.len(), 3);
    }

    #[test]
    fn ping_pong() {
        let pool = new(2);
        assert_eq!(pool.size().unwrap(), 2);
        assert_eq!(inner(&pool).workers.len(), 2);

        let diff = pool.set_size(10.try_into().unwrap()).unwrap();
        assert_eq!(diff, 8);
        assert_eq!(pool.size().unwrap(), 10);
        wait();
        assert_eq!(inner(&pool).workers.len(), 10);

        let diff = pool.set_size(32.try_into().unwrap()).unwrap();
        assert_eq!(diff, 22);
        assert_eq!(pool.size().unwrap(), 32);
        wait();
        assert_eq!(inner(&pool).workers.len(), 32);

        let diff = pool.set_size(16.try_into().unwrap()).unwrap();
        assert_eq!(diff, -16);
        assert_eq!(pool.size().unwrap(), 16);
        wait();
        inner_mut(&pool).respawn().unwrap();
        assert_eq!(inner(&pool).workers.len(), 16);

        let diff = pool.set_size(1.try_into().unwrap()).unwrap();
        assert_eq!(diff, -15);
        assert_eq!(pool.size().unwrap(), 1);
        wait();
        inner_mut(&pool).respawn().unwrap();
        assert_eq!(inner(&pool).workers.len(), 1);

        let diff = pool.set_size(10.try_into().unwrap()).unwrap();
        assert_eq!(diff, 9);
        assert_eq!(pool.size().unwrap(), 10);
        assert_eq!(inner(&pool).workers.len(), 10);
    }
}
