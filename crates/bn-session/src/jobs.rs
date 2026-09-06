//! Background-job plumbing: cancel flag + throttled progress reporting.
//! (Port of the egui app's worker.rs, minus the repaint hooks — the UI
//! forwards progress through a channel into a signal instead.)

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Progress snapshot shown by the UI. `frac: None` = indeterminate.
#[derive(Clone, Debug, Default)]
pub struct JobEvent {
    pub frac: Option<f32>,
    pub text: String,
}

/// Handed to a worker body: cancel flag + throttled progress sink.
pub struct JobCtx {
    pub cancel: Arc<AtomicBool>,
    sink: Box<dyn Fn(JobEvent) + Send + Sync>,
    last_report: Mutex<Instant>,
}

impl JobCtx {
    pub fn new(cancel: Arc<AtomicBool>, sink: impl Fn(JobEvent) + Send + Sync + 'static) -> JobCtx {
        JobCtx {
            cancel,
            sink: Box::new(sink),
            // Backdate so the first report always goes through.
            last_report: Mutex::new(Instant::now() - Duration::from_secs(1)),
        }
    }

    /// Publish progress, throttled to ~20 Hz so a hot callback can't flood
    /// the progress channel.
    pub fn report(&self, p: JobEvent) {
        let mut last = self.last_report.lock().unwrap();
        if last.elapsed() >= Duration::from_millis(50) {
            *last = Instant::now();
            drop(last);
            (self.sink)(p);
        }
    }

    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}
