use super::*;
use axum::http::HeaderValue;

#[test]
fn missing_invalid_and_out_of_range_first_fields_use_the_fixed_default() {
    let default = Selection::Period(Duration::from_secs(90));
    assert_eq!(select(&HeaderMap::new()), default);
    assert_eq!(
        bounded_period(&HeaderMap::new()),
        Ok(Duration::from_secs(90))
    );
    for value in [
        b"".as_slice(),
        b"+",
        b"-",
        b" 1",
        b"1 ",
        b"\t1",
        b"1.0",
        b"1e3",
        b"1_000",
        b"0x10",
        b"9223372036854775808",
        b"-9223372036854775809",
        b"\xff",
        "９０".as_bytes(),
    ] {
        let mut headers = HeaderMap::new();
        headers.append(
            "keepalive-ping-interval",
            HeaderValue::from_bytes(value).unwrap(),
        );
        headers.append("keepalive-ping-interval", "1".parse().unwrap());
        assert_eq!(select(&headers), default, "{value:?}");
        assert_eq!(
            bounded_period(&headers),
            Ok(Duration::from_secs(90)),
            "{value:?}"
        );
    }
}

#[test]
fn approved_period_rejects_parsed_nonpositive_or_overflowing_seconds_even_if_go_wraps_positive() {
    for value in [
        "0",
        "+0",
        "-0",
        "-1",
        "-9223372036854775808",
        "9223372036854775807",
        "9223372037",
        "-9223372037",
        "18446744074",
        "-18446744073",
        "20211507185753197",
        "36028797018963968",
    ] {
        let mut headers = HeaderMap::new();
        headers.append("keepalive-ping-interval", value.parse().unwrap());
        headers.append("keepalive-ping-interval", "50".parse().unwrap());
        assert_eq!(bounded_period(&headers), Err(InvalidInterval), "{value}");
    }
}

#[test]
fn approved_period_preserves_positive_whole_seconds_and_ignores_later_fields() {
    for (value, expected) in [
        ("1", 1),
        ("+0001", 1),
        ("50", 50),
        ("90", 90),
        ("9223372036", 9_223_372_036),
    ] {
        let mut headers = HeaderMap::new();
        headers.append("keepalive-ping-interval", value.parse().unwrap());
        headers.append("keepalive-ping-interval", "0".parse().unwrap());
        let period = bounded_period(&headers).unwrap();
        assert_eq!(period, Duration::from_secs(expected));
        assert!(tokio::time::Instant::now().checked_add(period).is_some());
    }
    let mut headers = HeaderMap::new();
    headers.insert(
        "keepalive-ping-interval",
        format!("+{}1", "0".repeat(4096)).parse().unwrap(),
    );
    assert_eq!(bounded_period(&headers), Ok(Duration::from_secs(1)));
}

#[test]
fn signed_decimal_and_long_zero_prefixes_are_not_the_connect_timeout_parser() {
    for value in ["1", "+1", "01", "+0001"] {
        assert_eq!(
            select_first(value.as_bytes()),
            Selection::Period(Duration::from_secs(1))
        );
    }
    for sign in ["", "+"] {
        let value = format!("{sign}{}1", "0".repeat(4096));
        assert_eq!(
            select_first(value.as_bytes()),
            Selection::Period(Duration::from_secs(1))
        );
    }
    let mut headers = HeaderMap::new();
    headers.append("keepalive-ping-interval", "+001".parse().unwrap());
    for later in ["", "90", "-1", "invalid"] {
        headers.append("keepalive-ping-interval", later.parse().unwrap());
    }
    assert_eq!(select(&headers), Selection::Period(Duration::from_secs(1)));
}

#[test]
fn signed_duration_wrap_and_constructor_panics_remain_explicit_observations() {
    for (value, nanos) in [
        ("0", 0),
        ("+0", 0),
        ("-0", 0),
        ("-1", -1_000_000_000),
        ("9223372036854775807", -1_000_000_000),
        ("-9223372036854775808", 0),
        ("9223372037", -9_223_372_036_709_551_616),
        ("18446744073", -709_551_616),
        ("36028797018963968", 0),
    ] {
        assert_eq!(
            select_first(value.as_bytes()),
            Selection::UpstreamPanic { nanoseconds: nanos },
            "{value}"
        );
    }
    for (value, nanos) in [
        ("9223372036", 9_223_372_036_000_000_000),
        ("-9223372037", 9_223_372_036_709_551_616),
        ("18446744074", 290_448_384),
        ("-18446744073", 709_551_616),
        ("20211507185753197", 512),
    ] {
        assert_eq!(
            select_first(value.as_bytes()),
            Selection::Period(Duration::from_nanos(nanos)),
            "{value}"
        );
    }
}

fn vectors() -> Vec<Vec<u8>> {
    let mut values: std::collections::BTreeSet<Vec<u8>> = [
        "",
        "+",
        "-",
        "0",
        "+0",
        "-0",
        "1_000",
        "1.0",
        "1e3",
        "0x10",
        " 90",
        "90 ",
        "９０",
        "9223372036854775808",
        "-9223372036854775809",
        "20211507185753197",
    ]
    .into_iter()
    .map(|value| value.as_bytes().to_vec())
    .collect();
    for value in -512_i64..=512 {
        values.insert(value.to_string().into_bytes());
        values.insert(format!("{value:+}").into_bytes());
        values.insert(
            format!(
                "{}000{}",
                if value < 0 { "-" } else { "+" },
                value.unsigned_abs()
            )
            .into_bytes(),
        );
    }
    for pivot in [
        i64::MIN,
        i64::MAX,
        -9_223_372_036,
        9_223_372_036,
        -18_446_744_074,
        18_446_744_074,
        36_028_797_018_963_968,
    ] {
        for delta in -4..=4 {
            if let Some(value) = pivot.checked_add(delta) {
                values.insert(value.to_string().into_bytes());
            }
        }
    }
    for width in [19, 20, 64, 4096] {
        for sign in ["", "+", "-"] {
            for tail in [
                "",
                "0",
                "1",
                "90",
                "9223372036854775807",
                "9223372036854775808",
            ] {
                values.insert(format!("{sign}{}{tail}", "0".repeat(width)).into_bytes());
            }
        }
    }
    for byte in 0..=255 {
        values.insert(vec![byte]);
        for index in 0..=2 {
            let mut value = b"12".to_vec();
            value.insert(index, byte);
            values.insert(value);
        }
    }
    let mut seed = 0x311_u64;
    let mut next = || {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        seed
    };
    let alphabet = b"0123456789+- \t_exX\x00\xff";
    for _ in 0..2048 {
        let signed = i64::from_ne_bytes(next().to_ne_bytes());
        values.insert(signed.to_string().into_bytes());
        let length = usize::try_from(next() % 33).unwrap();
        values.insert(
            (0..length)
                .map(|_| {
                    alphabet
                        [usize::try_from(next() % u64::try_from(alphabet.len()).unwrap()).unwrap()]
                })
                .collect(),
        );
    }
    values.into_iter().collect()
}

fn line(selection: Selection) -> String {
    match selection {
        Selection::Period(duration) => format!("OK\t{}", duration.as_nanos()),
        Selection::UpstreamPanic { nanoseconds } => format!("PANIC\t{nanoseconds}"),
    }
}

#[test]
#[ignore = "explicit fixed Go keepalive selection; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
fn fixed_go_header_selection_and_actual_ticker_constructor_match() {
    use crate::file_download::tests::reference::{Reference, hex};
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    oracle.check("I\t-", &line(select(&HeaderMap::new())));
    let mut comparisons = 1;
    for first in vectors() {
        let expected = line(select_first(&first));
        let original = format!("I\t{}", hex(&first));
        oracle.check(&original, &expected);
        comparisons += 1;
        for later in [b"".as_slice(), b"1", b"-1", b"invalid"] {
            let input = format!("{original}\t{}", hex(later));
            oracle.check(&input, &expected);
            comparisons += 1;
            if let Ok(value) = HeaderValue::from_bytes(&first) {
                let mut headers = HeaderMap::new();
                headers.append("keepalive-ping-interval", value);
                headers.append(
                    "keepalive-ping-interval",
                    HeaderValue::from_bytes(later).unwrap(),
                );
                assert_eq!(line(select(&headers)), expected);
            }
        }
    }
    oracle.finish();
    eprintln!("fixed Go keepalive selection comparisons: {comparisons}");
}
