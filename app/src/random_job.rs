//! Runs the random 3D fractal search on a worker thread.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc;

use render::RandomResult;
use scene::Scene;

pub struct RandomJob {
    attempt: Arc<AtomicU32>,
    cancel: Arc<AtomicBool>,
    result: mpsc::Receiver<Result<RandomResult, String>>,
}

impl RandomJob {
    pub fn start(device: &wgpu::Device, queue: &wgpu::Queue, base: &Scene, seed: u64) -> Self {
        let attempt = Arc::new(AtomicU32::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel();
        let (device, queue) = (device.clone(), queue.clone());
        let base = base.clone();
        let (worker_attempt, worker_cancel) = (Arc::clone(&attempt), Arc::clone(&cancel));
        std::thread::Builder::new()
            .name("random".into())
            .spawn(move || {
                let result = formulas::Library::builtin().and_then(|library| {
                    render::find_random_fractal(&device, &queue, &library, &base, seed, |n| {
                        worker_attempt.store(n, Ordering::Relaxed);
                        !worker_cancel.load(Ordering::Relaxed)
                    })
                    .ok_or_else(|| "cancelled".to_owned())
                });
                let _ = sender.send(result);
            })
            .expect("spawning the random search thread");
        Self {
            attempt,
            cancel,
            result: receiver,
        }
    }

    pub fn attempt(&self) -> u32 {
        self.attempt.load(Ordering::Relaxed)
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// The outcome, once the search has finished.
    pub fn poll(&self) -> Option<Result<RandomResult, String>> {
        match self.result.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err("random search crashed".into())),
        }
    }
}
