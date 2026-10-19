use super::*;
use axum::http::HeaderValue;

#[test]
fn execution_and_response_contexts_keep_their_distinct_zero_and_negative_rules() {
    let now = Instant::now();
    let original = now + Duration::from_secs(30);
    for (value, execution, response) in [
        ("", None, original),
        ("0", None, now),
        ("+0", None, now),
        ("-0", None, now),
        ("-1", None, now),
        ("-999999999", None, now),
        (
            "+1",
            Some(Duration::from_millis(1)),
            now + Duration::from_millis(1),
        ),
        (
            "9999999999",
            Some(Duration::from_millis(9_999_999_999)),
            original,
        ),
    ] {
        let mut headers = HeaderMap::new();
        headers.append("connect-timeout-ms", value.parse().unwrap());
        // Go Header.Get ignores subsequent fields even when they are invalid.
        headers.append("connect-timeout-ms", "invalid".parse().unwrap());
        assert_eq!(execution_duration(&headers), Ok(execution), "{value:?}");
        assert_eq!(deadline(&headers, now, original), Ok(response), "{value:?}");
    }
    assert_eq!(execution_duration(&HeaderMap::new()), Ok(None));
    assert_eq!(deadline(&HeaderMap::new(), now, original), Ok(original));
}

#[test]
fn both_contexts_reject_malformed_or_oversized_first_values() {
    let now = Instant::now();
    for value in [
        b"+".as_slice(),
        b"-",
        b" 1",
        b"1 ",
        b"1.0",
        b"1e3",
        b"10000000000",
        b"+9999999999",
        b"-9999999999",
        b"\xff",
    ] {
        let mut headers = HeaderMap::new();
        headers.append(
            "connect-timeout-ms",
            HeaderValue::from_bytes(value).unwrap(),
        );
        headers.append("connect-timeout-ms", "1".parse().unwrap());
        let reason = if value.len() > 10 {
            TimeoutReason::TooLong
        } else {
            TimeoutReason::Invalid
        };
        assert_eq!(execution_duration(&headers), Err(reason));
        let error = deadline(&headers, now, now + Duration::from_secs(1)).unwrap_err();
        assert_eq!(error.reason(), reason);
        assert_eq!(error.value(), value);
    }
}
