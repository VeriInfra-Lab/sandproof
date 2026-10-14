use super::*;
use crate::file_download::tests::reference::{Reference, hex};
use axum::http::HeaderValue;
use base64::engine::general_purpose::STANDARD;

fn headers(value: &[u8]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("authorization", HeaderValue::from_bytes(value).unwrap());
    headers
}

#[test]
fn valid_empty_and_malformed_basic_remain_distinct() {
    assert_eq!(explicit_user(&HeaderMap::new(), 8, 64), Ok(None));
    for value in [
        "",
        "Bearer token",
        "Basic malformed",
        "Basic dXNlcg==",
        "Basic dTo",
        "Basic dTo===",
        "Basic\tdTo=",
    ] {
        assert_eq!(explicit_user(&headers(value.as_bytes()), 8, 64), Ok(None));
    }
    for value in ["Basic dXNlcjo=", "bAsIc dXNlcjpzZWNyZXQ=", "BASIC dXNlcjo6"] {
        assert_eq!(
            explicit_user(&headers(value.as_bytes()), 4, 64),
            Ok(Some("user".into()))
        );
    }
    assert_eq!(
        explicit_user(&headers(b"Basic Og=="), 8, 64),
        Ok(Some(String::new()))
    );
}

#[test]
fn go_nonzero_padding_bits_do_not_fall_back_to_default_user() {
    for value in ["Basic dTo=", "Basic dTp=", "Basic dTq=", "Basic dTr="] {
        assert_eq!(
            explicit_user(&headers(value.as_bytes()), 8, 64),
            Ok(Some("u".into()))
        );
    }
}

#[test]
fn budgets_duplicates_and_non_utf8_names_are_explicit_rejections() {
    let mut fields = headers(b"Basic dXNlcjo=");
    assert_eq!(explicit_user(&fields, 3, 64), Err(Error::ResourceExhausted));
    assert_eq!(explicit_user(&fields, 4, 12), Err(Error::ResourceExhausted));
    fields.append("authorization", HeaderValue::from_static("Bearer ignored"));
    assert_eq!(explicit_user(&fields, 4, 64), Err(Error::Unauthenticated));
    assert_eq!(
        explicit_user(&headers(b"Basic /zo="), 4, 64),
        Err(Error::Unauthenticated)
    );
    // Opaque password bytes are ignored, never interpreted as account text.
    assert_eq!(
        explicit_user(&headers(b"Basic dTr/"), 4, 64),
        Ok(Some("u".into()))
    );
}

#[test]
#[ignore = "explicit Go 1.26.8 Request.BasicAuth reference; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
fn fixed_go_basic_auth_parser_reference() {
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    let mut cases: Vec<Vec<u8>> = [
        "",
        "Basic",
        "Basic ",
        "Bearer dTo=",
        "Basic\tdTo=",
        " Basic dTo=",
        "basic dTo=",
        "bAsIc dTo=",
        "BASIC dTo=",
        "Basic dTo",
        "Basic dTo===",
        "Basic dTo= ",
        "Basic d\r\nTo=",
        "Basic dTo=\r\n",
        "Basic Og==",
        "Basic dXNlcg==",
        "Basic /zo=",
        "Basic dTr/",
        "Basic dXNlcjo6",
    ]
    .into_iter()
    .map(|s| s.as_bytes().to_vec())
    .collect();
    for byte in 0..=255 {
        cases.push([b"Basic ".as_slice(), &[byte]].concat());
        cases.push([b"Basic d".as_slice(), &[byte], b"o="].concat());
        cases.push(format!("Basic {}", STANDARD.encode([byte, b':', 0xff])).into_bytes());
        cases.push(format!("Basic {}", STANDARD.encode([b'u', b':', byte])).into_bytes());
    }
    for suffix in b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/" {
        cases.push([b"Basic dT".as_slice(), &[*suffix], b"="].concat());
        cases.push([b"Basic O".as_slice(), &[*suffix], b"=="].concat());
    }
    for raw in &cases {
        let observed =
            parse(raw).map_or_else(|| "NONE".into(), |user| format!("OK\t{}", hex(&user)));
        oracle.check(&format!("B\t{}", hex(raw)), &observed);
    }
    oracle.finish();
    eprintln!(
        "{} original Go Request.BasicAuth vectors; account lookup/HTTP middleware not exercised.",
        cases.len()
    );
}
