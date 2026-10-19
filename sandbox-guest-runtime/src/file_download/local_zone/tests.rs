use super::*;
use crate::file_download::date;
use time::{Date, Month};

#[derive(Clone)]
struct Type(i32, bool, &'static str);
fn limits() -> Limits {
    Limits {
        input_bytes: 65536,
        transitions: 1024,
        zones: 256,
    }
}
fn block_bytes(
    version: u8,
    width: usize,
    types: &[Type],
    transitions: &[(i64, u8)],
    leaps: &[(i64, i32)],
) -> Vec<u8> {
    let mut names = Vec::new();
    let mut indexes = Vec::new();
    for Type(_, _, name) in types {
        indexes.push(u8::try_from(names.len()).unwrap());
        names.extend_from_slice(name.as_bytes());
        names.push(0);
    }
    let mut bytes = b"TZif".to_vec();
    bytes.push(version);
    bytes.extend_from_slice(&[0; 15]);
    for count in [
        0,
        0,
        leaps.len(),
        transitions.len(),
        types.len(),
        names.len(),
    ] {
        bytes.extend_from_slice(&u32::try_from(count).unwrap().to_be_bytes());
    }
    let append_time = |bytes: &mut Vec<u8>, seconds: i64| {
        if width == 4 {
            bytes.extend_from_slice(&i32::try_from(seconds).unwrap().to_be_bytes());
        } else {
            bytes.extend_from_slice(&seconds.to_be_bytes());
        }
    };
    for (seconds, _) in transitions {
        append_time(&mut bytes, *seconds);
    }
    bytes.extend(transitions.iter().map(|(_, index)| index));
    for (Type(offset, dst, _), index) in types.iter().zip(indexes) {
        bytes.extend_from_slice(&offset.to_be_bytes());
        bytes.extend_from_slice(&[u8::from(*dst), index]);
    }
    bytes.extend(names);
    for (seconds, correction) in leaps {
        append_time(&mut bytes, *seconds);
        bytes.extend_from_slice(&correction.to_be_bytes());
    }
    bytes
}
fn tzif(
    version: u8,
    types: &[Type],
    transitions: &[(i64, u8)],
    leaps: &[(i64, i32)],
    footer: &str,
) -> Vec<u8> {
    if version == 0 {
        return block_bytes(0, 4, types, transitions, leaps);
    }
    // A different, deliberately irrelevant 32-bit block detects wrong selection.
    let mut bytes = block_bytes(version, 4, &[Type(1234, false, "OLD")], &[], &[]);
    bytes.extend(block_bytes(version, 8, types, transitions, leaps));
    bytes.extend_from_slice(format!("\n{footer}\n").as_bytes());
    bytes
}
fn instant(year: i32, month: u8, day: u8) -> i64 {
    Date::from_calendar_date(year, Month::try_from(month).unwrap(), day)
        .unwrap()
        .midnight()
        .assume_utc()
        .unix_timestamp()
}
fn historical() -> Vec<Vec<u8>> {
    let types = [
        Type(3600, true, "AAA"),
        Type(7200, false, "BBB"),
        Type(10800, true, "CCC"),
    ];
    vec![
        tzif(0, &[Type(3661, false, "XYZ")], &[], &[], ""),
        tzif(0, &types, &[(100, 2), (200, 0)], &[], ""),
        tzif(0, &types, &[(100, 2)], &[], ""),
        tzif(0, &types, &[(100, 1), (200, 0)], &[], ""),
        tzif(
            0,
            &[Type(3600, true, "AAA"), Type(7200, true, "BBB")],
            &[(100, 1), (200, 0)],
            &[],
            "",
        ),
        tzif(
            0,
            &[Type(0, false, "XYZ"), Type(3600, false, "XYZ")],
            &[(1000, 1), (10000, 0)],
            &[],
            "",
        ),
        tzif(
            0,
            &[Type(0, false, "STD"), Type(3600, true, "DST")],
            &[(78_796_810, 1)],
            &[(78_796_800, 1)],
            "",
        ),
    ]
}
fn extended(version: u8, transitions: bool) -> Vec<u8> {
    tzif(
        version,
        &[Type(-18000, false, "EST"), Type(-14400, true, "EDT")],
        if transitions {
            &[(0, 1), (1_577_836_800, 0)]
        } else {
            &[]
        },
        &[],
        "EST5EDT,M3.2.0,M11.1.0",
    )
}

#[test]
fn first_zone_transitions_duplicate_names_and_last_zone_follow_go() {
    let cases = historical();
    for (index, before, after) in [
        (1, 7200, 3600),
        (2, 3600, 10800),
        (3, 7200, 3600),
        (4, 3600, 3600),
    ] {
        let zone = Snapshot::from_tzif(&cases[index], limits()).unwrap();
        assert_eq!(zone.lookup(99).ut_offset(), before);
        assert_eq!(zone.lookup(201).ut_offset(), after);
        assert_eq!(zone.lookup(i64::MAX).ut_offset(), after);
    }
    let zone = Snapshot::from_tzif(&cases[5], limits()).unwrap();
    assert_eq!(zone.offset(b"XYZ", 999), Some(0));
    assert_eq!(zone.offset(b"XYZ", 2000), Some(3600));
    assert_eq!(zone.offset(b"XYZ", 10000), Some(0));
    assert_eq!(zone.offset(b"unknown", 2000), None);
    let fixed = Snapshot::from_tzif(&cases[0], limits()).unwrap();
    drop(cases);
    assert_eq!(fixed.offset(b"XYZ", 0), Some(3661));
    assert_eq!(
        date::parse(b"Sunday, 06-Nov-94 08:49:37 XYZ", &fixed)
            .unwrap()
            .seconds(),
        784_111_777 - 3661
    );
}

#[test]
fn leaps_are_ignored_and_footer_is_used_only_after_last_transition() {
    let cases = historical();
    let leap = Snapshot::from_tzif(&cases[6], limits()).unwrap();
    assert_eq!(leap.lookup(78_796_809).ut_offset(), 0);
    assert_eq!(leap.lookup(78_796_810).ut_offset(), 3600);
    for version in *b"23" {
        for transitions in [false, true] {
            let zone = Snapshot::from_tzif(&extended(version, transitions), limits()).unwrap();
            assert_eq!(zone.lookup(instant(2050, 1, 1)).ut_offset(), -18000);
            assert_eq!(zone.lookup(instant(2050, 7, 1)).ut_offset(), -14400);
            // Unknown names are not added by a footer or selected from block 1.
            assert_eq!(zone.offset(b"OLD", instant(2050, 1, 1)), None);
            if transitions {
                assert_eq!(zone.lookup(instant(2019, 1, 1)).ut_offset(), -14400);
            }
        }
    }
}

#[test]
fn budgets_and_malformed_sources_fail_without_a_fallback_snapshot() {
    let bytes = historical().remove(1);
    for field in 0..3 {
        for value in [0, usize::MAX] {
            let mut caps = limits();
            match field {
                0 => caps.input_bytes = value,
                1 => caps.transitions = value,
                _ => caps.zones = value,
            }
            assert!(matches!(
                Snapshot::from_tzif(&[], caps),
                Err(Error::InvalidConfiguration)
            ));
        }
    }
    assert!(matches!(
        Snapshot::from_tzif(
            &bytes,
            Limits {
                input_bytes: bytes.len() - 1,
                ..limits()
            }
        ),
        Err(Error::InputLimit)
    ));
    assert!(matches!(
        Snapshot::from_tzif(
            &bytes,
            Limits {
                transitions: 1,
                ..limits()
            }
        ),
        Err(Error::TransitionLimit)
    ));
    assert!(matches!(
        Snapshot::from_tzif(
            &bytes,
            Limits {
                zones: 2,
                ..limits()
            }
        ),
        Err(Error::ZoneLimit)
    ));
    assert!(
        Snapshot::from_tzif(
            &bytes,
            Limits {
                input_bytes: bytes.len(),
                transitions: 2,
                zones: 3
            }
        )
        .is_ok()
    );
    for n in 0..bytes.len() {
        assert!(matches!(
            Snapshot::from_tzif(&bytes[..n], limits()),
            Err(Error::InvalidData)
        ));
    }
    for (at, value) in [(0, b'X'), (4, b'4'), (53, 250)] {
        let mut invalid = bytes.clone();
        invalid[at] = value;
        assert!(matches!(
            Snapshot::from_tzif(&invalid, limits()),
            Err(Error::InvalidData)
        ));
    }
    for count in 0..6 {
        let mut invalid = bytes.clone();
        invalid[20 + count * 4..24 + count * 4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(matches!(
            Snapshot::from_tzif(&invalid, limits()),
            Err(Error::InvalidData)
        ));
    }
    let mut invalid = extended(b'2', true);
    invalid.pop();
    assert!(matches!(
        Snapshot::from_tzif(&invalid, limits()),
        Err(Error::InvalidData)
    ));
}

#[test]
fn second_block_counts_and_version_are_checked_before_parsing() {
    let bytes = extended(b'3', true);
    let first = block_bytes(b'3', 4, &[Type(1234, false, "OLD")], &[], &[]).len();
    for (caps, error) in [
        (
            Limits {
                transitions: 1,
                ..limits()
            },
            Error::TransitionLimit,
        ),
        (
            Limits {
                zones: 1,
                ..limits()
            },
            Error::ZoneLimit,
        ),
    ] {
        assert!(matches!(Snapshot::from_tzif(&bytes, caps), Err(actual) if actual == error));
    }
    for end in first..first + 44 {
        assert!(matches!(
            Snapshot::from_tzif(&bytes[..end], limits()),
            Err(Error::InvalidData)
        ));
    }
    let mut invalid = bytes.clone();
    invalid[first + 4] = b'2';
    assert!(matches!(
        Snapshot::from_tzif(&invalid, limits()),
        Err(Error::InvalidData)
    ));
    for count in 0..6 {
        let mut invalid = bytes.clone();
        invalid[first + 20 + count * 4..first + 24 + count * 4]
            .copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(matches!(
            Snapshot::from_tzif(&invalid, limits()),
            Err(Error::InvalidData)
        ));
    }
}

mod reference;
