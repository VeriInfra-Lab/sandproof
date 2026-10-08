use super::super::test_support::{check, full_capacity_settings, resolve, settings};
use super::*;
use serde_json::json;

#[test]
fn production_journal_wiring_rejects_policy_disable_and_change() {
    struct Scratch(PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "sandproof-maintenance-config-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir(&directory).unwrap();
    let scratch = Scratch(directory);
    let database = scratch.0.join("controller.db");
    let config: ConfigFile = serde_json::from_value(full_capacity_settings()).unwrap();
    let mut contract = DeploymentContract::validate(&config).unwrap();
    let policy = contract.maintenance.clone();
    drop(open_journal(database.clone(), &config.namespace, &contract).unwrap());
    let mut disabled = full_capacity_settings();
    disabled.as_object_mut().unwrap().remove("maintenance");
    assert!(check(disabled).is_err());
    let mut changed = policy.clone();
    changed.retention_ms += 1;
    assert!(
        ControllerJournal::preflight_complete(
            &database,
            &config.namespace,
            &contract.bounded,
            &changed
        )
        .is_err()
    );
    contract.maintenance = changed;
    assert!(open_journal(database.clone(), &config.namespace, &contract,).is_err());
    contract.maintenance = policy.clone();
    let reopened = open_journal(database, &config.namespace, &contract).unwrap();
    assert_eq!(reopened.maintenance_config().unwrap(), Some(policy.clone()));
}

#[test]
fn persisted_route_limits_refuse_changed_config_without_changing_the_original() {
    struct Scratch(PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "sandproof-route-config-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir(&path).unwrap();
    let scratch = Scratch(path);
    let database = scratch.0.join("controller.db");
    let namespace = "0123456789abcdef";
    let original = resolve(settings());
    let maintenance = serde_json::from_value::<ConfigFile>(settings())
        .unwrap()
        .maintenance
        .unwrap();
    drop(
        ControllerJournal::open_complete(
            &database,
            namespace,
            original.clone(),
            maintenance.clone(),
            1,
        )
        .unwrap(),
    );
    for (field, value) in [
        ("max_transfers_per_vm", 5),
        ("max_transfers", 16),
        ("control_reserve", 2),
    ] {
        let mut changed = settings();
        changed["data_transfer"][field] = json!(value);
        let changed = resolve(changed);
        assert!(
            ControllerJournal::preflight_complete(&database, namespace, &changed, &maintenance)
                .is_err()
        );
        assert!(
            ControllerJournal::open_complete(&database, namespace, changed, maintenance.clone(), 2)
                .is_err()
        );
        let reopened = ControllerJournal::open_complete(
            &database,
            namespace,
            original.clone(),
            maintenance.clone(),
            2,
        )
        .unwrap();
        assert_eq!(
            reopened.bounded_host_config().unwrap(),
            Some(original.clone())
        );
    }
}
