use super::*;

#[test]
fn resize_observation_keeps_native_invalid_argument_distinct_from_mode_rejection() {
    for errno in [1, 4, 5, 9, 22, 25] {
        let error = std::io::Error::from_raw_os_error(errno);
        let observed = ResizeFailure::observed(&error);
        assert!(matches!(observed, ResizeFailure::Native { errno: n, .. } if n == errno));
        assert_eq!(observed.error(), input_error(&error));
    }
    for error in [
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a PTY"),
        std::io::Error::new(std::io::ErrorKind::BrokenPipe, "terminal hung up"),
        std::io::Error::from_raw_os_error(0),
    ] {
        assert!(matches!(
            ResizeFailure::observed(&error),
            ResizeFailure::Command(_)
        ));
        assert_eq!(ResizeFailure::observed(&error).error(), input_error(&error));
    }
    for error in [
        CommandError::StaleIdentity,
        CommandError::NotStarted,
        CommandError::ClosedInput,
    ] {
        assert_eq!(ResizeFailure::from(error).error(), error);
    }
}

#[test]
fn signal_errno_requires_original_native_observation_and_keeps_legacy_error() {
    for errno in [1, 3, 9, 22, 38] {
        let observed = SignalFailure::observed(&std::io::Error::from_raw_os_error(errno));
        assert!(matches!(observed, SignalFailure::Native(number) if number == errno));
        assert_eq!(observed.error(), CommandError::Unavailable);
    }
    for error in [
        std::io::Error::new(std::io::ErrorKind::NotFound, "no syscall"),
        std::io::Error::from_raw_os_error(0),
    ] {
        assert!(matches!(
            SignalFailure::observed(&error),
            SignalFailure::Command(CommandError::Unavailable)
        ));
    }
    assert_eq!(
        SignalFailure::from(CommandError::StaleIdentity).error(),
        CommandError::StaleIdentity
    );
}

#[test]
fn input_native_observation_requires_real_errno_and_preserves_legacy_error() {
    for errno in [5, 9, 11, 22, 28, 32] {
        let original = std::io::Error::from_raw_os_error(errno);
        let observed = input_failure(&original, false);
        assert!(
            matches!(observed, InputFailure::Native { errno: number, error } if number == errno && error == input_error(&original))
        );
        assert_eq!(observed.error(), input_error(&original));
    }
    for kind in [
        std::io::ErrorKind::BrokenPipe,
        std::io::ErrorKind::InvalidInput,
        std::io::ErrorKind::Other,
    ] {
        let error = std::io::Error::new(kind, "synthetic error with no syscall observation");
        assert!(matches!(
            input_failure(&error, false),
            InputFailure::Command(_)
        ));
    }
    assert!(matches!(
        input_failure(
            &std::io::Error::new(std::io::ErrorKind::BrokenPipe, "closed stdin"),
            true
        ),
        InputFailure::StdinClosed
    ));
}

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
