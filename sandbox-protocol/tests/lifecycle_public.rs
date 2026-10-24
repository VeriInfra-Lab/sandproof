use sandproof_protocol::{
    internal::lifecycle::Resources,
    lifecycle::{CreateParameters, PublicCreateRequest},
};

#[test]
fn relative_parameters_keep_exact_fields_and_arbitrary_bytes_before_business_validation() {
    let original = CreateParameters {
        template_revision: "template".try_into().unwrap(),
        template_digest: [255; 32],
        resources: Resources {
            instances: 0,
            cpu: u64::MAX,
            memory_bytes: 0,
            disk_bytes: u64::MAX,
        },
        options: vec![0, 255],
        startup_timeout_ms: 0,
        timeout_ms: u64::MAX,
        max_lifetime_ms: 1,
    };
    let value = serde_json::to_value(&original).unwrap();
    assert_eq!(
        value,
        serde_json::json!({"template_revision":"template","template_digest":vec![255u8;32],
        "resources":{"instances":0,"cpu":u64::MAX,"memory_bytes":0,"disk_bytes":u64::MAX},"options":[0,255],
        "startup_timeout_ms":0,"timeout_ms":u64::MAX,"max_lifetime_ms":1})
    );
    assert_eq!(
        serde_json::from_slice::<CreateParameters>(&serde_json::to_vec(&original).unwrap())
            .unwrap(),
        original
    );
    let request = PublicCreateRequest {
        window: 1.try_into().unwrap(),
        sequence: 1.try_into().unwrap(),
        parameters: original,
    };
    for field in ["owner", "deployment", "node", "session"] {
        let mut value = serde_json::to_value(&request).unwrap();
        value[field] = serde_json::json!("other");
        assert!(serde_json::from_value::<PublicCreateRequest>(value).is_err());
    }
}
