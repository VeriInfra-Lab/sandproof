//! Actual fixed-engine rewrite/restart durability, not a disk-quota claim.
use super::*;
use std::collections::BTreeMap;

fn persistence(connection: &mut redis::Connection) -> BTreeMap<String, String> {
    let raw: String = redis::cmd("INFO")
        .arg("persistence")
        .query(connection)
        .unwrap();
    raw.lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
}

#[test]
fn repeated_reaffirm_durably_advances_revision_without_rewriting_unchanged_payload() {
    const LIMIT: usize = 512 * 1024;
    const CONFIRMATIONS: u64 = 8;
    let mut fixture = Engine::new();
    let (mut ledger, initial) = fixture.initialize(LIMIT);
    let payload = State {
        frontier: 2,
        pending: "x".repeat(256 * 1024),
    };
    let mut observed = ledger.commit(&initial, &payload).unwrap().into_observed();
    let mut connection = connect(fixture.address);
    let before_fields: BTreeMap<String, String> = redis::cmd("HGETALL")
        .arg(KEY)
        .query(&mut connection)
        .unwrap();
    let before_size = persistence(&mut connection)["aof_current_size"]
        .parse::<u64>()
        .unwrap();
    for _ in 0..CONFIRMATIONS {
        let next_revision = observed.revision().get() + 1;
        observed = ledger.reaffirm(&observed).unwrap().into_observed();
        assert_eq!(observed.state(), &payload);
        assert_eq!(observed.revision().get(), next_revision);
        assert_eq!(fixture.revision(), next_revision.to_string());
    }
    let after_size = persistence(&mut connection)["aof_current_size"]
        .parse::<u64>()
        .unwrap();
    let growth = after_size.checked_sub(before_size).unwrap();
    eprintln!(
        "reaffirm AOF: confirmations={CONFIRMATIONS} payload_bytes={} growth_bytes={growth}",
        payload.pending.len()
    );
    // A fresh actual durable write is required; a cached/read-only success is
    // not a confirmation. Its log cost must not scale with unchanged payload.
    assert!(
        growth > 0 && growth < 8 * 1024,
        "unchanged-payload AOF growth {growth}"
    );
    let after_fields: BTreeMap<String, String> = redis::cmd("HGETALL")
        .arg(KEY)
        .query(&mut connection)
        .unwrap();
    assert_eq!(after_fields.len(), 4);
    for (field, value) in before_fields {
        if field == "revision" {
            assert_eq!(after_fields[&field], observed.revision().get().to_string());
        } else {
            assert!(after_fields[&field] == value, "original {field} changed");
        }
    }
    let revision = observed.revision();
    drop(connection);
    drop(ledger);
    // Actual SIGKILL/restart, retaining the original AOF and private writer
    // identity. Ordinary open observes the durable revision without writing.
    fixture.stop();
    fixture.start();
    let (_, reopened) = RedisLedger::<State>::open(
        endpoint(fixture.address),
        Engine::config(LIMIT),
        fixture.lease(),
    )
    .unwrap();
    assert_eq!(reopened.state(), &payload);
    assert_eq!(reopened.revision(), revision);
    assert_eq!(fixture.revision(), revision.get().to_string());
}

#[test]
fn actual_aof_rewrite_then_engine_restart_retains_exact_cas_revision_and_payload() {
    const LIMIT: usize = 64 * 1024;
    let mut fixture = Engine::new();
    let (mut ledger, mut observed) = fixture.initialize(LIMIT);
    let mut connection = connect(fixture.address);
    let old_rewrites = persistence(&mut connection)["aof_rewrites"]
        .parse::<u64>()
        .unwrap();
    for frontier in 2..=33 {
        let next = State {
            frontier,
            pending: "a".repeat(32 * 1024),
        };
        observed = ledger.commit(&observed, &next).unwrap().into_observed();
    }
    let before_bytes = persistence(&mut connection)["aof_current_size"]
        .parse::<u64>()
        .unwrap();
    assert!(before_bytes > 32 * 1024);
    let _: String = redis::cmd("BGREWRITEAOF").query(&mut connection).unwrap();
    // Real CAS writes continue after the rewrite is requested. A fixed bounded
    // poll observes completion; time alone or an initial "ok" cannot pass.
    for frontier in 34..=41 {
        let next = State {
            frontier,
            pending: "b".repeat(32 * 1024),
        };
        observed = ledger.commit(&observed, &next).unwrap().into_observed();
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let info = persistence(&mut connection);
        if info["aof_rewrites"].parse::<u64>().unwrap() > old_rewrites
            && info["aof_rewrite_in_progress"] == "0"
        {
            assert_eq!(info["aof_last_bgrewrite_status"], "ok");
            assert_eq!(info["aof_last_write_status"], "ok");
            assert!(info["aof_current_size"].parse::<u64>().unwrap() > 0);
            break;
        }
        assert!(Instant::now() < deadline, "actual AOF rewrite incomplete");
        thread::sleep(Duration::from_millis(10));
    }
    let revision = observed.revision();
    let final_state = State {
        frontier: 41,
        pending: "b".repeat(32 * 1024),
    };
    assert_eq!(observed.state(), &final_state);
    drop(connection);
    drop(ledger);
    fixture.stop();
    fixture.start();
    let (mut recovered, original) = RedisLedger::<State>::open(
        endpoint(fixture.address),
        Engine::config(LIMIT),
        fixture.lease(),
    )
    .unwrap();
    assert_eq!(original.state(), &final_state);
    assert_eq!(original.revision(), revision);
    // Open returns Observed, not Committed, and must not silently rewrite the
    // original revision. The authority's explicit reaffirm owns approval.
    assert_eq!(fixture.revision(), revision.get().to_string());
    let committed = recovered.reaffirm(&original).unwrap();
    assert_eq!(committed.state(), &final_state);
    assert_eq!(committed.revision().get(), revision.get() + 1);
    assert!(!recovered.writes_fenced());
}
