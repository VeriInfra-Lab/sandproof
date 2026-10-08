//! Durable allocator boundaries in the sole complete journal format.
use super::*;
use crate::NeverCrash;
use rusqlite::Connection;
use tempfile::TempDir;
const NS: &str = "0123456789abcdef";
fn setup() -> (TempDir, ControllerJournal) {
    let dir = TempDir::new().unwrap();
    let journal = tests_support::open(dir.path().join("db"), NS).unwrap();
    (dir, journal)
}
fn request(id: u64, operation: u64) -> core::ControllerInput {
    core::ControllerInput::Request(core::Request::Create(core::CreateRequest {
        actor: core::Actor::Tenant(core::TenantId(1)),
        owner: core::TenantId(1),
        sandbox: core::SandboxId(id),
        workspace: core::FsId(id),
        file_policy: core::FilePolicyRef {
            digest: core::PolicyDigest(1),
        },
        policy: core::NetworkPolicy::DenyAll,
        operation: core::OperationId(operation),
    }))
}
fn waters(journal: &ControllerJournal) -> Vec<(String, String, String)> {
    journal
        .journal
        .connection
        .prepare("SELECT kind,scope,high_water FROM controller_id_highwaters ORDER BY kind,scope")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}
#[test]
fn current_nonempty_restart_preserves_world_frontiers_and_feature_gates() {
    let (dir, mut journal) = setup();
    tests_support::admit(&mut journal, request(2, 3), &mut NeverCrash).unwrap();
    tests_support::admit(&mut journal, request(9, 23), &mut NeverCrash).unwrap();
    let world = journal.world().unwrap();
    let before = waters(&journal);
    drop(journal);
    let reopened = tests_support::open(dir.path().join("db"), NS).unwrap();
    assert_eq!(reopened.world().unwrap(), world);
    assert_eq!(waters(&reopened), before);
    assert_eq!(reopened.next_sandbox_id().unwrap(), core::SandboxId(10));
    assert_eq!(
        reopened.next_operation_id(core::SandboxId(9)).unwrap(),
        core::OperationId(24)
    );
    assert!(!reopened.has_template_registry().unwrap());
}
#[test]
fn generic_journal_is_not_a_controller_import_or_writer() {
    let dir = TempDir::new().unwrap();
    let legacy_path = dir.path().join("generic");
    drop(Journal::open(&legacy_path).unwrap());
    let before = std::fs::read(&legacy_path).unwrap();
    assert!(tests_support::open(&legacy_path, NS).is_err());
    assert_eq!(std::fs::read(&legacy_path).unwrap(), before);
    let path = dir.path().join("controller");
    let mut stale = Journal {
        connection: Connection::open(&path).unwrap(),
    };
    let journal = tests_support::open(&path, NS).unwrap();
    assert!(matches!(
        Journal::open(&path),
        Err(JournalError::UnsupportedStore)
    ));
    assert!(matches!(
        stale.claim_next::<core::EffectId>(&mut NeverCrash),
        Err(JournalError::UnsupportedStore)
    ));
    assert!(journal.world().unwrap().sandbox_records().is_empty());
}
#[test]
fn missing_metadata_or_frontiers_is_never_reinitialized() {
    for sql in [
        "DROP TABLE controller_meta",
        "DROP TABLE controller_id_highwaters",
        "DELETE FROM controller_id_highwaters WHERE kind='effect'",
        "UPDATE controller_id_highwaters SET high_water='01' WHERE kind='sandbox'",
        "UPDATE controller_id_highwaters SET high_water='2' WHERE kind='input'",
    ] {
        let (dir, journal) = setup();
        journal.journal.connection.execute_batch(sql).unwrap();
        drop(journal);
        let path = dir.path().join("db");
        assert!(tests_support::open(&path, NS).is_err(), "{sql}");
        assert_eq!(
            Connection::open(path)
                .unwrap()
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            crate::journal::MAINTAINED_FORMAT_VERSION
        );
    }
}
struct FailBefore;
impl CrashInjector for FailBefore {
    fn hit(&mut self, point: CrashPoint) -> Result<(), JournalError> {
        if point == CrashPoint::BeforeCommit {
            Err(JournalError::InjectedCrash(point))
        } else {
            Ok(())
        }
    }
}
#[test]
fn rejected_and_rolled_back_admission_never_consumes_frontiers() {
    let (_dir, mut journal) = setup();
    let initial = waters(&journal);
    assert!(matches!(
        tests_support::admit(&mut journal, request(40, 20), &mut FailBefore),
        Err(JournalError::InjectedCrash(_))
    ));
    assert_eq!(waters(&journal), initial);
    assert_eq!(journal.next_sandbox_id().unwrap(), core::SandboxId(1));
    tests_support::admit(&mut journal, request(1, 1), &mut NeverCrash).unwrap();
    let committed = waters(&journal);
    assert!(tests_support::admit(&mut journal, request(1, 90), &mut NeverCrash).is_err());
    assert_eq!(waters(&journal), committed);
}
#[test]
fn uncertain_commit_recovers_actual_allocator_outcome_before_retry() {
    for durable in [false, true] {
        let (dir, mut journal) = setup();
        transaction::fail_next_commit(durable);
        assert!(matches!(
            tests_support::admit(&mut journal, request(7, 11), &mut NeverCrash),
            Err(JournalError::CommitUncertain(_))
        ));
        let next = if durable { 8 } else { 1 };
        assert_eq!(journal.next_sandbox_id().unwrap(), core::SandboxId(next));
        assert_eq!(
            journal.world().unwrap().sandbox_records().len(),
            usize::from(durable)
        );
        tests_support::admit(&mut journal, request(next, 1), &mut NeverCrash).unwrap();
        let before = waters(&journal);
        drop(journal);
        assert_eq!(
            waters(&tests_support::open(dir.path().join("db"), NS).unwrap()),
            before
        );
    }
}
#[test]
fn finite_u64_frontiers_refuse_without_wrapping_or_mutating() {
    let (_dir, journal) = setup();
    journal
        .journal
        .connection
        .execute(
            "UPDATE controller_id_highwaters SET high_water=?1 WHERE kind='sandbox'",
            [u64::MAX.to_string()],
        )
        .unwrap();
    let before = waters(&journal);
    assert!(matches!(
        journal.next_sandbox_id(),
        Err(JournalError::IdentifierExhausted)
    ));
    assert_eq!(waters(&journal), before);
}
#[test]
fn each_sql_family_has_exact_finite_boundary_and_batch_rollback() {
    for family in [
        allocators::SqlId::Input,
        allocators::SqlId::Effect,
        allocators::SqlId::Outbox,
        allocators::SqlId::Receipt,
    ] {
        let (_dir, mut journal) = setup();
        let tx = journal.journal.connection.transaction().unwrap();
        let kind = match family {
            allocators::SqlId::Input => "input",
            allocators::SqlId::Effect => "effect",
            allocators::SqlId::Outbox => "outbox",
            allocators::SqlId::Receipt => "receipt",
        };
        let limit = if kind == "input" {
            i64::MAX - 1
        } else {
            i64::MAX
        };
        tx.execute(
            "UPDATE controller_id_highwaters SET high_water=?1 WHERE kind=?2",
            params![(limit - 1).to_string(), kind],
        )
        .unwrap();
        assert!(matches!(
            allocators::reserve_sql(&tx, family, 2),
            Err(JournalError::IdentifierExhausted)
        ));
        assert_eq!(
            allocators::read(&tx, kind, "").unwrap(),
            u64::try_from(limit - 1).unwrap()
        );
        assert_eq!(allocators::next_sql(&tx, family).unwrap(), limit);
        assert!(matches!(
            allocators::next_sql(&tx, family),
            Err(JournalError::IdentifierExhausted)
        ));
        tx.rollback().unwrap();
    }
}
#[test]
fn deleted_live_projections_or_input_suffix_are_corruption_not_sparse_gc() {
    for sql in [
        "DELETE FROM controller_inputs WHERE sequence=(SELECT MAX(sequence) FROM controller_inputs)",
        "DELETE FROM outbox",
        "DELETE FROM controller_effect_keys",
    ] {
        let (dir, mut journal) = setup();
        tests_support::admit(&mut journal, request(1, 1), &mut NeverCrash).unwrap();
        journal.journal.connection.execute_batch(sql).unwrap();
        drop(journal);
        assert!(
            tests_support::open(dir.path().join("db"), NS).is_err(),
            "{sql}"
        );
    }
}
#[test]
fn foreign_counter_commit_fails_closed_and_second_controller_cannot_allocate() {
    let (dir, journal) = setup();
    let path = dir.path().join("db");
    assert!(matches!(
        tests_support::open(&path, NS),
        Err(JournalError::ControllerBusy)
    ));
    Connection::open(path)
        .unwrap()
        .execute(
            "UPDATE controller_id_highwaters SET high_water='20' WHERE kind='sandbox'",
            [],
        )
        .unwrap();
    assert!(matches!(
        journal.next_sandbox_id(),
        Err(JournalError::CorruptStore)
    ));
}
