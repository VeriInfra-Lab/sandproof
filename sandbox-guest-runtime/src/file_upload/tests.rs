use super::*;
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
