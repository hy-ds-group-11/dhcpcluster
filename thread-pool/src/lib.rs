use std::{
    any::Any,
    num::NonZero,
    sync::{mpsc, Arc, Mutex},
    thread,
};

type PanicHandler = Box<dyn Fn(usize, Box<dyn Any>) + Send + Sync>;

pub struct ThreadPool {
    panic_handler: PanicHandler,
    workers: Vec<Worker>,
    sender: Option<mpsc::Sender<Job>>,
}

type Job = Box<dyn FnOnce() + Send + 'static>;

impl ThreadPool {
    /// Create a new ThreadPool.
    ///
    /// The size is the number of threads in the pool.
    pub fn new(size: NonZero<usize>, panic_handler: PanicHandler) -> ThreadPool {
        let (sender, receiver) = mpsc::channel();

        let receiver = Arc::new(Mutex::new(receiver));

        let mut workers = Vec::with_capacity(size.into());

        for id in 0..size.into() {
            workers.push(Worker::new(id, Arc::clone(&receiver)));
        }

        ThreadPool {
            panic_handler,
            workers,
            sender: Some(sender),
        }
    }

    pub fn execute<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        let job = Box::new(f);

        self.sender.as_ref().unwrap().send(job).unwrap();
    }
}

impl Drop for ThreadPool {
    fn drop(&mut self) {
        drop(self.sender.take());

        for worker in &mut self.workers {
            if let Some(thread) = worker.thread.take() {
                if let Err(message) = thread.join() {
                    (self.panic_handler)(worker.id, message);
                }
            }
        }
    }
}

#[derive(Debug)]
struct Worker {
    id: usize,
    thread: Option<thread::JoinHandle<()>>,
}

impl Worker {
    fn new(id: usize, receiver: Arc<Mutex<mpsc::Receiver<Job>>>) -> Worker {
        let thread = thread::Builder::new()
            .name(format!("{}::Worker({})", module_path!(), id))
            .spawn(move || loop {
                let message = receiver.lock().unwrap().recv();

                match message {
                    Ok(job) => {
                        job();
                    }
                    Err(_) => {
                        break;
                    }
                }
            })
            .unwrap();

        Worker {
            id,
            thread: Some(thread),
        }
    }
}
