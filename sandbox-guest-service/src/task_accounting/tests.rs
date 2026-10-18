use super::*;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(1)
        .build()
        .unwrap()
}

struct Payload {
    meter: Meter,
    blocking: bool,
    dropped: Arc<AtomicBool>,
}
impl Drop for Payload {
    fn drop(&mut self) {
        let state = self.meter.lock();
        assert!(
            if self.blocking {
                state.blocking.count.live
            } else {
                state.asynchronous.count.live
            } > 0
        );
        self.dropped.store(true, Ordering::SeqCst);
    }
}

fn payload(meter: &Meter, blocking: bool) -> (Payload, Arc<AtomicBool>) {
    let dropped = Arc::new(AtomicBool::new(false));
    (
        Payload {
            meter: meter.clone(),
            blocking,
            dropped: dropped.clone(),
        },
        dropped,
    )
}

fn empty(meter: &Meter) {
    let s = meter.lock();
    assert!(s.exact);
    assert_eq!(
        (
            s.total.live,
            s.asynchronous.count.live,
            s.blocking.count.live
        ),
        (0, 0, 0)
    );
    assert!(
        s.asynchronous
            .roles
            .iter()
            .chain(&s.blocking.roles)
            .all(|role| role.count.live == 0)
    );
}

#[test]
fn unpolled_async_abort_destroys_captures_before_credit() {
    let rt = runtime();
    let meter = Meter::default();
    let (payload, dropped) = payload(&meter, false);
    let task = rt.spawn(meter.track(AsyncTask::CommandDriver, async move {
        let _payload = payload;
        std::future::pending::<()>().await;
    }));
    assert_eq!(meter.lock().total.live, 1);
    task.abort();
    assert!(rt.block_on(task).unwrap_err().is_cancelled());
    assert!(dropped.load(Ordering::SeqCst));
    empty(&meter);
}

#[test]
fn async_success_and_panic_settle_once() {
    let rt = runtime();
    let meter = Meter::default();
    assert_eq!(
        rt.block_on(rt.spawn(meter.track(AsyncTask::Connection, async { 17 })))
            .unwrap(),
        17
    );
    empty(&meter);
    let (payload, dropped) = payload(&meter, false);
    let task = rt.spawn(meter.track(AsyncTask::CommandRegistry, async move {
        let _payload = payload;
        panic!("isolated expected async panic");
    }));
    assert!(rt.block_on(task).unwrap_err().is_panic());
    assert!(dropped.load(Ordering::SeqCst));
    empty(&meter);
}

// The running closure holds the sole blocking thread until explicitly released.
fn hold(meter: &Meter) -> (JoinHandle<()>, mpsc::Sender<()>) {
    let (entered, began) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    let running = meter.blocking(BlockingTask::FileOperation, move || {
        entered.send(()).unwrap();
        gate.recv_timeout(Duration::from_secs(10)).unwrap();
    });
    began.recv_timeout(Duration::from_secs(5)).unwrap();
    (running, release)
}

#[test]
fn queued_blocking_abort_retains_charge_until_actual_destruction() {
    let rt = runtime();
    let _entered = rt.enter();
    let meter = Meter::default();
    let (running, release) = hold(&meter);
    let executed = Arc::new(AtomicBool::new(false));
    let seen = executed.clone();
    let (payload, dropped) = payload(&meter, true);
    let queued = meter.blocking(BlockingTask::AccountLookup, move || {
        let _payload = payload;
        seen.store(true, Ordering::SeqCst);
    });
    queued.abort();
    assert_eq!(meter.lock().blocking.count.live, 2);
    assert!(!dropped.load(Ordering::SeqCst));
    release.send(()).unwrap();
    rt.block_on(running).unwrap();
    assert!(rt.block_on(queued).unwrap_err().is_cancelled());
    assert!(!executed.load(Ordering::SeqCst));
    assert!(dropped.load(Ordering::SeqCst));
    empty(&meter);
}

#[test]
fn lost_waiter_and_runtime_timeout_do_not_credit_running_blocking_work() {
    let rt = runtime();
    let meter = Meter::default();
    let (running, release) = {
        let _entered = rt.enter();
        hold(&meter)
    };
    let waiter = rt.spawn(meter.track(AsyncTask::CommandPreparation, async move {
        running.await.unwrap();
    }));
    waiter.abort();
    assert!(rt.block_on(waiter).unwrap_err().is_cancelled());
    assert_eq!(meter.lock().asynchronous.count.live, 0);
    assert_eq!(meter.lock().blocking.count.live, 1);
    rt.shutdown_timeout(Duration::ZERO);
    assert_eq!(meter.lock().blocking.count.live, 1);
    release.send(()).unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    while meter.lock().blocking.count.live != 0 {
        assert!(
            Instant::now() < until,
            "released blocking task never settled"
        );
        std::thread::yield_now();
    }
    empty(&meter);
}

#[test]
fn blocking_panic_destroys_captures_before_credit() {
    let rt = runtime();
    let _entered = rt.enter();
    let meter = Meter::default();
    let (payload, dropped) = payload(&meter, true);
    let job = meter.blocking(BlockingTask::CommandDecode, move || {
        let _payload = payload;
        panic!("isolated expected blocking panic");
    });
    assert!(rt.block_on(job).unwrap_err().is_panic());
    assert!(dropped.load(Ordering::SeqCst));
    empty(&meter);
}

#[test]
fn combined_peak_is_not_sum_of_disjoint_group_peaks() {
    let rt = runtime();
    let _entered = rt.enter();
    let meter = Meter::default();
    let tasks: Vec<_> = (0..4)
        .map(|_| rt.spawn(meter.track(AsyncTask::Connection, std::future::pending::<()>())))
        .collect();
    for task in tasks {
        task.abort();
        assert!(rt.block_on(task).unwrap_err().is_cancelled());
    }
    let (running, release) = hold(&meter);
    let queued: Vec<_> = (0..3)
        .map(|_| meter.blocking(BlockingTask::BodyEncode, || ()))
        .collect();
    let s = *meter.lock();
    assert_eq!(
        (
            s.asynchronous.count.peak,
            s.blocking.count.peak,
            s.total.peak
        ),
        (4, 4, 4)
    );
    release.send(()).unwrap();
    rt.block_on(running).unwrap();
    for task in queued {
        rt.block_on(task).unwrap();
    }
    empty(&meter);
}

#[test]
fn one_meter_covers_concurrent_runtimes_without_resetting_peaks() {
    let meter = Meter::default();
    let first = runtime();
    let second = runtime();
    let tasks: Vec<_> = [&first, &second]
        .into_iter()
        .map(|rt| rt.spawn(meter.track(AsyncTask::Connection, std::future::pending::<()>())))
        .collect();
    assert_eq!(meter.lock().total, Count { live: 2, peak: 2 });
    for (rt, task) in [&first, &second].into_iter().zip(tasks) {
        task.abort();
        assert!(rt.block_on(task).unwrap_err().is_cancelled());
    }
    empty(&meter);
    assert_eq!(meter.lock().total.peak, 2);
}

#[test]
fn returned_values_are_outside_task_lifetime() {
    let rt = runtime();
    let _entered = rt.enter();
    let meter = Meter::default();
    let owner = Arc::new(());
    let held = owner.clone();
    let value = rt
        .block_on(meter.blocking(BlockingTask::FileDownload, move || held))
        .unwrap();
    empty(&meter);
    assert_eq!(Arc::strong_count(&owner), 2);
    drop(value);
    assert_eq!(Arc::strong_count(&owner), 1);
}

#[test]
fn overflow_underflow_and_poison_are_sticky_inexact_observations() {
    let meter = Meter::default();
    meter.lock().total.live = usize::MAX;
    let guard = meter.begin(Kind::Async(AsyncTask::Connection));
    assert!(!meter.lock().exact);
    drop(guard);
    assert!(!meter.lock().exact);
    let meter = Meter::default();
    meter.change(Kind::Blocking(BlockingTask::BodyEncode), false);
    assert!(!meter.lock().exact);
    assert_eq!(meter.lock().total.live, 0);
    let meter = Meter::default();
    assert!(
        std::panic::catch_unwind(|| {
            let _lock = meter.0.lock().unwrap();
            panic!("isolated expected poisoned observation");
        })
        .is_err()
    );
    assert!(!meter.lock().exact);
    let guard = meter.begin(Kind::Async(AsyncTask::Connection));
    drop(guard);
    assert_eq!(meter.lock().total.live, 0);
    assert!(!meter.lock().exact);
}

#[test]
fn failed_submission_destroys_queued_captures_before_credit() {
    let meter = Meter::default();
    let (payload, dropped) = payload(&meter, true);
    assert!(std::panic::catch_unwind(|| meter.blocking(BlockingTask::FileUpload, move || drop(payload))).is_err());
    assert!(dropped.load(Ordering::SeqCst));
    empty(&meter);
}
