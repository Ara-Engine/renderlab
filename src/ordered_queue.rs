//! The renderer's queue, with an optional submit thread (`RENDERLAB_SUBMIT_THREAD=1`).
//!
//! On that thread a frame's last command encoder is finished and submitted while the caller
//! returns (wgpu-core does a frame's Vulkan encoding and resource tracking in `finish` and
//! `Queue::submit`: about 7 ms of CPU a frame on Ara's Outpost). Every later use of the queue,
//! through `Deref` (writes, uploads, other submits), first waits for that submit, so queue writes
//! and submissions keep the order they had on one thread.
use std::ops::Deref;
use std::panic::Location;
use std::sync::{Mutex, mpsc};
use std::time::Instant;
use std::thread::JoinHandle;

pub(crate) struct OrderedQueue {
    queue: wgpu::Queue,
    worker: Option<Worker>,
    /// Signalled when the last handed-off submit is done, with when it was handed off.
    pending: Mutex<Option<(mpsc::Receiver<()>, Instant)>>,
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
                            let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.;
                            crate::renderer::submit_log(|| {
                                format!(
                                    "submit_thread finish={:.2} queue_submit={:.2}",
                                    ms(finished),
                                    ms(start.elapsed() - finished)
                                )
                            });
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
    #[track_caller]
    pub(crate) fn submit_later(&self, encoder: wgpu::CommandEncoder) {
        self.join();
        let (done, signal) = mpsc::channel();
        let jobs = self.worker.as_ref().and_then(|w| w.jobs.as_ref());
        jobs.expect("submit_later without a submit thread")
            .send((encoder, done))
            .expect("the renderlab submit thread stopped");
        *self.pending.lock().unwrap() = Some((signal, Instant::now()));
    }

    /// Waits for the last handed-off submit, if any.
    #[track_caller]
    pub(crate) fn join(&self) {
        let Some((signal, handed)) = self.pending.lock().unwrap().take() else {
            return;
        };
        let start = Instant::now();
        // A closed channel means the thread panicked; the panic surfaces on drop.
        let _ = signal.recv();
        // RENDERLAB_SUBMIT_LOG: how long this queue use waited for the submit thread (near zero
        // when the caller's other work covered the submit), how long after the hand-off it came,
        // and where it was.
        let at = Location::caller();
        let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.;
        crate::renderer::submit_log(|| {
            format!(
                "submit_thread join_wait={:.2} after_handoff={:.2} at={}:{}",
                ms(start.elapsed()),
                ms(start - handed),
                at.file(),
                at.line()
            )
        });
    }
}

impl Deref for OrderedQueue {
    type Target = wgpu::Queue;
    #[track_caller]
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
