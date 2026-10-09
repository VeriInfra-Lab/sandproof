//! B7: real application/SQLite, controlled host/time; no HTTP or KVM claim.
use super::*;
use sandproof_controller_service::application::diagnostics::{
    Measurement, measure, measure_with_lock_notice,
};
use std::{sync::mpsc, thread, time::Duration};

const WAIT: Duration = Duration::from_secs(15);
const HOST_STALL: Duration = Duration::from_millis(25);

pub(super) struct SlowOperation {
    armed: AtomicBool,
    entered: mpsc::SyncSender<()>,
    release: Mutex<mpsc::Receiver<()>>,
}
impl SlowOperation {
    pub(super) fn pause(&self, action: &WorkAction<core::EffectId>) {
        if matches!(action.command.command(), core::Command::PrepareVm { .. })
            && self.armed.swap(false, Ordering::SeqCst)
        {
            self.entered.send(()).unwrap();
            self.release.lock().unwrap().recv_timeout(WAIT).unwrap();
        }
    }
}

// Parent assertion failures must not leave the controlled host parked forever.
struct Release(mpsc::SyncSender<()>);
impl Drop for Release {
    fn drop(&mut self) {
        let _ = self.0.try_send(());
    }
}

#[derive(Clone, Copy, Debug)]
enum Probe {
    Authorize,
    ExpiredAuthorize,
    Renew,
    Expire,
}

impl Probe {
    fn call(
        self,
        app: &ControllerApplication<TestHost>,
        external: &str,
        credential: &str,
    ) -> Result<(), ApplicationError> {
        match self {
            Self::Authorize | Self::ExpiredAuthorize => {
                app.authorize_data(credential, DataKind::Exec).map(|_| ())
            }
            Self::Renew => app.set_timeout("1", external, 10),
            Self::Expire => app.expire_due_report().map(|report| {
                assert_eq!(report.completed, 1);
                assert_eq!(report.unresolved_sandboxes, 0);
            }),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Authorize => "authorize",
            Self::ExpiredAuthorize => "expired-authorize",
            Self::Renew => "renew",
            Self::Expire => "expire",
        }
    }
}

fn fixture(directory: &TempDir, host: TestHost) -> ControllerApplication<TestHost> {
    let path = directory.path().join("controller.db");
    let mut maintenance = support::complete_fixture::maintenance();
    // Fixed finite deployment policy; do not restore an unbounded history mode.
    maintenance.max_tail_inputs = 64;
    let bounded = support::complete_fixture::host(
        &path,
        support::NAMESPACE,
        2,
        2 * 1024 * 1024,
        support::CLEANUP_BYTES + 1000,
    );
    let clock = host.clock.clone();
    ControllerApplication::new(
        ControllerJournal::open_complete(&path, support::NAMESPACE, bounded, maintenance, 0)
            .unwrap(),
        host,
        config(),
    )
    .with_clock(clock)
}

fn history(directory: &TempDir) -> (i64, i64, i64) {
    let connection = rusqlite::Connection::open_with_flags(
        directory.path().join("controller.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    connection.query_row(
        "SELECT input_cut,deleted_through,(SELECT COUNT(*) FROM controller_inputs WHERE sequence>input_cut) FROM controller_checkpoint,controller_checkpoint_gc WHERE controller_checkpoint.id=1 AND controller_checkpoint_gc.id=1",
        [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap()
}

fn report<T>(label: &str, cycles: u32, sample: &Measurement<T>) {
    // No Debug of results: successful create results contain guest credentials.
    eprintln!(
        "B7 backend=controlled active=2 cycles={cycles} operation={label} request_us={} lock_wait_us={} lock_held_us={} acquisitions={} host_calls={} host_us={}",
        sample.elapsed.as_micros(),
        sample.lock.wait.as_micros(),
        sample.lock.held.as_micros(),
        sample.lock.acquisitions,
        sample.host_calls,
        sample.host_elapsed.as_micros()
    );
}

fn warm_history(
    directory: &TempDir,
    app: &ControllerApplication<TestHost>,
    clock: &TestClock,
    cycles: u32,
) -> (sandproof_controller_service::application::Session, i64) {
    let mut now = 1000_i64;
    for _ in 0..cycles {
        let session = app.create("1", request()).unwrap();
        app.kill("1", &session.sandbox_id).unwrap();
        now += 101;
        clock.set(now);
        assert_eq!(app.maintain().unwrap().retired_sandboxes, 1);
    }
    let healthy = app.create("1", request()).unwrap();
    assert!(
        app.authorize_data(&healthy.envd_access_token, DataKind::Exec)
            .is_ok()
    );
    let (cut, deleted, tail) = history(directory);
    assert!(tail <= 64);
    if cycles != 0 {
        assert!(
            cut > 0 && deleted > 0,
            "warm-up must actually rotate and prune checkpoints"
        );
    }
    eprintln!(
        "B7 history cycles={cycles} input_cut={cut} deleted_through={deleted} tail_inputs={tail} tail_limit=64"
    );
    (healthy, now)
}

fn scenario(probe: Probe, cycles: u32) {
    let directory = TempDir::new().unwrap();
    let clock = TestClock::at(1000);
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let slow = Arc::new(SlowOperation {
        armed: AtomicBool::new(false),
        entered: entered_tx,
        release: Mutex::new(release_rx),
    });
    let mut host = TestHost::new(clock.clone());
    host.slow = Some(slow.clone());
    let app = Arc::new(fixture(&directory, host));
    let (healthy, now) = warm_history(&directory, &app, &clock, cycles);

    slow.armed.store(true, Ordering::SeqCst);
    let holder_app = app.clone();
    let holder = thread::spawn(move || measure(|| holder_app.create("1", request())));
    entered_rx.recv_timeout(WAIT).unwrap();
    let release = Release(release_tx);
    let (attempt_tx, attempt_rx) = mpsc::sync_channel(1);
    let (completed_tx, completed_rx) = mpsc::sync_channel(1);
    let probe_app = app.clone();
    let external = healthy.sandbox_id.clone();
    let credential = healthy.envd_access_token.clone();
    let waiter = thread::spawn(move || {
        let sample = measure_with_lock_notice(Some(attempt_tx), || {
            probe.call(&probe_app, &external, &credential)
        });
        assert!(completed_tx.send(sample).is_ok());
    });
    attempt_rx.recv_timeout(WAIT).unwrap();
    assert!(
        matches!(completed_rx.try_recv(), Err(mpsc::TryRecvError::Empty)),
        "a second request must not bypass the parked management boundary"
    );
    clock.set(
        now + match probe {
            Probe::Authorize | Probe::Renew => 1000,
            Probe::ExpiredAuthorize | Probe::Expire => 2000,
        },
    );
    // A fixed, bounded observation window while the host remains gated, not
    // a random sleep hoping to hit an interleaving. Admission was acknowledged
    // above, and the parent controls the only release signal.
    assert!(matches!(
        completed_rx.recv_timeout(HOST_STALL),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    drop(release);
    let created = holder.join().unwrap();
    let sample = completed_rx.recv_timeout(WAIT).unwrap();
    waiter.join().unwrap();
    let created_session = created.result.as_ref().unwrap();
    assert_eq!(created.lock.acquisitions, 1);
    assert!(created.host_calls > 0);
    assert!(created.host_elapsed >= HOST_STALL);
    assert!(created.host_elapsed <= created.lock.held);
    assert_eq!(sample.lock.acquisitions, 1);
    assert!(sample.lock.wait + sample.lock.held <= sample.elapsed);
    let expected = if matches!(probe, Probe::ExpiredAuthorize) {
        Err(ApplicationError::NotFound)
    } else {
        Ok(())
    };
    assert_eq!(sample.result, expected);
    report("slow-create", cycles, &created);
    report(probe.name(), cycles, &sample);
    if matches!(probe, Probe::Renew) {
        assert_eq!(
            app.get("1", &healthy.sandbox_id).unwrap().info.end_at_ms,
            now + 11_000
        );
    }
    if matches!(probe, Probe::Expire) {
        assert_eq!(
            app.get("1", &healthy.sandbox_id),
            Err(ApplicationError::NotFound)
        );
        assert_eq!(app.maintain().unwrap().retired_sandboxes, 1);
    } else {
        app.kill("1", &healthy.sandbox_id).unwrap();
    }
    // The slow create published its lifetime AFTER the stall, so it is still live.
    assert!(app.get("1", &created_session.sandbox_id).is_ok());
    app.kill("1", &created_session.sandbox_id).unwrap();
    clock.set(now + 2101);
    app.maintain().unwrap();
    assert!(
        app.list("1", ListSelection::default())
            .unwrap()
            .items
            .is_empty()
    );
}

#[test]
fn cross_instance_waits_preserve_authority_renewal_and_expiry_with_bounded_history() {
    for cycles in [0, 12] {
        for probe in [
            Probe::Authorize,
            Probe::ExpiredAuthorize,
            Probe::Renew,
            Probe::Expire,
        ] {
            scenario(probe, cycles);
        }
    }
}
