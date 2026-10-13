use super::*;
#[test]
fn detailed_metadata_checks_order_and_original_byte_offsets() {
    assert_eq!(
        validate_entry_detailed(b"", b"\xff"),
        Err(MetadataViolation::EmptyKey)
    );
    assert_eq!(
        validate_entry_detailed(&vec![0xff; 247], b"\0"),
        Err(MetadataViolation::KeyLength)
    );
    assert_eq!(
        validate_entry_detailed(b"a\xff", b"\0"),
        Err(MetadataViolation::KeyByte {
            byte: 255,
            offset: 1
        })
    );
    assert_eq!(
        validate_entry_detailed(b"key", b"ok\xc3\xa9"),
        Err(MetadataViolation::ValueByte {
            byte: 195,
            offset: 2
        })
    );
    for byte in 0..=255u8 {
        let result = validate_entry_detailed(b"key", &[byte]);
        assert_eq!(result.is_ok(), (32..=126).contains(&byte));
        assert_eq!(
            validate_entry(b"key", &[byte]),
            result.map_err(|_| Error::InvalidMetadata)
        );
    }
}
#[test]
fn returned_type_distinguishes_bad_parameters_from_conflicting_duplicates() {
    for value in [
        b"APPLICATION/OCTET-STREAM".as_slice(),
        b"application/octet-stream; broken",
        b"application/octet-stream; a=1;a=1",
        b"application/octet-stream; a*=utf-8''%ff",
    ] {
        assert_eq!(kind(value, 4096), Ok(Kind::Raw));
    }
    for value in [
        b"application/octet-stream; a=1;a=2".as_slice(),
        b"application/octet-stream/extra",
        b"multipart",
        b"",
    ] {
        assert_eq!(kind(value, 4096), Ok(Kind::Unsupported));
    }
    assert_eq!(
        kind(b"multipart/related; boundary=bad", 4096),
        Ok(Kind::Multipart)
    );
    assert_eq!(
        kind(b"application/octet-stream", 1),
        Err(Error::HeaderLimit)
    );
}
#[test]
fn stored_metadata_limits_remain_shared_with_real_io() {
    let mut data = BTreeMap::from([("x".into(), "v".repeat(4086))]);
    assert_eq!(validate_metadata(&data), Ok(()));
    data.get_mut("x").unwrap().push('v');
    assert_eq!(validate_metadata(&data), Err(Error::InvalidMetadata));
    for (key, value) in [
        ("", "ok"),
        ("key", "\t"),
        ("key", "\u{7f}"),
        ("key", "中文"),
    ] {
        assert_eq!(
            validate_metadata(&BTreeMap::from([(key.into(), value.into())])),
            Err(Error::InvalidMetadata)
        );
    }
    assert_eq!(
        validate_metadata(&BTreeMap::from([("x".repeat(247), String::new())])),
        Err(Error::InvalidMetadata)
    );
}
