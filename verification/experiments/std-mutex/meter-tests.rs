#[cfg(test)]
mod std_mutex_meter_trial {
    use super::*;
    #[test]
    fn original_meter_recovers_same_snapshot_and_marks_it_inexact() {
        let meter = Meter::default();
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut state = meter.0.lock().unwrap();
            state.total.live = 3;
            state.total.peak = 5;
            panic!("controlled original meter unwind");
        }));
        assert!(failed.is_err());
        let recovered = meter.lock();
        assert!(!recovered.exact);
        assert_eq!(recovered.total, Count { live: 3, peak: 5 });
        assert!(matches!(meter.0.try_lock(), Err(std::sync::TryLockError::WouldBlock)));
        drop(recovered);
        assert!(meter.0.is_poisoned());
        assert!(!meter.lock().exact);
    }
}
