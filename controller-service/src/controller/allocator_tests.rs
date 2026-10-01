//! Real `SQLite` allocator migration/transaction regressions. Deliberate SQL
//! deletions below are isolated fixtures, not a supported production GC path.
use super::{
    allocators::{self, SqlId},
    *,
};
use crate::NeverCrash;
use rusqlite::Connection;
use tempfile::TempDir;

const NS: &str = "0123456789abcdef";

fn setup() -> (TempDir, ControllerJournal) {
    let dir = TempDir::new().unwrap();
    let journal = ControllerJournal::open(dir.path().join("controller.db"), NS).unwrap();
    (dir, journal)
}

fn create(sandbox: u64, operation: u64) -> core::ControllerInput {
    core::ControllerInput::Request(core::Request::Create(core::CreateRequest {
        actor: core::Actor::Tenant(core::TenantId(1)),
        owner: core::TenantId(1),
        sandbox: core::SandboxId(sandbox),
        workspace: core::FsId(sandbox),
        file_policy: core::FilePolicyRef {
            digest: core::PolicyDigest(1),
        },
        policy: core::NetworkPolicy::DenyAll,
        operation: core::OperationId(operation),
    }))
}

fn water(j: &ControllerJournal, kind: &str, scope: &str) -> String {
    j.journal
        .connection
        .query_row(
            "SELECT high_water FROM controller_id_highwaters WHERE kind=?1 AND scope=?2",
            params![kind, scope],
            |r| r.get(0),
        )
        .unwrap()
}

fn waters(j: &ControllerJournal) -> Vec<(String, String, String)> {
    j.journal
        .connection
        .prepare("SELECT kind,scope,high_water FROM controller_id_highwaters ORDER BY kind,scope")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn scalar(db: &Connection, sql: &str) -> i64 {
    db.query_row(sql, [], |r| r.get(0)).unwrap()
}

fn make_legacy(j: ControllerJournal) {
    j.journal
        .connection
        .execute_batch("DROP TABLE controller_id_highwaters; PRAGMA user_version=1;")
        .unwrap();
    drop(j);
}

#[test]
fn allocator_migration_preserves_world_ids_and_feature_gates() {
    let (dir, mut j) = setup();
    j.apply(create(9, 23), &mut NeverCrash).unwrap();
    j.apply(create(2, 3), &mut NeverCrash).unwrap(); // Trusted explicit IDs remain legal.
    let world = j.world().unwrap();
    let keys: Vec<(i64, Vec<u8>)> = j
        .journal
        .connection
        .prepare("SELECT id,logical_json FROM controller_effect_keys ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    make_legacy(j);
    let mut j = ControllerJournal::open(dir.path().join("controller.db"), NS).unwrap();
    assert_eq!(scalar(&j.journal.connection, "PRAGMA user_version"), 2);
    assert_eq!(
        scalar(&j.journal.connection, "SELECT version FROM controller_meta"),
        7
    );
    assert!(!j.has_template_registry().unwrap());
    assert_eq!(j.world().unwrap(), world);
    assert_eq!(j.next_sandbox_id().unwrap(), core::SandboxId(10));
    assert_eq!(
        j.next_operation_id(core::SandboxId(9)).unwrap(),
        core::OperationId(24)
    );
    assert_eq!(
        j.next_operation_id(core::SandboxId(2)).unwrap(),
        core::OperationId(4)
    );
    let migrated: Vec<(i64, Vec<u8>)> = j
        .journal
        .connection
        .prepare("SELECT id,logical_json FROM controller_effect_keys ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(migrated, keys);
    assert_eq!(water(&j, "input", ""), "2");
    j.apply(create(10, 1), &mut NeverCrash).unwrap();
    let before = waters(&j);
    drop(j);
    let j = ControllerJournal::open(dir.path().join("controller.db"), NS).unwrap();
    assert_eq!(waters(&j), before);
    assert_eq!(j.next_sandbox_id().unwrap(), core::SandboxId(11));
}

#[test]
fn generic_journal_keeps_format_one_and_preopened_handles_cannot_write_format_two() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("controller.db");
    let mut generic = Journal::open(&path).unwrap();
    assert_eq!(scalar(&generic.connection, "PRAGMA user_version"), 1);
    let controller = ControllerJournal::open(&path, NS).unwrap();
    assert_eq!(
        scalar(&controller.journal.connection, "PRAGMA user_version"),
        2
    );
    assert!(matches!(
        Journal::open(&path),
        Err(JournalError::UnsupportedStore)
    ));
    assert!(matches!(
        generic.claim_next::<core::EffectId>(&mut NeverCrash),
        Err(JournalError::UnsupportedStore)
    ));
    assert_eq!(
        scalar(
            &controller.journal.connection,
            "SELECT count(*) FROM outbox"
        ),
        0
    );
}

#[test]
fn existing_format_two_missing_metadata_or_frontiers_is_never_reinitialized() {
    for sql in [
        "DROP TABLE controller_meta",
        "DROP TABLE controller_id_highwaters",
        "DELETE FROM controller_id_highwaters WHERE kind='effect'",
        "UPDATE controller_id_highwaters SET high_water='01' WHERE kind='sandbox'",
        "UPDATE controller_id_highwaters SET high_water='1' WHERE kind='input'",
    ] {
        let (dir, j) = setup();
        j.journal.connection.execute_batch(sql).unwrap();
        drop(j);
        let path = dir.path().join("controller.db");
        assert!(ControllerJournal::open(&path, NS).is_err(), "{sql}");
        assert_eq!(
            scalar(&Connection::open(&path).unwrap(), "PRAGMA user_version"),
            2
        );
    }
}

#[test]
fn migration_failure_rolls_back_new_frontiers_and_format_gate() {
    let (dir, mut j) = setup();
    j.apply(create(1, 1), &mut NeverCrash).unwrap();
    make_legacy(j);
    let path = dir.path().join("controller.db");
    Connection::open(&path)
        .unwrap()
        .execute("UPDATE outbox SET id=-1", [])
        .unwrap();
    assert!(matches!(
        ControllerJournal::open(&path, NS),
        Err(JournalError::CorruptStore)
    ));
    let db = Connection::open(&path).unwrap();
    assert_eq!(scalar(&db, "PRAGMA user_version"), 1);
    assert_eq!(
        scalar(
            &db,
            "SELECT count(*) FROM sqlite_master WHERE name='controller_id_highwaters'"
        ),
        0
    );
}

#[test]
fn sqlite_identity_frontiers_survive_sparse_deleted_maximum_rows() {
    let (dir, mut j) = setup();
    j.apply(create(1, 1), &mut NeverCrash).unwrap();
    j.apply(create(2, 1), &mut NeverCrash).unwrap();
    // Core history is deliberately retained. This narrowly checks SQL ID
    // allocation independently of old mapping/outbox rows; not lifecycle GC.
    j.journal
        .connection
        .execute_batch(
            "DELETE FROM outbox WHERE id=2; DELETE FROM controller_effect_keys WHERE id=2;",
        )
        .unwrap();
    drop(j);
    let mut j = ControllerJournal::open(dir.path().join("controller.db"), NS).unwrap();
    j.apply(create(3, 1), &mut NeverCrash).unwrap();
    assert_eq!(
        scalar(&j.journal.connection, "SELECT MAX(id) FROM outbox"),
        3
    );
    assert_eq!(
        scalar(
            &j.journal.connection,
            "SELECT MAX(id) FROM controller_effect_keys"
        ),
        3
    );
    assert_eq!(water(&j, "outbox", ""), "3");
    assert_eq!(water(&j, "effect", ""), "3");
    // Frontier metadata, rather than any retained maximum, is authoritative.
    j.journal.connection.execute("UPDATE controller_id_highwaters SET high_water='50' WHERE kind IN ('sandbox','operation')", []).unwrap();
    drop(j);
    let j = ControllerJournal::open(dir.path().join("controller.db"), NS).unwrap();
    assert_eq!(j.next_sandbox_id().unwrap(), core::SandboxId(51));
    assert_eq!(
        j.next_operation_id(core::SandboxId(1)).unwrap(),
        core::OperationId(51)
    );
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Evidence {
    resource: sandproof_protocol::ResourceRef,
    command: core::Command,
}
impl sandproof_protocol::ResourceEvidence for Evidence {
    fn resource(&self) -> &sandproof_protocol::ResourceRef {
        &self.resource
    }
}
impl HostEvidence for Evidence {
    fn validates(&self, command: &core::Command) -> bool {
        self.command == *command
    }
}

fn fence_receipt(action: &WorkAction<core::EffectId>) -> WorkReceipt<Evidence> {
    WorkReceipt {
        identity: action.identity.clone(),
        outcome: WorkOutcome::ObservedSuccess(Evidence {
            resource: action.identity.resource.clone(),
            command: action.command.command().clone(),
        }),
    }
}

#[test]
fn receipt_frontier_survives_deleted_maximum_reopen_and_duplicate_completion() {
    let (dir, mut j) = setup();
    for id in [1, 2] {
        j.apply(create(id, 1), &mut NeverCrash).unwrap();
        j.dispatch_next(&mut |action| Ok(fence_receipt(action)), &mut NeverCrash)
            .unwrap();
    }
    assert_eq!(
        scalar(
            &j.journal.connection,
            "SELECT MAX(sequence) FROM controller_receipt_history"
        ),
        2
    );
    j.journal
        .connection
        .execute(
            "DELETE FROM controller_receipt_history WHERE sequence=2",
            [],
        )
        .unwrap();
    drop(j);
    let mut j = ControllerJournal::open(dir.path().join("controller.db"), NS).unwrap();
    j.apply(create(3, 1), &mut NeverCrash).unwrap();
    let DispatchResult::ReceiptRecorded(receipt) = j
        .dispatch_next(&mut |action| Ok(fence_receipt(action)), &mut NeverCrash)
        .unwrap()
    else {
        panic!("receipt expected")
    };
    assert_eq!(
        scalar(
            &j.journal.connection,
            "SELECT MAX(sequence) FROM controller_receipt_history"
        ),
        3
    );
    let before = waters(&j);
    let world = j.world().unwrap();
    assert_eq!(j.complete(&receipt, &mut NeverCrash).unwrap(), world);
    assert_eq!(waters(&j), before);
}

#[test]
fn input_sequence_reserves_replay_sentinel_and_refuses_exact_boundary() {
    let (_dir, mut j) = setup();
    j.journal
        .connection
        .execute(
            "UPDATE controller_id_highwaters SET high_water=?1 WHERE kind='input'",
            [(i64::MAX - 2).to_string()],
        )
        .unwrap();
    let tx = transaction::WorldTransaction::begin(&mut j.journal.connection, &j.committed_world)
        .unwrap();
    let last = allocators::next_sql(&tx, SqlId::Input).unwrap();
    assert_eq!(last, i64::MAX - 1);
    assert_eq!(last.checked_add(1), Some(i64::MAX)); // unchanged load_world's sentinel
    assert!(matches!(
        allocators::next_sql(&tx, SqlId::Input),
        Err(JournalError::IdentifierExhausted)
    ));
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
fn rejected_and_rolled_back_acceptance_never_consumes_frontiers() {
    let (_dir, mut j) = setup();
    let initial = waters(&j);
    assert!(matches!(
        j.apply(create(40, 20), &mut FailBefore),
        Err(JournalError::InjectedCrash(_))
    ));
    assert_eq!(waters(&j), initial);
    assert_eq!(j.next_sandbox_id().unwrap(), core::SandboxId(1));
    assert!(j.world().unwrap().sandbox_records().is_empty());
    j.apply(create(1, 1), &mut NeverCrash).unwrap();
    let committed = waters(&j);
    assert!(j.apply(create(1, 90), &mut NeverCrash).is_err());
    assert_eq!(waters(&j), committed);
    assert_eq!(
        scalar(
            &j.journal.connection,
            "SELECT MAX(sequence) FROM controller_inputs"
        ),
        1
    );
}

#[test]
fn uncertain_commit_reads_durable_frontiers_before_retry_on_same_handle() {
    for durable in [false, true] {
        let (dir, mut j) = setup();
        transaction::fail_next_commit(durable);
        assert!(matches!(
            j.apply(create(7, 11), &mut NeverCrash),
            Err(JournalError::CommitUncertain(_))
        ));
        let next = if durable { 8 } else { 1 };
        assert_eq!(j.next_sandbox_id().unwrap(), core::SandboxId(next));
        assert_eq!(
            j.world().unwrap().sandbox_records().len(),
            usize::from(durable)
        );
        j.apply(create(next, 1), &mut NeverCrash).unwrap();
        let count = if durable { 2 } else { 1 };
        assert_eq!(
            scalar(
                &j.journal.connection,
                "SELECT COUNT(*) FROM controller_inputs"
            ),
            count
        );
        assert_eq!(
            scalar(
                &j.journal.connection,
                "SELECT MAX(sequence) FROM controller_inputs"
            ),
            count
        );
        let before = waters(&j);
        drop(j);
        let j = ControllerJournal::open(dir.path().join("controller.db"), NS).unwrap();
        assert_eq!(waters(&j), before);
    }
}

#[test]
fn foreign_counter_commit_fails_closed_and_second_controller_cannot_allocate() {
    let (dir, j) = setup();
    let path = dir.path().join("controller.db");
    assert!(matches!(
        ControllerJournal::open(&path, NS),
        Err(JournalError::ControllerBusy)
    ));
    Connection::open(&path)
        .unwrap()
        .execute(
            "UPDATE controller_id_highwaters SET high_water='5' WHERE kind='sandbox'",
            [],
        )
        .unwrap();
    assert!(matches!(
        j.next_sandbox_id(),
        Err(JournalError::CorruptStore)
    ));
    assert!(j.world().is_err());
}

#[test]
fn finite_u64_frontiers_refuse_without_wrapping_or_mutating() {
    let (_dir, mut j) = setup();
    j.apply(create(u64::MAX, u64::MAX), &mut NeverCrash)
        .unwrap();
    let before = waters(&j);
    assert!(matches!(
        j.next_sandbox_id(),
        Err(JournalError::IdentifierExhausted)
    ));
    assert!(matches!(
        j.next_operation_id(core::SandboxId(u64::MAX)),
        Err(JournalError::IdentifierExhausted)
    ));
    assert_eq!(waters(&j), before);
}

#[test]
fn each_sql_family_checks_capacity_before_reservation_and_acceptance_rolls_back() {
    for family in ["input", "effect", "outbox", "receipt"] {
        let (_dir, mut j) = setup();
        j.journal
            .connection
            .execute(
                "UPDATE controller_id_highwaters SET high_water=?1 WHERE kind=?2",
                params![i64::MAX.to_string(), family],
            )
            .unwrap();
        let before = waters(&j);
        let sql_family = match family {
            "input" => SqlId::Input,
            "effect" => SqlId::Effect,
            "outbox" => SqlId::Outbox,
            _ => SqlId::Receipt,
        };
        {
            let tx =
                transaction::WorldTransaction::begin(&mut j.journal.connection, &j.committed_world)
                    .unwrap();
            assert!(matches!(
                allocators::next_sql(&tx, sql_family),
                Err(JournalError::IdentifierExhausted)
            ));
        }
        assert_eq!(waters(&j), before);
        {
            assert!(matches!(
                j.apply(create(1, 1), &mut NeverCrash),
                Err(JournalError::IdentifierExhausted)
            ));
            assert_eq!(waters(&j), before);
            assert_eq!(
                scalar(
                    &j.journal.connection,
                    "SELECT COUNT(*) FROM controller_inputs"
                ),
                0
            );
            assert!(j.world().unwrap().sandbox_records().is_empty());
        }
    }
}

#[test]
fn exhausted_batch_does_not_partially_advance_and_deleted_input_tail_is_corruption() {
    let (dir, mut j) = setup();
    j.journal
        .connection
        .execute(
            "UPDATE controller_id_highwaters SET high_water=?1 WHERE kind='outbox'",
            [(i64::MAX - 1).to_string()],
        )
        .unwrap();
    {
        let tx =
            transaction::WorldTransaction::begin(&mut j.journal.connection, &j.committed_world)
                .unwrap();
        assert!(matches!(
            allocators::reserve_sql(&tx, SqlId::Outbox, 2),
            Err(JournalError::IdentifierExhausted)
        ));
        assert_eq!(allocators::next_sql(&tx, SqlId::Outbox).unwrap(), i64::MAX);
    }
    assert_eq!(water(&j, "outbox", ""), (i64::MAX - 1).to_string());
    j.apply(create(1, 1), &mut NeverCrash).unwrap();
    j.journal
        .connection
        .execute("DELETE FROM controller_inputs", [])
        .unwrap();
    drop(j);
    assert!(matches!(
        ControllerJournal::open(dir.path().join("controller.db"), NS),
        Err(JournalError::CorruptStore)
    ));
}
