use super::*;
#[path = "../../../sandbox-protocol/tests/support/lifecycle.rs"]
mod wire;

#[test]
fn creation_audit_exports_exact_binding_and_hashes_without_user_contents() {
    let options = CreateOptions {
        env_vars: [("SECRET".into(), "private-env-content".into())].into(),
        metadata: [
            ("task".into(), "private-task-content".into()),
            ("arbitrary".into(), "private-user-content".into()),
        ]
        .into(),
        ..CreateOptions::default()
    };
    let mut request = wire::request(17, 3, Intent::Create);
    request.create.options = serde_json::to_vec(&options).unwrap();
    let encoded = serde_json::to_string(&create_view(&request).unwrap()).unwrap();
    for secret in [
        "private-env-content",
        "private-task-content",
        "private-user-content",
    ] {
        assert!(!encoded.contains(secret));
    }
    let value: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    assert_eq!(
        value["identity"],
        serde_json::to_value(&request.identity).unwrap()
    );
    assert_eq!(
        value["resources"],
        serde_json::to_value(request.create.resources).unwrap()
    );
    assert_eq!(
        value["startup_deadline_ms"],
        request.create.startup_deadline_ms
    );
    assert_eq!(value["timeout_ms"], request.create.timeout_ms);
    assert_eq!(value["max_lifetime_ms"], request.create.max_lifetime_ms);
    assert_eq!(
        value["options_sha256"],
        format!("{:x}", Sha256::digest(&request.create.options))
    );
    assert_eq!(
        value["metadata_sha256"],
        format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&options.metadata).unwrap())
        )
    );
    assert!(value.get("metadata").is_none());
    assert!(value.get("options").is_none());
    assert!(encoded.len() < 4096);
}

#[test]
fn metadata_diagnostic_hash_is_canonical_but_not_an_authority() {
    let mut a = wire::request(1, 1, Intent::Create);
    let mut b = a.clone();
    a.create.options = br#"{"env_vars":{},"metadata":{"b":"2","a":"1"}}"#.to_vec();
    b.create.options = br#"{"metadata":{"a":"1","b":"2"},"env_vars":{}}"#.to_vec();
    let first = create_view(&a).unwrap();
    let second = create_view(&b).unwrap();
    assert_eq!(first.metadata_sha256, second.metadata_sha256);
    assert_ne!(first.options_sha256, second.options_sha256);
    // The audit is a borrowed projection, not a state update or acceptance.
    assert_eq!(first.identity, &a.identity);
    assert_eq!(second.identity, &b.identity);
}

#[test]
fn invalid_options_fail_with_fixed_diagnostics_instead_of_exporting_bytes() {
    let mut request = wire::request(1, 1, Intent::Create);
    request.create.options = b"private-invalid-input".to_vec();
    assert!(matches!(
        create_view(&request),
        Err("invalid original center options")
    ));
    request.create.options = br#"{"metadata":{"a":"\u0000"},"env_vars":{}}"#.to_vec();
    assert!(matches!(
        create_view(&request),
        Err("invalid original center options")
    ));
}
