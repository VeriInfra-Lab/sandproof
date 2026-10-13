use super::*;
use date::{Timestamp, Utc};
fn limits() -> Limits {
    Limits {
        header_bytes: 65536,
        items: 1024,
    }
}

#[test]
fn ranges_keep_suffix_zero_empty_files_overlaps_and_parse_order() {
    let partial = |start, length| RangePlan::Partial(vec![ByteRange { start, length }]);
    for (value, expected) in [
        ("bytes=0-1", partial(0, 2)),
        ("bytes=-0", partial(3, 0)),
        ("bytes=+0-+1", partial(0, 2)),
        ("bytes=0--0", partial(0, 1)),
        ("bytes=-99", partial(0, 3)),
        ("bytes=2-", partial(2, 1)),
        ("bytes=, ,", RangePlan::Full),
        ("bytes=0-2,0-2", RangePlan::Full),
        ("bytes=9-invalid", RangePlan::Rejected { no_overlap: true }),
        ("bytes=0-invalid", RangePlan::Rejected { no_overlap: false }),
        ("Bytes=0-1", RangePlan::Rejected { no_overlap: false }),
        ("bytes=\u{b}0-1", RangePlan::Rejected { no_overlap: false }),
    ] {
        assert_eq!(
            ranges(value.as_bytes(), 3, limits()),
            Ok(expected),
            "{value}"
        );
    }
    assert_eq!(ranges(b"bytes=0-bad", 0, limits()), Ok(RangePlan::Full));
    assert_eq!(ranges(b"bytes=-0", 0, limits()), Ok(partial(0, 0)));
    assert_eq!(
        ByteRange {
            start: 0,
            length: 0
        }
        .content_range(0),
        "bytes 0--1/0"
    );
    assert_eq!(
        ranges(
            b"bytes=0-0,",
            3,
            Limits {
                items: 1,
                ..limits()
            }
        ),
        Err(Error::ItemLimit)
    );
    assert_eq!(
        ranges(b"bytes=0-,0-", i64::MAX as u64, limits()),
        Err(Error::ArithmeticOverflow)
    );
}

#[test]
fn dates_retain_obsolete_layout_fraction_epoch_zero_and_local_zone_contract() {
    struct Zone;
    impl date::LocalZone for Zone {
        fn offset(&self, name: &[u8], _: i64) -> Option<i32> {
            (name == b"XYZ").then_some(3600)
        }
    }
    let canonical = b"Sun, 06 Nov 1994 08:49:37 GMT";
    let expected = Timestamp::new(784_111_777, 0).unwrap();
    assert_eq!(date::parse(canonical, &Utc), Some(expected));
    for value in [
        b"Sunday, 06-Nov-94 08:49:37 GMT".as_slice(),
        b"Sun Nov  6 08:49:37 1994",
        b"mon, 06 nOv 1994 8:49:37 GMT",
    ] {
        assert_eq!(date::parse(value, &Utc), Some(expected));
    }
    assert_eq!(expected.last_modified().unwrap().as_bytes(), canonical);
    assert_eq!(
        date::parse(b"Sun, 06 Nov 1994 08:49:37.1234567899 GMT", &Utc)
            .unwrap()
            .nanos(),
        123_456_789
    );
    assert!(Timestamp::new(0, 0).unwrap().last_modified().is_none());
    assert!(Timestamp::new(0, 1).unwrap().last_modified().is_some());
    assert!(
        Timestamp::new(-62_135_596_800, 0)
            .unwrap()
            .last_modified()
            .is_none()
    );
    for value in [
        b"Sun, 6 Nov 1994 08:49:37 GMT".as_slice(),
        b" Sun, 06 Nov 1994 08:49:37 GMT",
        b"Sun, 06 Nov 1994 08:49:60 GMT",
    ] {
        assert!(date::parse(value, &Utc).is_none());
    }
    assert_eq!(
        date::parse(b"Sunday, 06-Nov-94 08:49:37 XYZ", &Zone)
            .unwrap()
            .seconds(),
        expected.seconds() - 3600
    );
    assert_eq!(
        date::parse(b"Sunday, 06-Nov-94 08:49:37 GMT+3", &Utc),
        Some(expected)
    );
}

#[test]
fn precondition_order_and_weak_strong_tags_do_not_collapse_into_date_checks() {
    let modified = Timestamp::new(784_111_777, 999_999_999).unwrap();
    let checked =
        |headers, etag| precondition(b"GET", headers, modified, etag, limits(), &Utc).unwrap();
    let unchanged = Conditions {
        if_modified_since: b"Sun, 06 Nov 1994 08:49:37 GMT",
        ..Conditions::default()
    };
    assert_eq!(
        checked(unchanged, b""),
        Precondition::NotModified {
            keep_last_modified: true
        }
    );
    assert_eq!(
        checked(
            Conditions {
                if_match: b"bad",
                ..unchanged
            },
            b""
        ),
        Precondition::Failed
    );
    assert_eq!(
        checked(
            Conditions {
                if_none_match: b"bad",
                ..unchanged
            },
            b""
        ),
        Precondition::Proceed { use_range: true }
    );
    assert_eq!(
        checked(
            Conditions {
                if_match: b"W/\"tag\"",
                ..Conditions::default()
            },
            b"\"tag\""
        ),
        Precondition::Failed
    );
    assert_eq!(
        checked(
            Conditions {
                if_none_match: b"W/\"tag\"",
                ..Conditions::default()
            },
            b"\"tag\""
        ),
        Precondition::NotModified {
            keep_last_modified: false
        }
    );
    assert_eq!(
        checked(
            Conditions {
                if_match: b", *junk",
                ..Conditions::default()
            },
            b""
        ),
        Precondition::Proceed { use_range: true }
    );
    assert_eq!(
        checked(
            Conditions {
                range: b"bytes=0-1",
                if_range: b"W/\"tag\"",
                ..Conditions::default()
            },
            b"\"tag\""
        ),
        Precondition::Proceed { use_range: false }
    );
}

#[test]
fn component_limits_reject_before_parsing_or_allocating_ranges() {
    assert_eq!(
        ranges(
            b"malformed",
            1,
            Limits {
                header_bytes: 8,
                ..limits()
            }
        ),
        Err(Error::HeaderLimit)
    );
    assert_eq!(
        ranges(
            b"",
            0,
            Limits {
                items: 0,
                ..limits()
            }
        ),
        Err(Error::InvalidConfiguration)
    );
    let checked = |value: &[u8]| {
        precondition(
            b"GET",
            Conditions {
                if_match: value,
                ..Conditions::default()
            },
            Timestamp::new(1, 0).unwrap(),
            b"",
            Limits {
                items: 2,
                ..limits()
            },
            &Utc,
        )
    };
    assert_eq!(checked(b",,*"), Err(Error::ItemLimit));
    // Go stops once a matching wildcard is found; remaining bytes are unused.
    assert_eq!(
        checked(b"*,,,"),
        Ok(Precondition::Proceed { use_range: true })
    );
    assert_eq!(
        Timestamp::new(0, 1_000_000_000),
        Err(Error::InvalidTimestamp)
    );
    assert_eq!(Timestamp::new(i64::MAX, 0), Err(Error::InvalidTimestamp));
}
