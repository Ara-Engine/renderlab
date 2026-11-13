//! The renderer's queue, with an optional submit thread (`RENDERLAB_SUBMIT_THREAD=1`).
//!
//! On that thread a frame's last command encoder is finished and submitted while the caller
//! returns (wgpu-core does a frame's Vulkan encoding and resource tracking in `finish` and
//! `Queue::submit`: about 7 ms of CPU a frame on Ara's Outpost). Every later use of the queue,
//! through `Deref` (writes, uploads, other submits), first waits for that submit, so queue writes
//! and submissions keep the order they had on one thread.
use std::ops::Deref;
use std::sync::{Mutex, mpsc};
use std::thread::JoinHandle;

pub(crate) struct OrderedQueue {
    queue: wgpu::Queue,
    worker: Option<Worker>,
    /// Signalled when the last handed-off submit is done.
    pending: Mutex<Option<mpsc::Receiver<()>>>,
}

struct Worker {
    jobs: Option<mpsc::Sender<(wgpu::CommandEncoder, mpsc::Sender<()>)>>,
    thread: Option<JoinHandle<()>>,
}

impl OrderedQueue {
    pub(crate) fn new(queue: wgpu::Queue) -> Self {
        let worker = std::env::var("RENDERLAB_SUBMIT_THREAD")
            .is_ok_and(|v| v == "1")
            .then(|| {
                let (jobs, received) = mpsc::channel::<(wgpu::CommandEncoder, mpsc::Sender<()>)>();
                let queue = queue.clone();
                let thread = std::thread::Builder::new()
                    .name("renderlab-submit".into())
                    .spawn(move || {
                        for (encoder, done) in received {
                            let start = std::time::Instant::now();
                            let commands = encoder.finish();
                            let finished = start.elapsed();
                            queue.submit([commands]);
                            if std::env::var_os("RENDERLAB_SUBMIT_LOG").is_some() {
                                let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.;
                                eprintln!(
                                    "submit_thread finish={:.2} queue_submit={:.2}",
                                    ms(finished),
                                    ms(start.elapsed() - finished)
                                );
                            }
                            let _ = done.send(());
                        }
                    })
                    .expect("spawn the renderlab submit thread");
                Worker {
                    jobs: Some(jobs),
                    thread: Some(thread),
                }
            });
        Self {
            queue,
            worker,
            pending: Mutex::new(None),
        }
    }

    /// Whether `submit_later` hands work to the submit thread.
    pub(crate) fn threaded(&self) -> bool {
        self.worker.is_some()
    }

    /// Finishes and submits `encoder` on the submit thread; later queue uses wait for it.
    /// Only call when `threaded()`.
    pub(crate) fn submit_later(&self, encoder: wgpu::CommandEncoder) {
        self.join();
        let (done, signal) = mpsc::channel();
        let jobs = self.worker.as_ref().and_then(|w| w.jobs.as_ref());
        jobs.expect("submit_later without a submit thread")
            .send((encoder, done))
            .expect("the renderlab submit thread stopped");
        *self.pending.lock().unwrap() = Some(signal);
    }

    /// Waits for the last handed-off submit, if any.
    pub(crate) fn join(&self) {
        if let Some(signal) = self.pending.lock().unwrap().take() {
            let start = std::time::Instant::now();
            // A closed channel means the thread panicked; the panic surfaces on drop.
            let _ = signal.recv();
            // How long a queue use waited for the submit thread: near zero when the caller's
            // other work covered the submit.
            if std::env::var_os("RENDERLAB_SUBMIT_LOG").is_some() {
                eprintln!("submit_thread join_wait={:.2}", start.elapsed().as_secs_f64() * 1000.);
            }
        }
    }
}

impl Deref for OrderedQueue {
    type Target = wgpu::Queue;
    fn deref(&self) -> &wgpu::Queue {
        self.join();
        &self.queue
    }
}

impl Drop for OrderedQueue {
    fn drop(&mut self) {
        self.join();
        if let Some(mut w) = self.worker.take() {
            drop(w.jobs.take());
            if let Some(t) = w.thread.take() {
                let _ = t.join();
            }
        }
    }
}
