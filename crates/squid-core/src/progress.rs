//! Cooperative cancellation plus a low-noise progress channel.
//!
//! Scans run on worker threads; the CLI prints a spinner while the TUI drains
//! the same channel into its state machine.

use crate::bytes::Bytes;
use crate::model::{Category, ScanReport};
use crossbeam_channel::{unbounded, Receiver, Sender, TryRecvError};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub enum Event {
    /// A named phase started so the UI can say what it is waiting for.
    Phase {
        phase: &'static str,
        detail: String,
    },
    /// Progress against a known or unknown total.
    Tick {
        done: u64,
        total: Option<u64>,
        subject: Option<PathBuf>,
    },
    /// A finished bucket, streamed as soon as it is known.
    Bucket {
        category: Category,
        total: Bytes,
        count: usize,
    },
    /// Cumulative measurement counters, streamed while sizes are computed
    /// (Mole's "Scanning: 128,431 files · 38.2 GB" live status line).
    Metered {
        files: u64,
        bytes: Bytes,
    },
    Deleted {
        path: PathBuf,
        freed: Bytes,
    },
    Note(String),
    Warning(String),
    Done(ScanReport),
}

#[derive(Clone)]
pub struct Progress {
    tx: Sender<Event>,
    rx: Receiver<Event>,
    started: Instant,
}

impl Default for Progress {
    fn default() -> Self {
        Self::new()
    }
}

impl Progress {
    pub fn new() -> Self {
        let (tx, rx) = unbounded();
        Self {
            tx,
            rx,
            started: Instant::now(),
        }
    }

    pub fn sender(&self) -> Sender<Event> {
        self.tx.clone()
    }

    pub fn publish(&self, event: Event) {
        // Losing a progress event must never fail the operation.
        let _ = self.tx.send(event);
    }

    pub fn phase(&self, phase: &'static str, detail: impl Into<String>) {
        self.publish(Event::Phase {
            phase,
            detail: detail.into(),
        });
    }

    pub fn tick(&self, done: u64, total: Option<u64>, subject: Option<PathBuf>) {
        self.publish(Event::Tick {
            done,
            total,
            subject,
        });
    }

    pub fn note(&self, msg: impl Into<String>) {
        self.publish(Event::Note(msg.into()));
    }

    pub fn warning(&self, msg: impl Into<String>) {
        self.publish(Event::Warning(msg.into()));
    }

    pub fn drain(&self) -> Vec<Event> {
        let mut out = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(ev) => out.push(ev),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        out
    }

    /// Block until an event arrives or the timeout expires.
    pub fn recv(&self, timeout: Duration) -> Option<Event> {
        self.rx.recv_timeout(timeout).ok()
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Receiver for a worker thread.
    pub fn sink(&self) -> EventSink {
        EventSink {
            tx: self.tx.clone(),
        }
    }
}

/// Cheap handle passed into engine worker code.
#[derive(Clone)]
pub struct EventSink {
    tx: Sender<Event>,
}

impl EventSink {
    pub fn emit(&self, event: Event) {
        let _ = self.tx.send(event);
    }

    pub fn note(&self, msg: impl Into<String>) {
        let _ = self.tx.send(Event::Note(msg.into()));
    }

    pub fn warning(&self, msg: impl Into<String>) {
        let _ = self.tx.send(Event::Warning(msg.into()));
    }
}

/// Thread-safe cancel flag; cheap to clone into worker threads.
#[derive(Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    pub fn check(&self) -> crate::error::Result<()> {
        if self.is_cancelled() {
            Err(crate::error::SquidError::Cancelled)
        } else {
            Ok(())
        }
    }
}

/// Small helper for a spinner in the CLI.
pub struct ElapsedTicker {
    pub started: Instant,
}

impl Default for ElapsedTicker {
    fn default() -> Self {
        Self {
            started: Instant::now(),
        }
    }
}

impl ElapsedTicker {
    pub fn label(&self) -> String {
        let secs = self.started.elapsed().as_secs_f32();
        if secs < 60.0 {
            format!("{secs:.1}s")
        } else {
            format!("{}m {:02}s", secs as u64 / 60, secs as u64 % 60)
        }
    }
}
