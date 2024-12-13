use std::{
    any::Any,
    io,
    num::NonZero,
    sync::{
        mpsc::{self, Receiver, RecvError, Sender},
        Arc, Mutex, PoisonError, RwLock,
    },
    thread::{self, JoinHandle},
};

type PanicHandler = Box<dyn Fn(usize, Box<dyn Any>) + Send + Sync>;
type Job = Box<dyn FnOnce() + Send + 'static>;
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

impl ThreadPool {
    /// Create a new ThreadPool.
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

    fn execute_unchecked<F>(&self, f: F) -> io::Result<()>
    where
        F: FnOnce() + Send + 'static,
    {
        let job = Box::new(f);
        self.inner
            .read()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "ThreadPool lock poisoned"))?
            .tx
            .as_ref()
            .expect("Invariant violated: sender should exist before ThreadPool::drop")
            .send(job)
            .expect("Invariant violated: channel should always function before ThreadPool::drop");
        Ok(())
    }

    pub fn execute<F>(&self, f: F) -> io::Result<()>
    where
        F: FnOnce() + Send + 'static,
    {
        self.inner
            .write()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "ThreadPool lock poisoned"))?
            .respawn()?;
        self.execute_unchecked(f)
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
        self.workers
            .sort_unstable_by_key(|worker| worker.has_quit());
        self.workers.partition_point(|worker| !worker.has_quit())
    }

    fn respawn(&mut self) -> io::Result<()> {
        let i = self.partition_quit_workers();

        // Extract quit workers and join them
        let quit_workers = self.workers.split_off(i);
        for mut worker in quit_workers.into_iter() {
            worker.join(&self.panic_handler);
        }

        // Spawn new workers up to missing
        let need = self.size - self.workers.len();
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
                    Ok(job) => job(),
                    Err(RecvError { .. }) => break,
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
        self.thread
            .as_ref()
            .map(|handle| handle.is_finished())
            .unwrap_or(true)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn partition_booleans() {
        let bools = [true, true, false, false, false, false, false, false];
        assert_eq!(bools.partition_point(|&b| b), 2);
        let bools = [true, false, false, false, false, false, false, false];
        assert_eq!(bools.partition_point(|&b| b), 1);
    }

    #[test]
    fn partitioning_logic() {
        let pool = ThreadPool::new(8.try_into().unwrap(), |_, _| {}).unwrap();

        let i = pool.inner.write().unwrap().partition_quit_workers();

        for worker in pool.inner.read().unwrap().workers.iter() {
            println!("{}: {}", worker.id, worker.has_quit());
        }
        println!();
        assert_eq!(i, 8);

        {
            let mut pool = pool.inner.write().unwrap();
            let quit_workers = pool.workers.split_off(i);
            assert!(quit_workers.is_empty());
        }

        pool.execute_unchecked(|| {
            panic!();
        })
        .unwrap();
        thread::sleep(Duration::from_secs(1));

        let i = pool.inner.write().unwrap().partition_quit_workers();

        for worker in pool.inner.read().unwrap().workers.iter() {
            println!("{}: {}", worker.id, worker.has_quit());
        }
        println!();

        assert_eq!(i, 7);
        {
            let mut pool = pool.inner.write().unwrap();
            let quit_workers = pool.workers.split_off(i);
            for worker in &quit_workers {
                assert!(worker.has_quit());
            }
            assert_eq!(quit_workers.len(), 1);
            for worker in &pool.workers {
                assert!(!worker.has_quit());
            }
        }

        for _ in 0..3 {
            pool.execute_unchecked(|| {
                panic!();
            })
            .unwrap();
        }
        thread::sleep(Duration::from_secs(1));

        let i = pool.inner.write().unwrap().partition_quit_workers();

        for worker in pool.inner.read().unwrap().workers.iter() {
            println!("{}: {}", worker.id, worker.has_quit());
        }
        println!();

        assert_eq!(i, 4);
        {
            let mut pool = pool.inner.write().unwrap();
            let quit_workers = pool.workers.split_off(i);
            for worker in &quit_workers {
                assert!(worker.has_quit());
            }
            assert_eq!(quit_workers.len(), 3);
            for worker in &pool.workers {
                assert!(!worker.has_quit());
            }
        }
    }
}
