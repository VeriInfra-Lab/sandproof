use super::*;
use crate::file_download::tests::reference::{Reference, hex};
use axum::http::{HeaderName, HeaderValue};
use std::fmt::Write;
#[test]
fn metadata_entry_failures_precede_total_and_total_counts_every_valid_field() {
    for reversed in [false, true] {
        let mut headers = HeaderMap::new();
        let mut pairs = [
            (
                "x-metadata-large",
                HeaderValue::from_str(&"x".repeat(4096)).unwrap(),
            ),
            ("x-metadata-invalid", HeaderValue::from_static("ok\t")),
        ];
        if reversed {
            pairs.reverse();
        }
        for (name, value) in pairs {
            headers.insert(name, value);
        }
        assert_eq!(
            metadata_detailed(&headers),
            Err(MetadataFault {
                key: "invalid".into(),
                reason: MetadataReason::Entry(file_upload::MetadataViolation::ValueByte {
                    byte: 9,
                    offset: 2
                }),
            })
        );
        headers.insert("x-metadata-invalid", HeaderValue::from_static("fine"));
        assert_eq!(
            metadata_detailed(&headers),
            Err(MetadataFault {
                key: String::new(),
                reason: MetadataReason::Total(4096 + 9 + 5 + 9 + 7 + 4),
            })
        );
    }
}
#[test]
fn metadata_uses_first_value_ignores_empty_key_and_checks_before_copying() {
    let mut headers = HeaderMap::new();
    headers.append("x-metadata-key", HeaderValue::from_static("first"));
    headers.append(
        "x-metadata-key",
        HeaderValue::from_static("\tinvalid ignored"),
    );
    headers.insert("x-metadata-", HeaderValue::from_static("ignored"));
    assert_eq!(
        metadata(&headers).unwrap(),
        BTreeMap::from([("key".into(), "first".into())])
    );
    headers.insert(
        "x-metadata-limit",
        HeaderValue::from_str(&"x".repeat(4096)).unwrap(),
    );
    assert_eq!(metadata(&headers), Err(Error::InvalidMetadata));
    assert_eq!(
        check_headers(
            &headers,
            HeaderLimits {
                bytes: 8192,
                fields: 3
            }
        ),
        Err(Error::HeaderLimit)
    );
    assert_eq!(
        check_headers(
            &headers,
            HeaderLimits {
                bytes: 1,
                fields: 32
            }
        ),
        Err(Error::HeaderLimit)
    );
}
fn next(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    *seed
}
#[test]
#[ignore = "requires explicit fixed Go upload reference"]
fn upload_fault_status_matches_original_process_file() {
    use sandproof_guest_runtime::{
        file_download::LinuxErrno,
        file_upload::{Fault, Stage},
    };
    let mut reference = Reference::from_env("SANDPROOF_GUEST_UPLOAD_REFERENCE");
    let mut cases = 0;
    for stage in [
        Stage::ParentStat,
        Stage::ParentNotDirectory,
        Stage::ParentCreate,
        Stage::ParentChown,
        Stage::Stat,
        Stage::Directory,
        Stage::Chown,
        Stage::Open,
        Stage::FileChown,
        Stage::Write,
        Stage::WriteZero,
        Stage::MetadataList,
        Stage::MetadataRemove,
        Stage::MetadataSet,
    ] {
        // Ordinary I/O, permissions, disk full and quota. No injected ENOENT
        // or unsupported-xattr error that the original operation ignores.
        for errno in [5, 13, 28, 122] {
            let fault = Fault {
                stage,
                path: b"/file".to_vec(),
                errno: Some(LinuxErrno(errno)),
                attribute: None,
            };
            reference.check(
                &format!("F\t{stage:?}\t{errno}"),
                &fault.status().to_string(),
            );
            cases += 1;
        }
    }
    reference.finish();
    eprintln!("Fixed Go processFile: {cases} injected phase/errno status cases.");
}
#[test]
#[ignore = "requires explicit fixed Go upload reference"]
fn fixed_go_upload_selection_and_metadata_match_original_methods() {
    let mut reference = Reference::from_env("SANDPROOF_GUEST_UPLOAD_REFERENCE");
    let bases = [
        "multİpart/form-data",
        "applİcation/octet-stream",
        "application/octet-stream",
        "APPLICATION/OCTET-STREAM",
        "multipart/form-data",
        "multipart/related",
        "multipart/mixed",
        "multipart",
        "",
        "a/b/c",
        " text/plain ",
        "\u{a0}application/octet-stream\u{2000}",
    ];
    let suffixes: &[&[u8]] = &[
        b"",
        b";",
        b";;",
        b"; a=1",
        b"; a=1;a=2",
        b";a=1;A=1",
        b";a*=utf-8''%ff",
        b";a*0=1;a*0=2",
        b";a=\"\xff\";a=\"\xfe\"",
        b";a=\"\xff\";a=\"\xff\"",
        b";a=\"C:\\path\\file\"",
        b";a=\"\\\"\"",
        b";a=\"unterminated",
        b";a=\"\r\n\"",
        b"; bad;a=1;a=2",
        b";a=1 bad",
        b";a=1;\xff=1",
    ];
    let mut content: Vec<Vec<u8>> = bases
        .iter()
        .flat_map(|base| {
            suffixes
                .iter()
                .map(move |suffix| [base.as_bytes(), *suffix].concat())
        })
        .collect();
    let alphabet = b";a*=1\"\\ \t\r\n\xff\xfe/%";
    let mut seed = 0x311;
    for _ in 0..5000 {
        let base = bases[usize::try_from(next(&mut seed) % bases.len() as u64).unwrap()];
        let n = usize::try_from(next(&mut seed) % 40).unwrap();
        let mut value = base.as_bytes().to_vec();
        value.extend(
            (0..n).map(|_| {
                alphabet[usize::try_from(next(&mut seed) % alphabet.len() as u64).unwrap()]
            }),
        );
        content.push(value);
    }
    for value in &content {
        let result = file_upload::kind(value, 65536).unwrap();
        reference.check(
            &format!("C\t{}", hex(value)),
            match result {
                file_upload::Kind::Raw => "R",
                file_upload::Kind::Multipart => "M",
                file_upload::Kind::Unsupported => "U",
            },
        );
    }
    let cases = metadata_reference(&mut reference);
    reference.finish();
    eprintln!(
        "Fixed Go upload headers: {} media types, {cases} metadata field cases.",
        content.len()
    );
}
fn metadata_reference(reference: &mut Reference) -> usize {
    let values: Vec<Vec<u8>> = (0..=255u8)
        .filter(|b| HeaderValue::from_bytes(&[*b]).is_ok())
        .map(|b| vec![b])
        .chain([vec![], vec![b'x'; 4086], vec![b'x'; 4087]])
        .collect();
    let mut cases = 0;
    for name in ["X-Metadata-Key", "x-metadata-Mixed", "X-Metadata-", "Other"] {
        for value in &values {
            for duplicate in [false, true] {
                let mut headers = HeaderMap::new();
                let key = HeaderName::from_bytes(name.as_bytes()).unwrap();
                headers.append(key.clone(), HeaderValue::from_bytes(value).unwrap());
                let mut input = format!("M\t{}:{}", hex(name.as_bytes()), hex(value));
                if duplicate {
                    headers.append(key, HeaderValue::from_static("second"));
                    write!(input, "\t{}:{}", hex(name.as_bytes()), hex(b"second")).unwrap();
                }
                let result = match metadata(&headers) {
                    Err(Error::InvalidMetadata) => "ERR".into(),
                    Ok(values) => {
                        let mut output = String::from("OK");
                        for (key, value) in values {
                            write!(
                                output,
                                "\t{}:{}",
                                hex(key.as_bytes()),
                                hex(value.as_bytes())
                            )
                            .unwrap();
                        }
                        output
                    }
                    other => panic!("unexpected {other:?}"),
                };
                reference.check(&input, &result);
                cases += 1;
            }
        }
    }
    cases
}
