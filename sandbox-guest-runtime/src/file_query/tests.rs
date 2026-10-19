use super::*;

fn bind(raw: &[u8]) -> Result<Query, Error> {
    parse(
        raw,
        Limits {
            raw_bytes: 65536,
            fields: 128,
        },
    )
}

#[test]
fn optional_values_and_single_form_decode_preserve_original_bytes() {
    let q = bind(b"path=&username&signature=%FF%00%252B+%2b&signature_expiration=-0").unwrap();
    assert_eq!(q.path, Some(vec![]));
    assert_eq!(q.username, Some(vec![]));
    assert_eq!(q.signature, Some(b"\xff\0%2B +".to_vec()));
    assert_eq!(q.signature_expiration, Some(0));
    assert_eq!(
        q.into_text().err(),
        Some(Error::UnsupportedText(Parameter::Signature))
    );
    let empty = bind(b"unknown=a&&&").unwrap().into_text().unwrap();
    assert!(empty.path.is_none() && empty.username.is_none() && empty.signature.is_none());
    assert_eq!(empty.signature_expiration, None);
}

#[test]
fn invalid_pairs_are_dropped_without_losing_other_fields() {
    let q =
        bind(b"path=good&path=bad%&username=a;b&username=valid&%70ath=%xy&signature=%3b").unwrap();
    assert_eq!(q.path, Some(b"good".to_vec()));
    assert_eq!(q.username, Some(b"valid".to_vec()));
    assert_eq!(q.signature, Some(b";".to_vec()));
    assert_eq!(
        bind(b"path=good&%70ath=").unwrap_err(),
        Error::Duplicate(Parameter::Path)
    );
}

#[test]
fn binding_order_is_independent_of_wire_order_and_bad_integer() {
    for (raw, parameter) in [
        (
            b"signature_expiration=x&username=a&username=b&path=a&path=b".as_slice(),
            Parameter::Path,
        ),
        (
            b"signature=a&signature=b&username=a&username=b",
            Parameter::Username,
        ),
        (
            b"signature_expiration=x&signature=a&signature=b",
            Parameter::Signature,
        ),
        (
            b"signature_expiration=x&signature_expiration=0",
            Parameter::SignatureExpiration,
        ),
    ] {
        assert_eq!(bind(raw).unwrap_err(), Error::Duplicate(parameter));
    }
}

#[test]
fn integer_uses_go_base_ten_sign_syntax_and_unsigned_overflow_order() {
    for (input, expected) in [
        ("9223372036854775807", i64::MAX),
        ("-9223372036854775808", i64::MIN),
        ("%2B00012", 12),
        ("-0", 0),
    ] {
        assert_eq!(
            bind(format!("signature_expiration={input}").as_bytes())
                .unwrap()
                .signature_expiration,
            Some(expected)
        );
    }
    for (input, reason) in [
        ("", IntegerError::Syntax),
        ("+1", IntegerError::Syntax),
        ("0x10", IntegerError::Syntax),
        ("1_0", IntegerError::Syntax),
        ("9223372036854775808x", IntegerError::Syntax),
        ("18446744073709551616x", IntegerError::Range),
        ("-9223372036854775809", IntegerError::Range),
    ] {
        assert!(
            matches!(bind(format!("signature_expiration={input}").as_bytes()), Err(Error::Integer {reason: found,..}) if found == reason)
        );
    }
}

#[test]
fn raw_bytes_and_all_nonempty_pairs_are_charged_before_binding() {
    let limits = Limits {
        raw_bytes: 13,
        fields: 2,
    };
    assert!(parse(b"path=x&&&bad%", limits).is_ok());
    assert_eq!(
        parse(b"path=x&&&bad%x", limits),
        Err(Error::ResourceExhausted)
    );
    for raw in [b"a&a&a".as_slice(), b"%&;&?", b"path&path&x"] {
        assert_eq!(parse(raw, limits), Err(Error::ResourceExhausted));
    }
    for bad in [
        Limits {
            raw_bytes: 0,
            fields: 1,
        },
        Limits {
            raw_bytes: 1,
            fields: 0,
        },
        Limits {
            raw_bytes: usize::MAX,
            fields: 1,
        },
    ] {
        assert_eq!(parse(b"", bad), Err(Error::InvalidConfiguration));
    }
}
