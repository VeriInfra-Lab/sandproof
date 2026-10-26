//! Current formal SQL/Application and original Unload advance. Host source and
//! capture providers remain controlled; this does not run a VM. Only the gate
//! directory owner is the unprivileged test account rather than deployment root.
use super::*;
use sandproof_controller_service::{JournalError, application::DataPermit};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::PathBuf,
};

struct Fixture {
    directory: tempfile::TempDir,
    gate: PathBuf,
    app: ControllerApplication<local::TestHost>,
    state: Arc<std::sync::Mutex<local::State>>,
    create: sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
    permit: DataPermit,
    relay: TrackedRelayChild,
    arm: Vec<u8>,
    owner: u32,
}

impl Fixture {
    fn running() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("db");
        let state = running_state();
        let app = lifecycle::application(&path, Arc::clone(&state), true);
        let create = lifecycle::request();
        app.record_lifecycle_intent(&create, &lifecycle::context())
            .unwrap();
        app.establish_lifecycle_host_epoch().unwrap();
        lifecycle::advance(&app).unwrap();
        let (permit, _) = app
            .lifecycle_data_authorization(
                &current_identity(&create),
                &lifecycle::context(),
                DataKind::FileWrite,
            )
            .unwrap();
        let relay = RelayReaper::new().unwrap().attach(
            Command::new("/bin/cat")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let pid = relay.process_id().unwrap();
        let text = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        let start = text
            .rsplit_once(')')
            .unwrap()
            .1
            .split_whitespace()
            .nth(19)
            .unwrap();
        let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap();
        let arm = format!(
            "sandproof-data-drain-v1\n{}\n{pid}\n{start}\n{}\n",
            "a".repeat(64),
            boot.trim()
        )
        .into_bytes();
        let gate = directory.path().join("gate");
        fs::create_dir(&gate).unwrap();
        fs::set_permissions(&gate, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(gate.join("selected.arm"), &arm).unwrap();
        fs::set_permissions(gate.join("selected.arm"), fs::Permissions::from_mode(0o600)).unwrap();
        let owner = gate.metadata().unwrap().uid();
        let connection = rusqlite::Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT version FROM controller_meta WHERE id=1",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            23
        );
        assert!(!connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='controller_internal_inbox')",
            [], |row| row.get::<_, bool>(0)).unwrap());
        Self {
            directory,
            gate,
            app,
            state,
            create,
            permit,
            relay,
            arm,
            owner,
        }
    }

    fn admitted(&self) -> sandproof_protocol::lifecycle::AcceptedLifecycleIntent {
        let intent = prepare_unload(&self.app, &self.state, &self.create, &lifecycle::context());
        assert_eq!(
            self.app
                .advance_recorded_lifecycle_unload(&intent, &intent.request.binding.node),
            Err(PortError::Busy)
        );
        assert!(*self.permit.revocation.borrow());
        assert!(capture_dispatches(&self.directory.path().join("db")).is_empty());
        intent
    }

    fn observe(
        &self,
        intent: &sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
    ) -> std::io::Result<()> {
        drain::record_unload_busy(intent, &self.app, &self.gate, self.owner)
    }

    fn no_receipt(&self) {
        assert!(!self.gate.join("unload-busy").exists());
        assert!(!self.gate.join(".unload-busy.stage").exists());
        assert!(capture_dispatches(&self.directory.path().join("db")).is_empty());
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.relay.terminate_and_wait().unwrap();
    }
}

#[test]
fn current_formal_application_busy_publishes_the_exact_original_request() {
    let fixture = Fixture::running();
    let intent = fixture.admitted();
    // Deterministic regression of the precise old API mismatch, not a missing
    // table stub: the original current-format application cannot use it.
    assert!(matches!(
        fixture.app.internal_audit(),
        Err(JournalError::UnsupportedStore)
    ));
    let result = fixture.observe(&intent);
    assert!(
        result.is_ok(),
        "current formal Application Busy must publish its exact request: {result:?}"
    );
    let bytes = fs::read(fixture.gate.join("unload-busy")).unwrap();
    assert!(bytes.starts_with(&fixture.arm));
    let actual: sandproof_protocol::lifecycle::AcceptedLifecycleIntent =
        serde_json::from_slice(&bytes[fixture.arm.len()..]).unwrap();
    assert_eq!(actual, intent);
    assert!(capture_dispatches(&fixture.directory.path().join("db")).is_empty());
}

#[test]
fn wrong_full_request_version_owner_or_source_never_publishes_busy() {
    let fixture = Fixture::running();
    let intent = fixture.admitted();
    for field in ["version", "owner", "source", "capture", "key"] {
        let mut wrong = intent.clone();
        match field {
            "version" => wrong.accepted_version += 1,
            "owner" => wrong.request.binding.key.owner = "another-owner".try_into().unwrap(),
            "source" => {
                wrong.request.operation = LifecycleOperation::Unload {
                    source_incarnation: 102.try_into().unwrap(),
                    capture: 1.try_into().unwrap(),
                }
            }
            "capture" => {
                wrong.request.operation = LifecycleOperation::Unload {
                    source_incarnation: 101.try_into().unwrap(),
                    capture: 2.try_into().unwrap(),
                }
            }
            _ => wrong.request.binding.key.sequence = 99.try_into().unwrap(),
        }
        assert!(fixture.observe(&wrong).is_err(), "mismatched {field}");
        fixture.no_receipt();
    }
}

#[test]
fn running_or_unadmitted_source_does_not_claim_a_pausing_full() {
    let fixture = Fixture::running();
    let _ = fixture.observe(&fixture.create);
    fixture.no_receipt();
    let mut intent = fixture.create.clone();
    intent.accepted_version = 2;
    intent.request.binding.expected = ExpectedSession::Version(1);
    intent.request.binding.key.sequence = 2.try_into().unwrap();
    intent.request.operation = LifecycleOperation::Unload {
        source_incarnation: 101.try_into().unwrap(),
        capture: 1.try_into().unwrap(),
    };
    fixture
        .app
        .record_lifecycle_intent(&intent, &lifecycle::context())
        .unwrap();
    let record = fixture
        .app
        .prepare_lifecycle_source_work(&intent, &lifecycle::context())
        .unwrap();
    assert!(record.full_capture().is_none());
    let _ = fixture.observe(&intent);
    fixture.no_receipt();
}

#[test]
fn wrong_selected_process_or_private_owner_never_publishes_busy() {
    let fixture = Fixture::running();
    let intent = fixture.admitted();
    assert!(
        drain::record_unload_busy(
            &intent,
            &fixture.app,
            &fixture.gate,
            fixture.owner.saturating_add(1)
        )
        .is_err()
    );
    fixture.no_receipt();
    for field in ["magic", "start", "boot"] {
        let text = String::from_utf8(fixture.arm.clone()).unwrap();
        let mut fields: Vec<_> = text.lines().map(str::to_owned).collect();
        match field {
            "magic" => fields[0] = "other-protocol".into(),
            "start" => fields[3] = "0".into(),
            _ => fields[4] = "foreign-boot".into(),
        }
        fs::write(fixture.gate.join("selected.arm"), fields.join("\n") + "\n").unwrap();
        assert!(fixture.observe(&intent).is_err(), "wrong gate {field}");
        fixture.no_receipt();
    }
}
