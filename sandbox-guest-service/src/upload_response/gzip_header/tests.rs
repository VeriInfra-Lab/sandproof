use super::*;
use crate::{
    file_download::tests::reference::{Reference, hex},
    upload_body::{GzipBody, GzipLimits, HttpBodyLimits},
    upload_response::{self, Limits, Rejection},
};
use axum::{
    body::{Body, Bytes},
    http::HeaderValue,
};
use futures_util::stream;
use http_body_util::BodyExt;
use sandproof_guest_runtime::gzip::DecodeLimits;
use std::{fmt::Write, sync::Arc, time::Duration};
use tokio::time::Instant;

fn limits(chunk: usize) -> GzipLimits {
    GzipLimits {
        http: HttpBodyLimits {
            total_bytes: 131_072,
            frame_bytes: 8192,
            frames: 131_072,
            trailer_bytes: 256,
            trailer_fields: 8,
        },
        decode: DecodeLimits {
            chunk_bytes: chunk,
            encoded_bytes: 131_072,
            decoded_bytes: 131_072,
            header_bytes: 70000,
            members: 16,
        },
    }
}
fn body(bytes: &[u8], chunk: usize) -> Body {
    let frames: Vec<_> = bytes
        .chunks(chunk)
        .map(|bytes| Ok::<_, GzipError>(Bytes::copy_from_slice(bytes)))
        .collect();
    Body::from_stream(stream::iter(frames))
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(30)
}

#[tokio::test]
async fn observed_eof_is_distinct_from_transport_length_budget_and_deadline() {
    for (bytes, expected, anticipated) in [
        (b"".as_slice(), Some(0), GzipError::EmptyHeader),
        (b"\x1f".as_slice(), Some(1), GzipError::Truncated),
        (b"".as_slice(), Some(1), GzipError::Transport),
    ] {
        let actual = GzipBody::open(
            body(bytes, 1),
            expected,
            limits(1),
            deadline(),
            Arc::new(()),
        )
        .await
        .err()
        .unwrap();
        assert_eq!(actual, anticipated);
    }
    let broken = Body::from_stream(stream::iter([Err::<Bytes, _>(GzipError::Transport)]));
    assert_eq!(
        GzipBody::open(broken, None, limits(1), deadline(), Arc::new(()))
            .await
            .err(),
        Some(GzipError::Transport)
    );
    let mut budget = limits(1);
    budget.decode.encoded_bytes = 0;
    assert_eq!(
        GzipBody::open(body(b"x", 1), None, budget, deadline(), Arc::new(()))
            .await
            .err(),
        Some(GzipError::InputLimit)
    );
    assert_eq!(
        GzipBody::open(Body::empty(), None, limits(1), Instant::now(), Arc::new(()))
            .await
            .err(),
        Some(GzipError::DeadlineExceeded)
    );
    for error in [
        GzipError::InvalidConfiguration,
        GzipError::InputLimit,
        GzipError::DecodedLimit,
        GzipError::HeaderLimit,
        GzipError::MemberLimit,
        GzipError::Deflate,
        GzipError::Checksum,
        GzipError::Transport,
        GzipError::SourceNotFound,
        GzipError::SourceInvalid,
        GzipError::DeadlineExceeded,
        GzipError::Unavailable,
    ] {
        assert_eq!(GzipHeaderError::try_from(error), Err(error));
    }
}

// Construct header bytes only. The fixed Go reader independently validates
// these CRCs, flags and strings; no body or trailer is decoded in this suite.
fn header(flags: u8, extra: &[u8], name: &[u8], comment: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0x1f, 0x8b, 8, flags, 0, 0, 0, 0, 0, 0xff];
    if flags & 4 != 0 {
        bytes.extend(u16::try_from(extra.len()).unwrap().to_le_bytes());
        bytes.extend(extra);
    }
    if flags & 8 != 0 {
        bytes.extend(name);
        bytes.push(0);
    }
    if flags & 16 != 0 {
        bytes.extend(comment);
        bytes.push(0);
    }
    if flags & 2 != 0 {
        let mut crc = u32::MAX;
        for &byte in &bytes {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xedb8_8320 * (crc & 1));
            }
        }
        bytes.extend(&(!crc).to_le_bytes()[..2]);
    }
    bytes
}
fn cases() -> Vec<Vec<u8>> {
    let mut cases = Vec::new();
    for flags in 0..=255 {
        let bytes = header(flags, b"extra", b"name\xff", b"comment");
        if [0, 2, 4, 8, 16, 30, 255].contains(&flags) {
            for end in 0..bytes.len() {
                cases.push(bytes[..end].to_vec());
            }
        }
        if flags & 2 != 0 {
            let mut corrupt = bytes.clone();
            *corrupt.last_mut().unwrap() ^= 1;
            cases.push(corrupt);
        }
        cases.push(bytes);
    }
    for length in [0, 510, 511, 512, 513] {
        for flags in [8, 16, 26] {
            let bytes = header(flags, b"", &vec![0xff; length], &vec![b'c'; length]);
            cases.push(bytes.clone());
            cases.push(bytes[..bytes.len() - 1].to_vec());
        }
    }
    let maximum = header(4, &vec![7; 65535], b"", b"");
    for end in [10, 11, 12, 13, 500, maximum.len() - 1, maximum.len()] {
        cases.push(maximum[..end].to_vec());
    }
    for index in 0..3 {
        for byte in 0..=255u8 {
            let mut bytes = header(0, b"", b"", b"");
            bytes[index] = byte;
            cases.push(bytes);
        }
    }
    cases
}

#[tokio::test]
#[ignore = "requires explicit fixed Go upload reference and local loopback HTTP"]
async fn first_gzip_header_and_public_errors_match_original_go_reader() {
    let cases = cases();
    let mut reference = Reference::from_env("SANDPROOF_GUEST_UPLOAD_REFERENCE");
    for bytes in &cases {
        let input = format!("G\t{}", hex(bytes));
        for chunk in [1, 3, 17, 1024] {
            let result = GzipBody::open(
                body(bytes, chunk),
                Some(bytes.len() as u64),
                limits(chunk),
                deadline(),
                Arc::new(()),
            )
            .await;
            match result {
                Ok(body) => {
                    drop(body);
                    reference.check(&input, "READY");
                }
                Err(error) => {
                    let failure = GzipHeaderError::try_from(error).unwrap();
                    let response = upload_response::rejected(
                        Rejection::GzipHeader(failure),
                        Limits {
                            wire_bytes: 4096,
                            chunk_bytes: 7,
                        },
                        deadline(),
                        Arc::new(()),
                    )
                    .await
                    .unwrap();
                    reference.check(&input, &wire(response).await);
                }
            }
        }
    }
    reference.finish();
    eprintln!(
        "Fixed Go gzip first header: {} cases x 4 chunk sizes, readiness or exact HTTP errors.",
        cases.len()
    );
}
async fn wire(response: axum::response::Response) -> String {
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
    output
}
