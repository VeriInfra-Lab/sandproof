use super::*;

#[test]
fn timeout_headers_use_the_fixed_first_value_and_one_absolute_execution_origin() {
    let start = Instant::now();
    let mut headers = HeaderMap::new();
    assert!(
        ExecutionDeadline::from_headers(&headers, start)
            .unwrap()
            .0
            .is_none()
    );
    for (wire, ms) in [
        ("", 0),
        ("0", 0),
        ("-1", 0),
        ("+0", 0),
        ("-000000001", 0),
        ("0000000000", 0),
        ("1", 1),
        ("+1", 1),
        ("9999999999", 9_999_999_999),
    ] {
        headers.insert("connect-timeout-ms", wire.parse().unwrap());
        let deadline = ExecutionDeadline::from_headers(&headers, start).unwrap();
        assert_eq!(
            deadline.0,
            (ms != 0).then(|| start + Duration::from_millis(ms))
        );
    }
    for wire in [
        "+",
        "-",
        " 1",
        "1 ",
        "1.0",
        "10000000000",
        "forever",
        "+9999999999",
    ] {
        headers.insert("connect-timeout-ms", wire.parse().unwrap());
        assert!(matches!(
            ExecutionDeadline::from_headers(&headers, start),
            Err(CommandError::InvalidArgument)
        ));
    }
    headers.insert("connect-timeout-ms", "1".parse().unwrap());
    headers.append("connect-timeout-ms", "invalid".parse().unwrap());
    assert_eq!(
        ExecutionDeadline::from_headers(&headers, start).unwrap().0,
        Some(start + Duration::from_millis(1))
    );
    assert!(ExecutionDeadline::from_duration(start, Duration::MAX).is_err());
}

#[test]
fn record_and_output_limits_are_explicit_and_shared_by_pool_clones() {
    let limits = OwnerLimits {
        records: 1,
        subscribers: 2,
        queued_chunks: 2,
        output_bytes: 1024,
        stdin_bytes: 1024,
    };
    for bad in [
        OwnerLimits {
            records: 0,
            ..limits
        },
        OwnerLimits {
            subscribers: 0,
            ..limits
        },
        OwnerLimits {
            queued_chunks: 0,
            ..limits
        },
        OwnerLimits {
            output_bytes: usize::MAX,
            ..limits
        },
        OwnerLimits {
            stdin_bytes: 0,
            ..limits
        },
        OwnerLimits {
            records: usize::MAX,
            ..limits
        },
    ] {
        assert!(matches!(
            CommandOwners::new(bad),
            Err(CommandError::InvalidArgument)
        ));
    }
    let owners = CommandOwners::new(limits).unwrap();
    let reservation = owners.reserve().unwrap();
    assert!(matches!(
        owners.clone().reserve(),
        Err(CommandError::ResourceExhausted)
    ));
    drop(reservation); // no process was created; no cleanup receipt is implied.
    assert!(owners.reserve().is_ok());
}
