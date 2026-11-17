use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

/// Shared cancel flag for one running job.
///
/// The worker checks [`Cancel::is_cancelled`] between files. External
/// programs are started in their own process group; [`Cancel::cancel`]
/// kills that group so a blocked read returns.
#[derive(Clone, Debug)]
pub struct Cancel {
    flag: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    pid: Arc<Mutex<Option<u32>>>,
}

impl Default for Cancel {
    fn default() -> Self {
        Self::new()
    }
}

impl Cancel {
    pub fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
            paused: Arc::new(AtomicBool::new(false)),
            pid: Arc::new(Mutex::new(None)),
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    pub fn check(&self) -> super::Result<()> {
        if self.is_cancelled() {
            Err(super::Error::Cancelled)
        } else {
            Ok(())
        }
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    /// Stop the job's process group until [`resume`](Self::resume).
    pub fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
        if let Some(pid) = *self.lock() {
            if pid > 1 {
                unsafe { libc::kill(-(pid as i32), libc::SIGSTOP) };
            }
        }
    }

    pub fn resume(&self) {
        self.paused.store(false, Ordering::SeqCst);
        if let Some(pid) = *self.lock() {
            if pid > 1 {
                unsafe { libc::kill(-(pid as i32), libc::SIGCONT) };
            }
        }
    }

    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
        if let Some(pid) = *self.lock() {
            if pid > 1 {
                // Negative pid: the process group started for this job.
                unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
            }
        }
    }

    pub(crate) fn register(&self, pid: u32) {
        *self.lock() = Some(pid);
        if self.is_cancelled() && pid > 1 {
            unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
        }
    }

    pub(crate) fn clear_pid(&self) {
        *self.lock() = None;
    }

    fn lock(&self) -> MutexGuard<'_, Option<u32>> {
        self.pid.lock().unwrap_or_else(|err| err.into_inner())
    }
}
