use sandproof_protocol::numeric::{
    available_resource, bytes_to_mebibytes, seconds_to_milliseconds,
};

#[test]
fn seconds_keep_zero_exact_multiplication_and_original_overflow_boundary() {
    let last = u64::MAX / 1000;
    for seconds in [0, 1, 999, 1000, last - 1, last, last + 1, u64::MAX] {
        assert_eq!(seconds_to_milliseconds(seconds), seconds.checked_mul(1000));
        let wide = u128::from(seconds) * 1000;
        assert_eq!(seconds_to_milliseconds(seconds), u64::try_from(wide).ok());
    }
    assert_eq!(seconds_to_milliseconds(0), Some(0));
    assert_eq!(seconds_to_milliseconds(last + 1), None);
}

#[test]
fn disk_units_keep_original_rounding_and_u32_rejection() {
    const MIB: u64 = 1024 * 1024;
    let last = u64::from(u32::MAX) * MIB;
    for bytes in [
        0,
        1,
        MIB - 1,
        MIB,
        MIB + 1,
        last - MIB,
        last - 1,
        last,
        last + 1,
        u64::MAX - 1,
        u64::MAX,
    ] {
        assert_eq!(
            bytes_to_mebibytes(bytes),
            u32::try_from(bytes.div_ceil(MIB)).ok()
        );
        let wide = u128::from(bytes).div_ceil(u128::from(MIB));
        assert_eq!(bytes_to_mebibytes(bytes), u32::try_from(wide).ok());
    }
    assert_eq!(bytes_to_mebibytes(0), Some(0));
    assert_eq!(bytes_to_mebibytes(last), Some(u32::MAX));
    assert_eq!(bytes_to_mebibytes(last + 1), None);
}

#[test]
fn free_components_keep_original_saturation_and_observation_cap() {
    let cases = [0, 1, 2, 17, 1024, u64::MAX - 1, u64::MAX];
    for limit in cases {
        for used in cases {
            for observed in cases {
                let free = available_resource(limit, used, observed);
                assert_eq!(free, limit.saturating_sub(used).min(observed));
                assert!(free <= limit);
                assert!(free <= observed);
                if used >= limit {
                    assert_eq!(free, 0);
                } else {
                    assert!(free.checked_add(used).unwrap() <= limit);
                }
            }
        }
    }
}
