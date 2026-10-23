use super::*;

#[test]
fn current_claim_requires_boot_identity_and_refuses_older_or_null_bytes() {
    for bytes in [
        br#"{"resource_id":"original","pid":71,"start_time_ticks":93}"#.as_slice(),
        br#"{"resource_id":"original","pid":71,"start_time_ticks":93,"boot_id":null}"#.as_slice(),
    ] {
        assert!(serde_json::from_slice::<ProcessRecord>(bytes).is_err());
    }
}

#[test]
fn new_claim_retains_exact_original_boot_and_rejects_corruption() {
    let source = RuntimeProcessIdentity::capture(std::process::id()).unwrap();
    let record = ProcessRecord {
        resource_id: "original".into(),
        pid: source.pid,
        start_time_ticks: source.start_time_ticks,
        boot_id: source.boot_id.clone(),
        launch_cpu_domain: vec![0],
    };
    let saved = serde_json::to_vec(&record).unwrap();
    let restored: ProcessRecord = serde_json::from_slice(&saved).unwrap();
    assert_eq!(restored.source_identity().unwrap(), source);
    for boot_id in ["", "boot", "FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF"] {
        let invalid = ProcessRecord {
            boot_id: boot_id.into(),
            ..record.clone()
        };
        assert!(invalid.source_identity().is_err());
        assert!(verified_process(&invalid).is_err());
    }
}

#[test]
fn valid_foreign_boot_never_selects_the_current_process_with_the_same_pid() {
    let source = RuntimeProcessIdentity::capture(std::process::id()).unwrap();
    let alternate = if source.boot_id == "ffffffff-ffff-ffff-ffff-ffffffffffff" {
        "00000000-0000-0000-0000-000000000000"
    } else {
        "ffffffff-ffff-ffff-ffff-ffffffffffff"
    };
    let record = ProcessRecord {
        resource_id: "original".into(),
        pid: source.pid,
        start_time_ticks: source.start_time_ticks,
        boot_id: alternate.into(),
        launch_cpu_domain: vec![0],
    };
    assert!(verified_process(&record).unwrap().is_none());
    assert!(source.is_alive().unwrap());
}

#[test]
fn current_claim_requires_actual_launch_cpu_domain_and_rejects_bad_sets() {
    let source=RuntimeProcessIdentity::capture(std::process::id()).unwrap();
    let record=ProcessRecord {resource_id:"original".into(),pid:source.pid,
        start_time_ticks:source.start_time_ticks,boot_id:source.boot_id,
        launch_cpu_domain:vec![0,2]};
    let mut missing=serde_json::to_value(&record).unwrap();
    missing.as_object_mut().unwrap().remove("launch_cpu_domain");
    assert!(serde_json::from_value::<ProcessRecord>(missing).is_err());
    for domain in [vec![],vec![0,0],vec![2,1],vec![8192],(0..257).collect()] {
        let invalid=ProcessRecord{launch_cpu_domain:domain,..record.clone()};
        assert!(invalid.source_identity().is_err());
    }
    let bytes=serde_json::to_vec(&record).unwrap();
    assert_eq!(serde_json::from_slice::<ProcessRecord>(&bytes).unwrap(),record);
}
