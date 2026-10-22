use super::*;

#[test]
fn legacy_claim_is_not_upgraded_and_preserves_its_serialized_shape() {
    let bytes = br#"{"resource_id":"original","pid":71,"start_time_ticks":93}"#;
    let legacy: ProcessRecord = serde_json::from_slice(bytes).unwrap();
    assert!(legacy.source_identity().unwrap().is_none());
    assert_eq!(serde_json::to_vec(&legacy).unwrap(), bytes);
}

#[test]
fn new_claim_retains_exact_original_boot_and_rejects_corruption() {
    let source = RuntimeProcessIdentity::capture(std::process::id()).unwrap();
    let record = ProcessRecord {
        resource_id: "original".into(),
        pid: source.pid,
        start_time_ticks: source.start_time_ticks,
        boot_id: Some(source.boot_id.clone()),
    };
    let saved = serde_json::to_vec(&record).unwrap();
    let restored: ProcessRecord = serde_json::from_slice(&saved).unwrap();
    assert_eq!(restored.source_identity().unwrap(), Some(source));
    for boot_id in ["", "boot", "FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF"] {
        let invalid = ProcessRecord {
            boot_id: Some(boot_id.into()),
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
        boot_id: Some(alternate.into()),
    };
    assert!(verified_process(&record).unwrap().is_none());
    assert!(source.is_alive().unwrap());
}
