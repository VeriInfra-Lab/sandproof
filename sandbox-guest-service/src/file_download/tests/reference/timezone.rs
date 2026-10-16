//! The actual `TZif` parser feeds the response planner; no stand-in `LocalZone`.
use super::*;
use wire::{
    date,
    local_zone::{Limits as ZoneLimits, Snapshot},
};

pub(crate) const MODIFIED: i64 = 784_111_777;
pub(crate) fn zone_limits() -> ZoneLimits {
    ZoneLimits {
        input_bytes: 4096,
        transitions: 16,
        zones: 16,
    }
}

fn block(
    version: u8,
    width: usize,
    types: &[(i32, bool, &str)],
    transitions: &[(i64, u8)],
) -> Vec<u8> {
    let mut names = Vec::new();
    let mut indexes = Vec::new();
    for (_, _, name) in types {
        indexes.push(u8::try_from(names.len()).unwrap());
        names.extend_from_slice(name.as_bytes());
        names.push(0);
    }
    let mut bytes = b"TZif".to_vec();
    bytes.push(version);
    bytes.extend_from_slice(&[0; 15]);
    for n in [0, 0, 0, transitions.len(), types.len(), names.len()] {
        bytes.extend_from_slice(&u32::try_from(n).unwrap().to_be_bytes());
    }
    for (seconds, _) in transitions {
        if width == 4 {
            bytes.extend_from_slice(&i32::try_from(*seconds).unwrap().to_be_bytes());
        } else {
            bytes.extend_from_slice(&seconds.to_be_bytes());
        }
    }
    bytes.extend(transitions.iter().map(|(_, index)| index));
    for ((offset, dst, _), index) in types.iter().zip(indexes) {
        bytes.extend_from_slice(&offset.to_be_bytes());
        bytes.extend_from_slice(&[u8::from(*dst), index]);
    }
    bytes.extend(names);
    bytes
}

pub(crate) fn fixed_material() -> Vec<u8> {
    block(0, 4, &[(3661, false, "XYZ")], &[])
}

fn materials() -> Vec<Vec<u8>> {
    let mut materials = vec![
        fixed_material(),
        block(
            0,
            4,
            &[(0, false, "XYZ"), (3600, false, "XYZ")],
            &[(MODIFIED - 7200, 1), (MODIFIED + 7200, 0)],
        ),
    ];
    for version in *b"23" {
        let mut bytes = block(version, 4, &[(0, false, "OLD")], &[]);
        bytes.extend(block(
            version,
            8,
            &[(3600, false, "STD"), (7200, true, "DST")],
            &[(0, 0)],
        ));
        bytes.extend_from_slice(b"\nSTD-1DST-2,M3.2.0,M11.1.0\n");
        materials.push(bytes);
    }
    materials
}

#[test]
fn parsed_snapshot_changes_date_conditions_without_changing_utc_last_modified() {
    let zone = Snapshot::from_tzif(&fixed_material(), zone_limits()).unwrap();
    // 09:50:38 XYZ (+01:01:01) is exactly the modification second in UTC.
    for (name, value, expected) in [
        (
            "if-modified-since",
            "Sunday, 06-Nov-94 09:50:37 XYZ",
            StatusCode::OK,
        ),
        (
            "if-modified-since",
            "Sunday, 06-Nov-94 09:50:38 XYZ",
            StatusCode::NOT_MODIFIED,
        ),
        (
            "if-unmodified-since",
            "Sunday, 06-Nov-94 09:50:37 XYZ",
            StatusCode::PRECONDITION_FAILED,
        ),
        (
            "if-range",
            "Sunday, 06-Nov-94 09:50:38 XYZ",
            StatusCode::PARTIAL_CONTENT,
        ),
        ("if-range", "Sunday, 06-Nov-94 09:50:37 XYZ", StatusCode::OK),
    ] {
        let mut request = HeaderMap::new();
        request.insert(name, HeaderValue::from_static(value));
        if name == "if-range" {
            request.insert("range", HeaderValue::from_static("bytes=1-2"));
        }
        let result = match prepare(
            &Method::GET,
            &request,
            initial(),
            Timestamp::new(MODIFIED, 123).unwrap(),
            limits(),
            &zone,
        )
        .unwrap()
        {
            Preparation::Finished(plan) => plan,
            Preparation::Continue(next) => next
                .finish(
                    5,
                    HeaderValue::from_static("application/octet-stream"),
                    None,
                )
                .unwrap(),
        };
        assert_eq!(result.status, expected, "{name}: {value}");
        assert_eq!(
            result.headers["last-modified"],
            "Sun, 06 Nov 1994 08:49:37 GMT"
        );
    }
}

#[test]
#[ignore = "explicit Go 1.26.8 TZif/ServeContent oracle; set SANDPROOF_GUEST_DOWNLOAD_REFERENCE"]
fn fixed_go_tzif_download_conditions() {
    let mut oracle = Reference::new();
    let dates = [
        "Sunday, 06-Nov-94 08:49:37 XYZ",
        "Sunday, 06-Nov-94 09:49:37 XYZ",
        "Sunday, 06-Nov-94 09:50:38 XYZ",
        "Sunday, 06-Nov-94 10:49:37 XYZ",
        "Sunday, 06-Nov-94 09:49:37 STD",
        "Sunday, 06-Nov-94 10:49:37 DST",
        "Sunday, 06-Nov-94 08:49:37 ABC",
        "Sunday, 06-Nov-94 08:49:37 GMT",
        "Sunday, 06-Nov-94 09:50:38.123 XYZ",
        "Monday, 01-Jan-68 01:00:00 STD",
        "Friday, 01-Jun-68 02:00:00 DST",
        "Sunday, 04-Nov-68 01:30:00 STD",
        "Sunday, 04-Nov-68 01:30:00 DST",
        "Sun, 06 Nov 1994 08:49:37 GMT",
        "Sun Nov  6 08:49:37 1994",
        "bad date",
    ];
    let mut count = 0;
    let mut statuses = std::collections::BTreeSet::new();
    for bytes in materials() {
        let zone = Snapshot::from_tzif(&bytes, zone_limits()).unwrap();
        for value in dates {
            // Boundary instants are selected here; the independent Go parser
            // and ServeContent decide all expected statuses, headers and bytes.
            let parsed = date::parse(value.as_bytes(), &zone).map_or(MODIFIED, Timestamp::seconds);
            for seconds in [parsed - 1, parsed, parsed + 1, MODIFIED] {
                for method in ["GET", "HEAD"] {
                    for condition in [1, 3, 4] {
                        for range in ["", "bytes=1-2", "bytes=0-0,3-4", "bytes=99-"] {
                            let mut case = Case::new();
                            case.method = method;
                            case.modified = Timestamp::new(seconds, 123).unwrap();
                            case.headers[condition] = value.as_bytes().to_vec();
                            case.headers[5] = range.as_bytes().to_vec();
                            statuses.insert(
                                case.check_local(&mut oracle, &zone, Some(&bytes)).as_u16(),
                            );
                            count += 1;
                        }
                    }
                    // Tag conditions suppress date checks even with a real
                    // non-UTC zone. Their precedence is not a zone fallback.
                    for (tag, date) in [(0, 1), (2, 3)] {
                        let mut case = Case::new();
                        case.method = method;
                        case.modified = Timestamp::new(seconds, 0).unwrap();
                        case.headers[tag] = b"*".to_vec();
                        case.headers[date] = value.as_bytes().to_vec();
                        case.check_local(&mut oracle, &zone, Some(&bytes));
                        count += 1;
                    }
                }
            }
        }
    }
    assert_eq!(statuses, [200, 206, 304, 412, 416].into_iter().collect());
    oracle.finish();
    eprintln!("Fixed Go TZif/ServeContent: {count} condition/status/header/body combinations.");
}
