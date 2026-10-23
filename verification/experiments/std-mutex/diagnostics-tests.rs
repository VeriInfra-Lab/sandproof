#[cfg(test)]
mod std_mutex_contract_trial {
    use super::*;
    use std::sync::{Arc, TryLockError};
    use std::panic::{catch_unwind, AssertUnwindSafe};

    fn poison(value: &Mutex<u64>) {
        let failed = catch_unwind(AssertUnwindSafe(|| {
            let mut guard = value.lock().unwrap();
            *guard = 19;
            panic!("controlled original-lock unwind");
        }));
        assert!(failed.is_err());
    }

    #[test]
    fn aliases_lock_the_original_cell_and_other_cells_remain_available() {
        let original = Arc::new(Mutex::new(vec![1_u64]));
        let alias = Arc::clone(&original);
        let other = Mutex::new(vec![1_u64]);
        let mut guard = original.lock().unwrap();
        guard.push(2);
        assert!(matches!(alias.try_lock(), Err(TryLockError::WouldBlock)));
        assert_eq!(*other.try_lock().unwrap(), vec![1]);
        drop(guard);
        assert_eq!(*alias.try_lock().unwrap(), vec![1, 2]);
    }

    #[test]
    fn guard_drop_unlocks_without_dropping_the_owned_payload() {
        let pool = Arc::new(tokio::sync::Semaphore::new(1));
        let permit = Arc::clone(&pool).try_acquire_owned().unwrap();
        let original = Mutex::new(Some(permit));
        {
            let _guard = original.lock().unwrap();
            assert_eq!(pool.available_permits(), 0);
        }
        assert_eq!(pool.available_permits(), 0);
        let returned = original.lock().unwrap().take().unwrap();
        assert_eq!(pool.available_permits(), 0);
        drop(returned);
        assert_eq!(pool.available_permits(), 1);
    }

    #[test]
    fn poison_error_still_owns_the_original_guard_and_mutated_value() {
        let original = Mutex::new(3_u64);
        poison(&original);
        let mut guard = original.lock().err().unwrap().into_inner();
        assert_eq!(*guard, 19);
        assert!(matches!(original.try_lock(), Err(TryLockError::WouldBlock)));
        *guard = 23;
        drop(guard);
        let failure = original.try_lock().err().unwrap();
        let TryLockError::Poisoned(error) = failure else {
            panic!("poisoned original guard was not returned");
        };
        assert_eq!(**error.get_ref(), 23);
        assert!(original.is_poisoned());
    }

    #[test]
    fn mapping_a_poison_error_releases_its_guard_without_rollback() {
        let original = Mutex::new(3_u64);
        poison(&original);
        let refused = original.lock().map_err(|_| "unavailable");
        assert!(matches!(refused, Err("unavailable")));
        let TryLockError::Poisoned(error) = original.try_lock().err().unwrap() else {
            panic!("discarded poison error kept the original lock");
        };
        assert_eq!(**error.get_ref(), 19);
    }

    #[test]
    fn original_unique_get_mut_keeps_the_borrow_and_existing_poison() {
        let mut original = SerialMutex::new(3_u64);
        poison(&original.0);
        let borrowed = original.get_mut();
        assert!(borrowed.is_err(), "original poisoned mutable borrow was silently accepted");
        *borrowed.err().unwrap().into_inner() = 29;
        assert!(original.0.is_poisoned());
        assert_eq!(*original.lock().err().unwrap().into_inner(), 29);
    }

    #[test]
    fn a_full_notice_does_not_replace_the_original_lock_attempt() {
        let original = SerialMutex::new(3_u64);
        let (notice, observed) = std::sync::mpsc::sync_channel(0);
        let result = measure_with_lock_notice(Some(notice), || {
            *original.lock().unwrap() += 1;
            41_u64
        });
        assert_eq!(result.result, 41);
        assert_eq!(result.lock.acquisitions, 1);
        assert_eq!(*original.0.try_lock().unwrap(), 4);
        assert!(observed.try_recv().is_err());
    }

    #[test]
    fn original_guard_released_before_diagnostic_failure() {
        let original = SerialMutex::new(3_u64);
        measure(|| {
            let guard = original.lock().unwrap();
            CURRENT.with(|current| {
                let _borrow = current.borrow();
                let failed = catch_unwind(AssertUnwindSafe(|| drop(guard)));
                assert!(failed.is_err());
                assert!(
                    original.0.try_lock().is_ok(),
                    "original mutex not released cleanly before diagnostic failure"
                );
            });
        });
        assert!(!original.0.is_poisoned());
    }

    #[test]
    fn acquiring_during_unwind_can_change_data_without_setting_poison() {
        struct ChangeDuringUnwind<'a>(&'a Mutex<u64>, &'a std::sync::atomic::AtomicBool);
        impl Drop for ChangeDuringUnwind<'_> {
            fn drop(&mut self) {
                self.1.store(std::thread::panicking(), std::sync::atomic::Ordering::SeqCst);
                *self.0.lock().unwrap() = 37;
            }
        }
        let original = Mutex::new(3_u64);
        let saw_unwind = std::sync::atomic::AtomicBool::new(false);
        let failed = catch_unwind(AssertUnwindSafe(|| {
            let _change = ChangeDuringUnwind(&original, &saw_unwind);
            panic!("controlled outer unwind before original mutex acquisition");
        }));
        assert!(failed.is_err());
        assert!(saw_unwind.load(std::sync::atomic::Ordering::SeqCst));
        assert!(!original.is_poisoned());
        assert_eq!(*original.lock().unwrap(), 37);
    }

    #[test]
    fn unique_borrow_does_not_unlock_forgotten_original_guard() {
        let mut original = SerialMutex::new(3_u64);
        poison(&original.0);
        let guard = original.lock().err().unwrap().into_inner();
        std::mem::forget(guard);
        let borrowed = original.get_mut();
        assert!(borrowed.is_err());
        *borrowed.err().unwrap().into_inner() = 41;
        assert!(matches!(original.0.try_lock(), Err(TryLockError::WouldBlock)));
        assert_eq!(*original.get_mut().err().unwrap().into_inner(), 41);
    }

}
