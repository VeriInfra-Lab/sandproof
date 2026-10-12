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
