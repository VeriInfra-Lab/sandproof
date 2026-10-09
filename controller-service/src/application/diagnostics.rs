//! Opt-in, thread-local measurement of the existing serial boundary.
//!
//! No identifiers, credentials, history or user payloads are retained. This is
//! a component diagnostic, not a telemetry service or a concurrency policy.
use std::{
    cell::RefCell,
    ops::{Deref, DerefMut},
    sync::{Mutex, MutexGuard, PoisonError, mpsc::SyncSender},
    time::{Duration, Instant},
};

/// Bounded totals for one synchronous call on one thread, not a histogram.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LockTiming {
    pub acquisitions: u64,
    pub wait: Duration,
    pub held: Duration,
}

/// Only durations and the caller's ordinary result; never a log of inputs.
pub struct Measurement<T> {
    pub result: T,
    pub elapsed: Duration,
    pub lock: LockTiming,
    pub host_calls: u64,
    pub host_elapsed: Duration,
}

struct Scope {
    timing: LockTiming,
    notice: Option<SyncSender<()>>,
    host_calls: u64,
    host_elapsed: Duration,
}
thread_local! {
    static CURRENT: RefCell<Option<Scope>> = const { RefCell::new(None) };
}

struct Restore(Option<Scope>);
impl Drop for Restore {
    fn drop(&mut self) {
        CURRENT.with(|current| *current.borrow_mut() = self.0.take());
    }
}

/// Measure a synchronous call. Nested calls have separate totals; the parent
/// still includes their elapsed time. This does not follow spawned threads.
/// # Panics
/// Propagates the operation's panic after restoring the prior diagnostic scope.
pub fn measure<T>(operation: impl FnOnce() -> T) -> Measurement<T> {
    measure_with_lock_notice(None, operation)
}

/// As above, with a nonblocking, payload-free notice immediately BEFORE each
/// lock attempt. A full/disconnected diagnostic channel cannot affect work.
/// The notice is NOT evidence of acquisition, authorization or dispatch.
/// # Panics
/// Propagates the operation's panic; an internally lost scope is an invariant
/// violation. In either case the prior diagnostic scope is restored.
pub fn measure_with_lock_notice<T>(
    notice: Option<SyncSender<()>>,
    operation: impl FnOnce() -> T,
) -> Measurement<T> {
    let _restore = Restore(CURRENT.with(|current| {
        current.replace(Some(Scope {
            timing: LockTiming::default(),
            notice,
            host_calls: 0,
            host_elapsed: Duration::ZERO,
        }))
    }));
    let started = Instant::now();
    let result = operation();
    let elapsed = started.elapsed();
    let (lock, host_calls, host_elapsed) = CURRENT.with(|current| {
        let scope = current.borrow();
        let scope = scope.as_ref().expect("active measurement");
        (scope.timing, scope.host_calls, scope.host_elapsed)
    });
    Measurement {
        result,
        elapsed,
        lock,
        host_calls,
        host_elapsed,
    }
}

/// Time an explicitly selected synchronous host-port call. No automatic
/// interception: consumers must state whether the backend is controlled or
/// real. Nested host calls would double count and must not be instrumented.
pub fn host_call<T>(operation: impl FnOnce() -> T) -> T {
    struct Sample(Instant);
    impl Drop for Sample {
        fn drop(&mut self) {
            let elapsed = self.0.elapsed();
            CURRENT.with(|current| {
                if let Some(scope) = current.borrow_mut().as_mut() {
                    scope.host_calls = scope.host_calls.saturating_add(1);
                    scope.host_elapsed = scope.host_elapsed.saturating_add(elapsed);
                }
            });
        }
    }
    let _sample = Sample(Instant::now());
    operation()
}

/// Same mutex, same acquisition and poison error; no try-lock or second lock.
pub(super) struct SerialMutex<T>(Mutex<T>);
impl<T> SerialMutex<T> {
    pub(super) fn new(value: T) -> Self {
        Self(Mutex::new(value))
    }

    pub(super) fn get_mut(&mut self) -> std::sync::LockResult<&mut T> {
        self.0.get_mut()
    }

    pub(super) fn lock(&self) -> Result<SerialGuard<'_, T>, PoisonError<MutexGuard<'_, T>>> {
        let started = CURRENT.with(|current| {
            let scope = current.borrow();
            scope.as_ref().map(|scope| {
                let started = Instant::now();
                if let Some(notice) = &scope.notice {
                    let _ = notice.try_send(());
                }
                started
            })
        });
        let guard = self.0.lock()?;
        let acquired = started.map(|started| (started.elapsed(), Instant::now()));
        Ok(SerialGuard {
            guard: Some(guard),
            acquired,
        })
    }
}

pub(super) struct SerialGuard<'a, T> {
    guard: Option<MutexGuard<'a, T>>,
    acquired: Option<(Duration, Instant)>,
}
impl<T> Deref for SerialGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.guard.as_ref().expect("guard is present until drop")
    }
}
impl<T> DerefMut for SerialGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.guard.as_mut().expect("guard is present until drop")
    }
}
impl<T> Drop for SerialGuard<'_, T> {
    fn drop(&mut self) {
        let timing = self
            .acquired
            .map(|(wait, acquired)| (wait, acquired.elapsed()));
        // Do not write diagnostics or notify consumers while holding Inner.
        drop(self.guard.take());
        if let Some((wait, held)) = timing {
            CURRENT.with(|current| {
                if let Some(scope) = current.borrow_mut().as_mut() {
                    scope.timing.acquisitions = scope.timing.acquisitions.saturating_add(1);
                    scope.timing.wait = scope.timing.wait.saturating_add(wait);
                    scope.timing.held = scope.timing.held.saturating_add(held);
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totals_are_bounded_isolated_and_unwinding_preserves_poison() {
        let mutex = SerialMutex::new(0);
        let (sender, _) = std::sync::mpsc::sync_channel(0);
        let first = measure_with_lock_notice(Some(sender), || *mutex.lock().unwrap() += 1);
        assert_eq!(first.lock.acquisitions, 1);
        assert_eq!(*mutex.lock().unwrap(), 1);
        assert_eq!(measure(|| ()).lock, LockTiming::default());
        let nested = measure(|| measure(|| *mutex.lock().unwrap() += 1));
        assert_eq!(nested.lock.acquisitions, 0);
        assert_eq!(nested.result.lock.acquisitions, 1);
        let panic = std::panic::catch_unwind(|| {
            measure(|| {
                let _guard = mutex.lock().unwrap();
                panic!("controlled diagnostic unwind");
            })
        });
        assert!(panic.is_err());
        assert!(mutex.lock().is_err());
        assert_eq!(measure(|| ()).lock, LockTiming::default());
    }
}
