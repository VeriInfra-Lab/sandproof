use super::*;
use crate::file_download::tests::reference::{Reference, hex};
use http_body_util::BodyExt;
use std::{sync::Arc, time::Duration};
use tokio::sync::Semaphore;

fn query_limits() -> QueryLimits {
    QueryLimits {
        raw_bytes: 65536,
        fields: 128,
    }
}
fn response_limits() -> ResponseLimits {
    ResponseLimits {
        wire_bytes: 65536,
        chunk_bytes: 17,
    }
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(30)
}
fn failure(raw: &[u8]) -> BindingError {
    parse(raw, query_limits()).unwrap_err()
}
fn owner() -> (Owner, Arc<Semaphore>) {
    let slots = Arc::new(Semaphore::new(1));
    (
        Arc::new(Arc::clone(&slots).try_acquire_owned().unwrap()),
        slots,
    )
}
async fn released(slots: Arc<Semaphore>) {
    let _slot = tokio::time::timeout(Duration::from_secs(5), slots.acquire_owned())
        .await
        .unwrap()
        .unwrap();
}
async fn drain(mut body: axum::body::Body) -> Vec<u8> {
    let mut bytes = vec![];
    while let Some(frame) = body.frame().await {
        let chunk = frame.unwrap().into_data().unwrap();
        bytes.extend_from_slice(&chunk);
    }
    bytes
}

#[test]
fn plain_text_errors_keep_go_quote_bytes_and_exact_wire_budget() {
    for (raw, expected) in [
        (b"path=first&path=second".as_slice(), b"Invalid format for parameter path: multiple values for single value parameter 'path'\n".as_slice()),
        (b"signature_expiration=%FF%22%0A%3C", b"Invalid format for parameter signature_expiration: error binding string parameter: strconv.ParseInt: parsing \"\\xff\\\"\\n<\": invalid syntax\n"),
    ] {
        let error = failure(raw);
        assert_eq!(payload(&error, expected.len()).unwrap(), expected);
        assert_eq!(payload(&error, expected.len() - 1), Err(ResponseError::ResponseLimit));
    }
    for error in [
        BindingError::InvalidConfiguration,
        BindingError::ResourceExhausted,
        BindingError::UnsupportedText(Parameter::Path),
    ] {
        assert_eq!(
            payload(&error, 65536),
            Err(ResponseError::UnclassifiedFault)
        );
    }
}

#[tokio::test]
async fn errors_keep_original_capacity_through_last_clone_and_original_expiry() {
    for (expire, chunk_bytes) in [(false, 17), (true, 17), (false, 65536), (true, 65536)] {
        let (guard, slots) = owner();
        let response = rejected(
            failure(b"path=a&path=b"),
            ResponseLimits {
                chunk_bytes,
                ..response_limits()
            },
            deadline(),
            guard,
        )
        .await
        .unwrap();
        assert_eq!(response.status(), 400);
        assert_eq!(
            response.headers()["content-type"],
            "text/plain; charset=utf-8"
        );
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
        let mut body = response.into_body();
        let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
        let copy = bytes.clone();
        drop(bytes);
        if expire {
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(31)).await;
            assert!(body.frame().await.unwrap().is_err());
            tokio::time::resume();
        }
        drop(body);
        assert_eq!(slots.available_permits(), 0);
        drop(copy);
        released(slots).await;
    }
}

#[tokio::test]
async fn preparation_failures_release_guard_and_do_not_create_a_reply() {
    for (limit, end, expected) in [
        (
            ResponseLimits {
                wire_bytes: 0,
                ..response_limits()
            },
            deadline(),
            ResponseError::InvalidConfiguration,
        ),
        (
            ResponseLimits {
                chunk_bytes: 0,
                ..response_limits()
            },
            deadline(),
            ResponseError::InvalidConfiguration,
        ),
        (
            ResponseLimits {
                wire_bytes: 1,
                ..response_limits()
            },
            deadline(),
            ResponseError::ResponseLimit,
        ),
        (
            response_limits(),
            Instant::now(),
            ResponseError::DeadlineExceeded,
        ),
    ] {
        let (guard, slots) = owner();
        assert_eq!(
            rejected(failure(b"username=a&username=b"), limit, end, guard)
                .await
                .err(),
            Some(expected)
        );
        released(slots).await;
    }
}

#[tokio::test]
async fn go_http_error_buffer_length_boundary() {
    for length in [2047, 2048, 2049] {
        let fixed = payload(&failure(b"signature_expiration=x"), 65536)
            .unwrap()
            .len()
            - 1;
        let raw = format!("signature_expiration={}", "x".repeat(length - fixed));
        let (guard, slots) = owner();
        let response = rejected(
            failure(raw.as_bytes()),
            response_limits(),
            deadline(),
            guard,
        )
        .await
        .unwrap();
        assert_eq!(
            response
                .headers()
                .get("content-length")
                .map(|v| v.to_str().unwrap()),
            (length <= 2048).then(|| length.to_string()).as_deref()
        );
        assert_eq!(drain(response.into_body()).await.len(), length);
        released(slots).await;
    }
}

fn cases() -> Vec<Vec<u8>> {
    let fields = [
        "",
        "unknown=x",
        "path",
        "path=",
        "path=ok",
        "%70ath=two",
        "path=%GG",
        "path=a;b",
        "path=%3B",
        "username=u",
        "username=",
        "signature=s",
        "signature=",
        "signature_expiration=0",
        "signature_expiration=bad",
        "signature_expiration=",
        "signature_expiration=%2B1",
        "signature_expiration=%FF",
    ];
    let mut cases = vec![vec![]];
    for first in fields {
        for second in fields {
            cases.push(format!("{first}&{second}").into_bytes());
        }
    }
    for value in [
        "9223372036854775807",
        "9223372036854775808",
        "-9223372036854775808",
        "-9223372036854775809",
        "18446744073709551615x",
        "18446744073709551616x",
        "000000000000000000000001",
        "0x10",
        "1_0",
        "+1",
        "%2B1",
        "-",
        "%2B",
        "nan",
    ] {
        cases.push(format!("signature_expiration={value}").into_bytes());
    }
    for byte in 0..=255 {
        for field in ["path", "username", "signature", "signature_expiration"] {
            cases.push(format!("{field}=%{byte:02x}").into_bytes());
        }
    }
    for value in [
        "\u{00ad}",
        "\u{2028}",
        "\u{2029}",
        "中文💬",
        "\u{e000}",
        "\u{10ffff}",
    ] {
        cases.push(format!("signature_expiration={value}").into_bytes());
    }
    let mut seed = 0x311u64;
    let mut next = |n: usize| {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        usize::try_from(seed % n as u64).unwrap()
    };
    for _ in 0..4096 {
        let mut raw = vec![];
        for _ in 0..next(9) {
            raw.extend_from_slice(fields[next(fields.len())].as_bytes());
            if next(3) == 0 {
                raw.extend_from_slice(format!("%{:02x}", next(256)).as_bytes());
            }
            raw.push(b'&');
        }
        cases.push(raw);
    }
    cases
}

#[test]
#[ignore = "explicit Go 1.26.8 / oapi runtime v1.6.0 oracle; set SANDPROOF_GUEST_QUERY_REFERENCE"]
fn fixed_go_query_binding_reference() {
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_QUERY_REFERENCE");
    let cases = cases();
    for raw in &cases {
        let observed = match parse(raw, query_limits()) {
            Ok(query) => {
                let value =
                    |v: Option<Vec<u8>>| v.map_or_else(|| "-".into(), |v| format!("+{}", hex(&v)));
                format!(
                    "OK\t{}\t{}\t{}\t{}",
                    value(query.path),
                    value(query.username),
                    value(query.signature),
                    query
                        .signature_expiration
                        .map_or_else(|| "-".into(), |v| v.to_string())
                )
            }
            Err(error) => format!("ERR\t{}", hex(&payload(&error, 65536).unwrap())),
        };
        oracle.check(&format!("P\t{}", hex(raw)), &observed);
    }
    oracle.finish();
    eprintln!(
        "Fixed Go URL.Query / oapi v1.6.0: {} binding/result/error cases (seed 0x311).",
        cases.len()
    );
}

#[tokio::test]
#[ignore = "explicit Go 1.26.8 HTTP error oracle; set SANDPROOF_GUEST_QUERY_REFERENCE"]
async fn fixed_go_query_http_reference() {
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_QUERY_REFERENCE");
    let mut cases = vec![
        b"path=a&path=b".to_vec(),
        b"signature_expiration=%FF%22%0A%3C".to_vec(),
    ];
    let fixed = payload(&failure(b"signature_expiration=x"), 65536)
        .unwrap()
        .len()
        - 1;
    for n in [2047, 2048, 2049, 4096] {
        cases.push(format!("signature_expiration={}", "x".repeat(n - fixed)).into_bytes());
    }
    for raw in &cases {
        for method in ["GET", "HEAD", "POST"] {
            let response = rejected_with_head(
                failure(raw),
                response_limits(),
                deadline(),
                Arc::new(()),
                method == "HEAD",
            )
            .await
            .unwrap();
            let (parts, body) = response.into_parts();
            let header = |key| parts.headers.get(key).map_or("", |v| v.to_str().unwrap());
            let observed = format!(
                "{}\t{}\t{}\t{}\t{}",
                parts.status.as_u16(),
                header("content-type"),
                header("content-length"),
                header("x-content-type-options"),
                hex(&drain(body).await)
            );
            let mode = if method == "POST" { "H" } else { method };
            oracle.check(&format!("{mode}\t{}", hex(raw)), &observed);
        }
    }
    oracle.finish();
    eprintln!(
        "Fixed Go http.Error: {} real loopback status/header/body cases.",
        cases.len() * 3
    );
}
