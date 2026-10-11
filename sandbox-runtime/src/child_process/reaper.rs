//! Fixed ownership queue: waiting for exit never creates another OS thread.
use std::{
    io,
    process::Child,
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};
const MAX_HELPERS: usize = 64;
struct Reaper {
    active: AtomicUsize,
    pending: Mutex<Vec<(Child, Slot)>>,
    wake: Condvar,
}
pub(super) struct Slot(Arc<Reaper>);
impl Slot {
    pub fn acquire() -> io::Result<Self> {
        static REAPER: OnceLock<Result<Arc<Reaper>, io::Error>> = OnceLock::new();
        let reaper = REAPER
            .get_or_init(|| {
                let reaper = Arc::new(Reaper {
                    active: AtomicUsize::new(0),
                    pending: Mutex::new(Vec::with_capacity(MAX_HELPERS)),
                    wake: Condvar::new(),
                });
                let worker = Arc::clone(&reaper);
                thread::Builder::new()
                    .name("host-helper-reaper".into())
                    .spawn(move || worker.run())?;
                Ok(reaper)
            })
            .as_ref()
            .map_err(|error| io::Error::new(error.kind(), "cannot start host helper reaper"))?;
        Self::reserve(reaper)
    }
    fn reserve(reaper: &Arc<Reaper>) -> io::Result<Self> {
        reaper
            .active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < MAX_HELPERS).then(|| n + 1)
            })
            .map_err(|_| {
                io::Error::new(io::ErrorKind::WouldBlock, "host helper capacity exhausted")
            })?;
        Ok(Self(Arc::clone(reaper)))
    }
    pub fn retain(self, child: Child) {
        let reaper = Arc::clone(&self.0);
        // Every entry owns a pre-spawn slot, so length is at most MAX_HELPERS.
        reaper.pending.lock().unwrap().push((child, self));
        reaper.wake.notify_one();
    }
}
impl Drop for Slot {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::AcqRel);
    }
}
impl Reaper {
    fn reap_ready(pending: &mut Vec<(Child, Slot)>) {
        let mut i = 0;
        while i < pending.len() {
            if matches!(pending[i].0.try_wait(), Ok(Some(_))) {
                pending.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }
    fn run(&self) {
        let mut pending = self.pending.lock().unwrap();
        loop {
            Self::reap_ready(&mut pending);
            if pending.is_empty() {
                pending = self.wake.wait(pending).unwrap();
            } else {
                pending = self
                    .wake
                    .wait_timeout(pending, Duration::from_millis(10))
                    .unwrap()
                    .0;
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};
    #[test]
    fn unknown_child_retains_capacity_until_exact_exit_is_reaped() {
        let reaper = Arc::new(Reaper {
            active: AtomicUsize::new(0),
            pending: Mutex::new(Vec::with_capacity(MAX_HELPERS)),
            wake: Condvar::new(),
        });
        let mut slots: Vec<_> = (0..MAX_HELPERS)
            .map(|_| Slot::reserve(&reaper).unwrap())
            .collect();
        assert!(Slot::reserve(&reaper).is_err());
        let child = Command::new("/bin/sh")
            .args(["-c", "IFS= read -r line"])
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        slots.pop().unwrap().retain(child);
        let mut pending = reaper.pending.lock().unwrap();
        Reaper::reap_ready(&mut pending);
        assert_eq!(pending.len(), 1);
        assert!(Slot::reserve(&reaper).is_err());
        pending[0].0.kill().unwrap();
        pending[0].0.wait().unwrap();
        Reaper::reap_ready(&mut pending);
        assert!(pending.is_empty());
        assert_eq!(reaper.active.load(Ordering::Acquire), MAX_HELPERS - 1);
        assert!(Slot::reserve(&reaper).is_ok());
    }
}
