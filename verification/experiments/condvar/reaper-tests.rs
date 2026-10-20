#[cfg(test)]
mod condvar_contract_trial {
    use super::*;
    use std::sync::{MutexGuard, TryLockError, mpsc};

    fn original_reaper() -> Arc<Reaper> {
        Arc::new(Reaper {
            active: AtomicUsize::new(0),
            pending: Mutex::new(Vec::with_capacity(MAX_HELPERS)),
            wake: Condvar::new(),
        })
    }

    #[test]
    fn original_retain_wakes_waiter_with_original_pair_and_reacquired_guard() {
        let _serial = crate::linux::PROCESS_TEST_LOCK.lock().unwrap();
        let reaper = original_reaper();
        let slot = Slot::reserve(&reaper).unwrap();
        let child = std::process::Command::new("/bin/sh")
            .args(["-c", "IFS= read -r line"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let original_pid = child.id();
        let (ready, started) = mpsc::channel();
        let worker = Arc::clone(&reaper);
        let waiter = thread::spawn(move || {
            let mut pending = worker.pending.lock().unwrap();
            let original_queue = std::ptr::from_ref(&*pending);
            ready.send(()).unwrap();
            // As in Reaper::run, a wake-up is followed by another predicate check.
            while pending.is_empty() {
                pending = worker.wake.wait(pending).unwrap();
            }
            assert_eq!(std::ptr::from_ref(&*pending), original_queue);
            assert!(matches!(
                worker.pending.try_lock(),
                Err(TryLockError::WouldBlock)
            ));
            assert_eq!(pending.len(), 1);
            let (child, slot) = pending.pop().unwrap();
            assert_eq!(child.id(), original_pid);
            assert!(Arc::ptr_eq(&slot.0, &worker));
            assert_eq!(worker.active.load(Ordering::Acquire), 1);
            (child, slot)
        });
        started.recv().unwrap();
        // This real production method must first acquire the original pending lock.
        slot.retain(child);
        let (mut child, slot) = waiter.join().unwrap();
        assert_eq!(reaper.active.load(Ordering::Acquire), 1);
        child.kill().unwrap();
        child.wait().unwrap();
        drop(slot);
        assert_eq!(reaper.active.load(Ordering::Acquire), 0);
    }

    #[test]
    fn timeout_reply_holds_the_original_mutex() {
        let original = Mutex::new(19_u64);
        let alternate = Mutex::new(23_u64);
        let original_address = {
            let held = original.lock().unwrap();
            std::ptr::from_ref(&*held)
        };
        let wake = Condvar::new();
        let guard = original.lock().unwrap();
        let (returned, _timeout) = wake.wait_timeout(guard, Duration::ZERO).unwrap();
        assert!(
            std::ptr::eq(std::ptr::from_ref(&*returned), original_address),
            "Condvar reply changed the original mutex"
        );
        assert!(matches!(original.try_lock(), Err(TryLockError::WouldBlock)));
        drop(returned);
        assert_eq!(*original.lock().unwrap(), 19);
        assert_eq!(*alternate.lock().unwrap(), 23);
    }

    #[test]
    fn poisoned_timeout_error_keeps_the_entire_tuple_and_original_lock() {
        let pair = Arc::new((Mutex::new(false), Condvar::new()));
        let worker = Arc::clone(&pair);
        let (ready, started) = mpsc::channel();
        let waiter = thread::spawn(move || {
            let mut guard = worker.0.lock().unwrap();
            ready.send(()).unwrap();
            loop {
                match worker.1.wait_timeout(guard, Duration::from_hours(1)) {
                    Ok((next, _timeout)) => {
                        guard = next;
                        assert!(
                            !*guard,
                            "poisoned write must not return an unpoisoned guard"
                        );
                    }
                    Err(error) => {
                        let (returned, _timeout) = error.into_inner();
                        assert!(*returned);
                        assert!(matches!(worker.0.try_lock(), Err(TryLockError::WouldBlock)));
                        drop(returned);
                        assert!(worker.0.is_poisoned());
                        break;
                    }
                }
            }
        });
        started.recv().unwrap();
        let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut original = pair.0.lock().unwrap();
            *original = true;
            panic!("controlled original writer unwind");
        }));
        assert!(failure.is_err());
        pair.1.notify_one();
        waiter.join().unwrap();
    }

    struct WaitDuringUnwind<'a> {
        guard: Option<MutexGuard<'a, bool>>,
        wake: &'a Condvar,
        ready: mpsc::Sender<()>,
    }
    impl Drop for WaitDuringUnwind<'_> {
        fn drop(&mut self) {
            let mut guard = self.guard.take().unwrap();
            self.ready.send(()).unwrap();
            while !*guard {
                guard = self.wake.wait(guard).unwrap();
            }
            // The guard was originally acquired before this unwind began.
            drop(guard);
        }
    }

    #[test]
    fn wait_preserves_the_guards_original_panic_marker() {
        let pair = Arc::new((Mutex::new(false), Condvar::new()));
        let worker = Arc::clone(&pair);
        let (ready, started) = mpsc::channel();
        let waiter = thread::spawn(move || {
            let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _waiting = WaitDuringUnwind {
                    guard: Some(worker.0.lock().unwrap()),
                    wake: &worker.1,
                    ready,
                };
                panic!("controlled unwind after original acquisition");
            }));
            assert!(failure.is_err());
        });
        started.recv().unwrap();
        // Condvar suspension is not Guard::drop: this acquisition is unpoisoned.
        {
            let mut original = pair.0.lock().unwrap();
            *original = true;
        }
        pair.1.notify_one();
        waiter.join().unwrap();
        assert!(
            pair.0.is_poisoned(),
            "wait reset the original guard panic marker"
        );
    }
}
