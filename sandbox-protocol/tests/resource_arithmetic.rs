use sandproof_protocol::internal::lifecycle::Resources;

#[test]
fn explicit_resource_guards_keep_original_component_order_and_boundaries() {
    let mut cases = Vec::new();
    for instances in [0, 1, u64::MAX - 1, u64::MAX] {
        for cpu in [0, 1, u64::MAX - 1, u64::MAX] {
            for memory_bytes in [0, 1, u64::MAX - 1, u64::MAX] {
                for disk_bytes in [0, 1, u64::MAX - 1, u64::MAX] {
                    cases.push(Resources {
                        instances,
                        cpu,
                        memory_bytes,
                        disk_bytes,
                    });
                }
            }
        }
    }
    for value in &cases {
        assert_eq!(value.positive(), value.values().iter().all(|n| *n > 0));
        for limit in &cases {
            assert_eq!(
                value.fits(*limit),
                value
                    .values()
                    .iter()
                    .zip(limit.values())
                    .all(|(a, b)| *a <= b)
            );
        }
    }
}
