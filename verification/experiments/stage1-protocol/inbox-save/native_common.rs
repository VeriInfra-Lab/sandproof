//! Actual outer-validator checks shared by original, candidate, wrong, and restored Rust.
//! These value assertions do not claim durable acceptance or resource truth.
use super::*;
use crate::controller::lifecycle_inbox::model::{Inbox, LifecycleRetentionLane, Owner};

fn empty() -> Inbox {
    let config = config();
    let lanes = (1..=config.max_sessions)
        .map(LifecycleRetentionLane::fresh)
        .collect();
    Inbox {
        config,
        host_epoch_observation: None,
        owner_high_water: 0,
        owners: vec![],
        sessions: vec![],
        lanes,
    }
}

fn recorded() -> Inbox {
    let mut inbox = empty();
    let intent = create("owner", "session");
    assert_eq!(intent.validate(&context()), Ok(()));
    assert_eq!(inbox.record(&intent).unwrap(), (0, 0));
    inbox
}

fn attached() -> Inbox {
    let mut inbox = recorded();
    let local = InternalLocalBinding {
        sandbox: core::SandboxId(10),
        owner: core::TenantId(20),
        operation: core::OperationId(30),
    };
    inbox.owner_high_water = local.owner.0;
    inbox.owners.push(Owner {
        external: inbox.sessions[0].owner.clone(),
        local: local.owner,
    });
    inbox.sessions[0].local = Some(local);
    inbox.sessions[0].entries[0].local_operation = Some(local.operation);
    inbox.host_epoch_observation = Some(crate::capture::HostEpochObservation {
        node: inbox.config.node.clone(),
        registration: inbox.config.host_epoch.clone().unwrap(),
    });
    inbox
}

fn check(inbox: &Inbox, accepted: bool) {
    let before = inbox.clone();
    match (inbox.validate(), accepted) {
        (Ok(()), true) | (Err(JournalError::CorruptStore), false) => {}
        (result, _) => panic!("unexpected full Inbox validation: {result:?}; accepted={accepted}"),
    }
    assert_eq!(*inbox, before);
}

#[test]
fn empty_recorded_and_attached_inboxes_are_valid_values() {
    check(&empty(), true);
    check(&recorded(), true);
    check(&attached(), true);
}

#[test]
fn owner_table_checks_both_unique_fields_and_high_water() {
    let mut inbox = empty();
    inbox.owner_high_water = 21;
    inbox.owners.push(Owner {
        external: "owner".try_into().unwrap(),
        local: core::TenantId(20),
    });
    check(&inbox, true);
    inbox.owners.push(Owner {
        external: "owner".try_into().unwrap(),
        local: core::TenantId(21),
    });
    check(&inbox, false);
    inbox.owners[1].external = "other".try_into().unwrap();
    check(&inbox, true);
    inbox.owners[1].local = core::TenantId(20);
    check(&inbox, false);
    inbox.owners.pop();
    inbox.owner_high_water = 19;
    check(&inbox, false);
    inbox.owners[0].local = core::TenantId(0);
    inbox.owner_high_water = 0;
    check(&inbox, true);
}

#[test]
fn local_membership_requires_one_matching_external_internal_pair() {
    let inbox = attached();
    let mut wrong = inbox.clone();
    wrong.owner_high_water = 21;
    wrong.owners[0].local = core::TenantId(21);
    check(&wrong, false);
    wrong = inbox;
    wrong.owners[0].external = "foreign".try_into().unwrap();
    check(&wrong, false);
    wrong.owners.clear();
    check(&wrong, false);
}

#[test]
fn observation_binds_full_node_and_full_registration() {
    let inbox = attached();
    let mut wrong = inbox.clone();
    wrong
        .host_epoch_observation
        .as_mut()
        .unwrap()
        .registration
        .policy = sandproof_protocol::lifecycle::ContentDigest::from_sha256([8; 32]);
    check(&wrong, false);
    wrong = inbox.clone();
    wrong
        .host_epoch_observation
        .as_mut()
        .unwrap()
        .node
        .node_generation = 2.try_into().unwrap();
    check(&wrong, false);
    wrong = inbox;
    wrong.config.host_epoch = None;
    check(&wrong, false);
}

#[test]
fn current_terminal_and_incarnation_match_the_actual_entry_fold() {
    let inbox = recorded();
    let mut wrong = inbox.clone();
    wrong.sessions[0].current_version = 2;
    check(&wrong, false);
    wrong = inbox.clone();
    wrong.sessions[0].terminal = true;
    check(&wrong, false);
    wrong = inbox;
    wrong.sessions[0].incarnation = None;
    check(&wrong, false);
}

#[test]
fn lane_count_and_slot_order_are_outer_invariants() {
    let inbox = empty();
    let mut wrong = inbox.clone();
    wrong.lanes.swap(0, 1);
    check(&wrong, false);
    wrong = inbox.clone();
    wrong.lanes[1].slot = 1;
    check(&wrong, false);
    wrong = inbox;
    wrong.lanes.pop();
    check(&wrong, false);
}

fn destroy(version: u64, expected: u64, sequence: u64) -> AcceptedLifecycleIntent {
    let mut intent = create("owner", "session");
    intent.accepted_version = version;
    intent.request.binding.expected = ExpectedSession::Version(expected);
    intent.request.binding.key.sequence = sequence.try_into().unwrap();
    intent.request.operation = LifecycleOperation::Destroy {};
    assert_eq!(intent.validate(&context()), Ok(()));
    intent
}

#[test]
fn destroy_versions_are_strict_even_when_predecessor_adjacency_is_exempt() {
    let mut inbox = recorded();
    let mut entry = inbox.sessions[0].entries[0].clone();
    entry.intent = destroy(1, 0, 2);
    inbox.sessions[0].entries.push(entry);
    inbox.sessions[0].terminal = true;
    check(&inbox, false);

    let last = inbox.sessions[0].entries.last_mut().unwrap();
    last.intent = destroy(2, 1, 2);
    inbox.sessions[0].current_version = 2;
    check(&inbox, true);
}

#[test]
fn duplicate_destroy_cannot_hide_behind_a_nonterminal_flag() {
    let mut inbox = recorded();
    assert_eq!(inbox.record(&destroy(2, 1, 2)).unwrap(), (0, 1));
    check(&inbox, true);
    let mut second = inbox.sessions[0].entries[1].clone();
    second.intent = destroy(3, 2, 3);
    inbox.sessions[0].entries.push(second);
    inbox.sessions[0].current_version = 3;
    inbox.sessions[0].terminal = false;
    check(&inbox, false);
}

// Draft appended only to an isolated copy of the existing Inbox value fixtures.
// Not executed yet; no persistence, resource, or formal proof claim.
#[cfg(test)]
mod save_guard_obligations {
    use super::*;
    use crate::controller::lifecycle_inbox::{codec, unload_resolution::UnloadResolution};
    use sandproof_protocol::lifecycle::CurrentReadyFacts;

    fn unload(inbox: &Inbox, version: u64, sequence: u64, capture: u64) -> AcceptedLifecycleIntent {
        let mut intent = create("owner", "session");
        intent.accepted_version = version;
        intent.request.binding.expected = ExpectedSession::Version(version - 1);
        intent.request.binding.key.sequence = sequence.try_into().unwrap();
        intent.request.operation = LifecycleOperation::Unload {
            source_incarnation: inbox.sessions[0].incarnation.unwrap(),
            capture: capture.try_into().unwrap(),
        };
        assert_eq!(intent.validate(&context()), Ok(()));
        intent
    }

    fn resolved_unload() -> Inbox {
        let mut inbox = attached();
        check(&inbox, true);
        let first = unload(&inbox, 2, 2, 7);
        assert_eq!(inbox.record(&first).unwrap(), (0, 1));
        check(&inbox, true);
        inbox.sessions[0].entries[1].local_operation = Some(core::OperationId(31));
        inbox.sessions[0].entries[1].unload_resolution = Some(UnloadResolution::AfterCapture {
            facts: CurrentReadyFacts {
                started_at_ms: 1,
                observed_at_ms: 2,
                end_at_ms: 3,
                max_end_at_ms: 4,
                startup_deadline_ms: 2,
            },
        });
        check(&inbox, true);
        inbox
    }

    #[test]
    fn complete_value_preconditions_do_not_alone_make_record_preserve_validation() {
        let mut inbox = resolved_unload();
        let repeated_capture = unload(&inbox, 3, 3, 7);
        assert_eq!(inbox.record(&repeated_capture).unwrap(), (0, 2));
        check(&inbox, false);
    }

    #[test]
    fn actual_save_rejects_the_invalid_poststate_before_replacing_the_row() {
        let mut inbox = resolved_unload();
        let connection = rusqlite::Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE controller_lifecycle_inbox(id INTEGER PRIMARY KEY, state_json BLOB, sha256 BLOB);\
                 INSERT INTO controller_lifecycle_inbox VALUES(1,X'00',X'00');",
            )
            .unwrap();
        codec::save(&connection, &inbox).unwrap();
        let row = || -> (Vec<u8>, Vec<u8>) {
            connection
                .query_row(
                    "SELECT state_json,sha256 FROM controller_lifecycle_inbox WHERE id=1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap()
        };
        let before = row();
        let repeated_capture = unload(&inbox, 3, 3, 7);
        assert_eq!(inbox.record(&repeated_capture).unwrap(), (0, 2));
        assert!(matches!(
            codec::save(&connection, &inbox),
            Err(JournalError::CorruptStore)
        ));
        assert_eq!(row(), before);

        let mut fresh = resolved_unload();
        let next_capture = unload(&fresh, 3, 3, 8);
        assert_eq!(fresh.record(&next_capture).unwrap(), (0, 2));
        check(&fresh, true);
        codec::save(&connection, &fresh).unwrap();
        assert_ne!(row(), before);
    }
}

#[test]
fn actual_sqlite_save_requires_the_one_target_row() {
    use crate::controller::lifecycle_inbox::codec;
    let inbox = empty();
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    connection.execute_batch(
        "CREATE TABLE controller_lifecycle_inbox(id INTEGER PRIMARY KEY, state_json BLOB, sha256 BLOB);\
         INSERT INTO controller_lifecycle_inbox VALUES(2,X'31',X'32');",
    ).unwrap();
    assert!(matches!(
        codec::save(&connection, &inbox),
        Err(JournalError::CorruptStore)
    ));
    let row: (Vec<u8>, Vec<u8>) = connection
        .query_row(
            "SELECT state_json,sha256 FROM controller_lifecycle_inbox WHERE id=2",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(row, (vec![0x31], vec![0x32]));
}
