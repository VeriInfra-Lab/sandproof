//! Immutable assembly comparison is distinct from preserved dynamic history.
use super::*;

#[test]
fn protected_registration_changes_cannot_rewrite_recovered_business_history() {
    let configured = envelope().metadata;
    let encoded = serde_json::to_value(&configured).unwrap();
    let patches = [
        ("catalog", "aliases", serde_json::json!({"alias":"base"})),
        ("registry", "config", {
            let mut config = encoded["registry"]["config"].clone();
            config["root"] = serde_json::json!("/other/registry");
            config
        }),
        (
            "budget",
            "max_certificate_bytes",
            serde_json::json!(256 * 1024),
        ),
    ];
    assert!(configured.registration_matches(&configured));
    for (object, field, value) in patches {
        let mut changed = encoded.clone();
        changed[object][field] = value;
        let changed = serde_json::from_value::<RegistryMetadata>(changed).unwrap();
        assert!(!configured.registration_matches(&changed));
    }
    let mut changed = encoded.clone();
    changed["registrations"][0]["revision"] = serde_json::json!("base-r2");
    let changed = serde_json::from_value::<RegistryMetadata>(changed).unwrap();
    assert!(!configured.registration_matches(&changed));
}

#[test]
fn actual_recovered_records_do_not_require_empty_provisioning_history() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let business = authority(&fixture, &proxy);
    business.begin_publication(candidate(), revision()).unwrap();
    let restored = business.snapshot().unwrap();
    assert_eq!(restored.metadata.registry().records().len(), 1);
    assert!(restored.metadata.registration_matches(&envelope().metadata));
}
