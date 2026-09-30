#![cfg(unix)]
use sandproof_controller_service::build_requests::{
    BuildConfig, BuildError, BuildKind, BuildRequest, BuildState, BuildStore,
};
use std::{
    os::unix::fs::{MetadataExt, PermissionsExt},
    process::Command,
};
use tempfile::TempDir;

fn setup() -> (TempDir, BuildConfig, u32) {
    let dir = TempDir::new().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let config = BuildConfig {
        root: dir.path().canonicalize().unwrap(),
        max_jobs_global: 3,
        max_jobs_per_tenant: 2,
        max_input_bytes_global: 12,
        max_input_bytes_per_tenant: 8,
        max_log_bytes: 4,
    };
    let uid = std::fs::metadata(dir.path()).unwrap().uid();
    (dir, config, uid)
}
fn request(id: &str) -> BuildRequest {
    BuildRequest {
        build_id: id.repeat(32),
        kind: BuildKind::Dockerfile,
        disk_mib: 512,
        min_free_mib: 16,
        cpu_count: 1,
        memory_mib: 256,
        host_memory_mib: 512,
    }
}

#[test]
fn durable_replay_is_owner_scoped_and_all_history_counts_toward_quota() {
    let (_dir, config, uid) = setup();
    let store = BuildStore::open(config.clone(), uid).unwrap();
    let one = request("1");
    let accepted = store.submit("1", &one, b"1234").unwrap();
    assert_eq!(accepted.state, BuildState::Queued);
    assert!(matches!(
        store.get("2", &one.build_id),
        Err(BuildError::Missing)
    ));
    assert!(matches!(
        store.submit("1", &one, b"other"),
        Err(BuildError::Conflict)
    ));
    let mut changed = one.clone();
    changed.cpu_count = 2;
    assert!(matches!(
        store.submit("1", &changed, b"1234"),
        Err(BuildError::Conflict)
    ));
    store.submit("1", &request("2"), b"1234").unwrap();
    assert!(matches!(
        store.submit("1", &request("3"), b"x"),
        Err(BuildError::Quota)
    ));
    store.submit("2", &one, b"1234").unwrap();
    assert!(matches!(
        store.submit("3", &request("3"), b"x"),
        Err(BuildError::Quota)
    ));
    assert_eq!(store.submit("1", &one, b"1234").unwrap(), accepted);
    drop(store);
    assert_eq!(
        BuildStore::open(config, uid)
            .unwrap()
            .get("1", &one.build_id)
            .unwrap(),
        accepted
    );
}

#[test]
fn python_supervisor_claim_and_receipts_share_the_actual_rust_schema() {
    let (_dir, config, uid) = setup();
    let store = BuildStore::open(config.clone(), uid).unwrap();
    let one = request("1");
    store.submit("1", &one, b"1234").unwrap();
    let script = r"
import sys
from tools.template_supervisor import Queue
q=Queue(sys.argv[1],4)
j=q.claim()
assert j['source']==b'1234' and j['owner']=='1'
try: Queue(sys.argv[1],4).claim()
except RuntimeError: pass
else: raise AssertionError('uncertain build was replayed')
q.resource(j,'original-vm')
try: q.resource(j,'replacement-vm')
except RuntimeError: pass
else: raise AssertionError('original VM identity was replaced')
q.log(j,b'\x00\xffabc')
try: q.finish(dict(j,claim='stale'),candidate='a'*32)
except RuntimeError: pass
else: raise AssertionError('stale receipt accepted')
q.finish(j,candidate='a'*32)
try: q.finish(j,failure='build_failed')
except RuntimeError: pass
else: raise AssertionError('terminal state changed')
assert q.claim() is None
";
    let output = Command::new("python3")
        .args(["-c", script])
        .arg(&config.root)
        .current_dir(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap(),
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let view = store.get("1", &one.build_id).unwrap();
    assert_eq!(view.state, BuildState::Candidate);
    assert_eq!(view.candidate_id, Some("a".repeat(32)));
    let log = store.logs("1", &one.build_id, 1, 2).unwrap();
    assert_eq!(log.bytes, [255, b'a']);
    assert_eq!(log.next_offset, 3);
    assert!(log.truncated);
    assert!(matches!(
        store.logs("2", &one.build_id, 0, 4),
        Err(BuildError::Missing)
    ));
    assert!(matches!(
        store.logs("1", &one.build_id, 5, 4),
        Err(BuildError::Invalid)
    ));
    assert_eq!(store.submit("1", &one, b"1234").unwrap(), view);
}

#[test]
fn unsafe_roots_foreign_databases_and_invalid_resources_fail_closed() {
    let (dir, config, uid) = setup();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(BuildStore::open(config.clone(), uid).is_err());
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let db = rusqlite::Connection::open(dir.path().join("builds.db")).unwrap();
    db.execute("CREATE TABLE unrelated(id INTEGER)", [])
        .unwrap();
    drop(db);
    assert!(BuildStore::open(config, uid).is_err());
    let mut input = request("1");
    input.min_free_mib = input.disk_mib;
    assert!(!input.is_valid());
    input.min_free_mib = 16;
    input.host_memory_mib = input.memory_mib;
    assert!(!input.is_valid());
}
