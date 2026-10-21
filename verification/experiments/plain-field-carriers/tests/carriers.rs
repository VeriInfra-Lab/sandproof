use sandproof_plain_field_carriers_extract::{
    sandbox_new, sandbox_value, tenant_new, tenant_value, versions_current,
};

#[test]
fn actual_tenant_methods_keep_zero_and_maximum() {
    for value in [0, 1, u64::MAX] {
        let original = tenant_new(value);
        assert_eq!(tenant_value(original), value);
    }
}

#[test]
fn actual_sandbox_methods_keep_zero_and_maximum() {
    for value in [0, 1, u64::MAX] {
        let original = sandbox_new(value);
        assert_eq!(sandbox_value(original), value);
    }
}

#[test]
fn current_versions_keep_original_types_and_constants() {
    let original = versions_current();
    let expected: (u16, u32, i64) = (2, 6, 16);
    assert_eq!(
        (
            original.internal_protocol,
            original.center_store,
            original.host_store,
        ),
        expected,
    );
}
