use super::*;
use crate::{
    file_download::tests::reference::{Reference, hex},
    file_encoding, file_upload,
};
use axum::http::{HeaderMap, HeaderName, HeaderValue};
use http_body_util::BodyExt;
use std::{fmt::Write, sync::Arc, time::Duration};

fn limits() -> Limits {
    Limits {
        wire_bytes: 65536,
        chunk_bytes: 37,
    }
}
#[test]
fn rejection_wire_size_is_checked_after_escaping_before_allocation() {
    for error in [
        Rejection::RawPathRequired,
        Rejection::UnsupportedEncoding(b"br<&\xff".to_vec()),
        Rejection::UnsupportedContentType(b"text/\"<\xe2\x80\xa8".to_vec()),
    ] {
        let full = payload(&error, 65536).unwrap();
        assert_eq!(payload(&error, full.len()).unwrap(), full);
        assert_eq!(payload(&error, full.len() - 1), Err(Error::ResponseLimit));
    }
}

#[tokio::test]
async fn rejection_retains_original_owner_until_last_byte_clone() {
    let slots = Arc::new(tokio::sync::Semaphore::new(1));
    let owner = Arc::new(Arc::clone(&slots).try_acquire_owned().unwrap());
    let response = rejected(
        Rejection::RawPathRequired,
        limits(),
        Instant::now() + Duration::from_secs(30),
        owner,
    )
    .await
    .unwrap();
    let mut body = response.into_body();
    let bytes = body.frame().await.unwrap().unwrap().into_data().unwrap();
    let copy = bytes.clone();
    drop(bytes);
    drop(body);
    assert_eq!(slots.available_permits(), 0);
    drop(copy);
    let _slot = tokio::time::timeout(Duration::from_secs(5), slots.acquire_owned())
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
#[ignore = "requires explicit fixed Go upload reference and local loopback HTTP"]
async fn fixed_go_header_rejection_status_headers_and_text_match() {
    let mut reference = Reference::from_env("SANDPROOF_GUEST_UPLOAD_REFERENCE");
    let mut cases = 0;
    compare(&mut reference, "P\tR", Rejection::RawPathRequired).await;
    cases += 1;
    for (mode, name, values) in [
        (
            "E",
            "content-encoding",
            vec![
                b"br".to_vec(),
                b" ".to_vec(),
                b"gzip, identity".to_vec(),
                b"gz\xff".to_vec(),
                b"br; q=\"<&>\"".to_vec(),
                "雪\u{2028}".as_bytes().to_vec(),
            ],
        ),
        (
            "C",
            "content-type",
            vec![
                vec![],
                b"text/plain".to_vec(),
                b"multipart".to_vec(),
                b"a/b/c".to_vec(),
                b"application/octet-stream;a=1;a=2".to_vec(),
                b"\xff".to_vec(),
                b"text/\"<&>".to_vec(),
                "雪\u{2028}".as_bytes().to_vec(),
            ],
        ),
    ] {
        for value in values
            .into_iter()
            .chain((0..=255u8).filter_map(|b| HeaderValue::from_bytes(&[b]).ok().map(|_| vec![b])))
        {
            for duplicate in [false, true] {
                let mut headers = HeaderMap::new();
                headers.append(name, HeaderValue::from_bytes(&value).unwrap());
                if duplicate {
                    headers.append(name, HeaderValue::from_static("ignored"));
                }
                let reject = if mode == "E" {
                    file_encoding::content(
                        &headers,
                        file_encoding::Limits {
                            header_bytes: 65536,
                            items: 64,
                        },
                    ) == Err(file_encoding::Error::Unsupported)
                } else {
                    sandproof_guest_runtime::file_upload::kind(&value, 65536).unwrap()
                        == sandproof_guest_runtime::file_upload::Kind::Unsupported
                };
                if reject {
                    check(&mut reference, mode, &headers).await;
                    cases += 1;
                }
            }
        }
    }
    cases += metadata_cases(&mut reference).await;
    for length in [2047, 2048, 2049, 4096] {
        let overhead = payload(&Rejection::UnsupportedEncoding(vec![]), 65536)
            .unwrap()
            .len();
        let mut headers = HeaderMap::new();
        headers.insert(
            "content-encoding",
            HeaderValue::from_str(&"x".repeat(length - overhead)).unwrap(),
        );
        check(&mut reference, "E", &headers).await;
        cases += 1;
    }
    reference.finish();
    eprintln!("Fixed Go upload header rejections: {cases} status/header/JSON cases.");
}

async fn metadata_cases(reference: &mut Reference) -> usize {
    let mut cases = 0;
    for key in [
        "key".into(),
        "!#$%&'*+-.^_`|~".into(),
        "x".repeat(246),
        "x".repeat(247),
    ] {
        for value in (0..=255u8)
            .filter_map(|b| HeaderValue::from_bytes(&[b]).ok())
            .chain([
                HeaderValue::from_static(""),
                HeaderValue::from_str(&"x".repeat(4096)).unwrap(),
            ])
        {
            let mut headers = HeaderMap::new();
            let name = HeaderName::from_bytes(format!("x-metadata-{key}").as_bytes()).unwrap();
            headers.append(name.clone(), value);
            headers.append(name, HeaderValue::from_static("ignored"));
            headers.insert("x-metadata-", HeaderValue::from_static("\tignored"));
            if file_upload::metadata_detailed(&headers).is_err() {
                check(reference, "M", &headers).await;
                cases += 1;
            }
        }
    }
    for invalid in [false, true] {
        for reversed in [false, true] {
            let mut headers = HeaderMap::new();
            let mut pairs = [
                (
                    "x-metadata-large",
                    HeaderValue::from_str(&"x".repeat(4096)).unwrap(),
                ),
                (
                    "x-metadata-last",
                    HeaderValue::from_static(if invalid { "bad\t" } else { "ok" }),
                ),
            ];
            if reversed {
                pairs.reverse();
            }
            for (name, value) in pairs {
                headers.insert(name, value);
            }
            check(reference, "M", &headers).await;
            cases += 1;
        }
    }
    cases
}
async fn check(reference: &mut Reference, mode: &str, headers: &HeaderMap) {
    let mut input = format!("P\t{mode}");
    for (name, value) in headers {
        write!(
            input,
            "\t{}:{}",
            hex(name.as_str().as_bytes()),
            hex(value.as_bytes())
        )
        .unwrap();
    }
    let first = |name| {
        headers
            .get(name)
            .map_or(&[][..], HeaderValue::as_bytes)
            .to_vec()
    };
    let error = match mode {
        "E" => Rejection::UnsupportedEncoding(first("content-encoding")),
        "C" => Rejection::UnsupportedContentType(first("content-type")),
        "M" => Rejection::Metadata(file_upload::metadata_detailed(headers).unwrap_err()),
        _ => unreachable!(),
    };
    compare(reference, &input, error).await;
}
async fn compare(reference: &mut Reference, input: &str, error: Rejection) {
    let response = rejected(
        error,
        limits(),
        Instant::now() + Duration::from_secs(30),
        Arc::new(()),
    )
    .await
    .unwrap();
    let mut output = response.status().as_u16().to_string();
    for name in ["content-type", "content-length", "x-content-type-options"] {
        write!(
            output,
            "\t{}",
            hex(response
                .headers()
                .get(name)
                .map_or(&[], HeaderValue::as_bytes))
        )
        .unwrap();
    }
    let mut body = response.into_body();
    let mut bytes = Vec::new();
    while let Some(frame) = body.frame().await {
        bytes.extend_from_slice(&frame.unwrap().into_data().unwrap());
    }
    write!(output, "\t{}", hex(&bytes)).unwrap();
    reference.check(input, &output);
}
