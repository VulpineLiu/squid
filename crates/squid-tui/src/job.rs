//! Background work. A TUI that blocks on a disk walk is a broken TUI, so every
//! engine call runs on its own thread and reports through two channels:
//!
//! * [`Progress`] events (phases, ticks, warnings) feed the live progress bar;
//! * one final value on completion, delivered to [`JobHandle::poll`].
//!
//! Each job builds its own [`Engine`]. That keeps the borrow checker happy and
//! means a job always sees current settings, at the cost of re-reading a small
//! TOML file per scan.

use crossbeam_channel::{bounded, Receiver, TryRecvError};
use squid_core::progress::{CancelToken, Progress};
use squid_engine::Engine;

pub struct JobHandle<T> {
    rx: Receiver<Result<T, String>>,
    cancel: CancelToken,
    label: String,
}

impl<T> JobHandle<T> {
    /// Non-blocking peek: `None` while the worker is still running.
    pub fn poll(&self) -> Option<Result<T, String>> {
        match self.rx.try_recv() {
            Ok(value) => Some(value),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(Err("the worker thread disappeared".to_string()))
            }
        }
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn is_finished(&self) -> bool {
        self.poll().is_some()
    }
}

/// Run `work` on a worker thread. The returned [`Progress`] is the handle the
/// UI drains for live feedback.
pub fn spawn<T, F>(label: &str, work: F) -> (JobHandle<T>, Progress)
where
    T: Send + 'static,
    F: FnOnce(&Engine, &Progress, &CancelToken) -> squid_core::Result<T> + Send + 'static,
{
    let progress = Progress::new();
    let ui_progress = progress.clone();
    let cancel = CancelToken::new();
    let worker_cancel = cancel.clone();
    let (tx, rx) = bounded::<Result<T, String>>(1);
    let label = label.to_string();
    let thread_label = label.clone();
    std::thread::Builder::new()
        .name(format!("squid-{label}"))
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let engine = Engine::new();
                work(&engine, &progress, &worker_cancel)
            }));
            let value = match result {
                Ok(inner) => inner.map_err(|err| err.to_string()),
                Err(panic) => {
                    let message = panic
                        .downcast_ref::<&str>()
                        .map(|text| text.to_string())
                        .or_else(|| panic.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "unknown error".to_string());
                    Err(format!("internal error: {message}"))
                }
            };
            let _ = tx.send(value);
        })
        .expect("spawn worker thread");
    (
        JobHandle {
            rx,
            cancel,
            label: thread_label,
        },
        ui_progress,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_reports_its_value() {
        let (job, progress) = spawn("test", |_engine, _progress, _cancel| Ok(41u32 + 1));
        loop {
            let _ = progress.drain();
            if let Some(value) = job.poll() {
                assert_eq!(value.expect("ok"), 42u32);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn a_panicking_job_becomes_an_error_instead_of_a_hang() {
        let (job, _progress) = spawn(
            "boom",
            |_engine, _progress, _cancel| -> squid_core::Result<u32> {
                panic!("deliberate");
            },
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(value) = job.poll() {
                assert!(value.unwrap_err().contains("deliberate"));
                break;
            }
            assert!(std::time::Instant::now() < deadline, "job never finished");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn cancellation_reaches_the_worker() {
        let (job, _progress) = spawn("cancel", |_engine, _progress, cancel| {
            for _ in 0..1_000 {
                if cancel.is_cancelled() {
                    return Err(squid_core::SquidError::Cancelled);
                }
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Ok(0u32)
        });
        std::thread::sleep(std::time::Duration::from_millis(20));
        job.cancel();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(value) = job.poll() {
                assert!(value.unwrap_err().contains("cancel"));
                break;
            }
            assert!(std::time::Instant::now() < deadline, "cancel ignored");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}
