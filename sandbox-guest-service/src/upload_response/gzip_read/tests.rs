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
use sandproof_guest_runtime::gzip::{DecodeLimits, EncodeLimits, Encoder};
use std::{fmt::Write, sync::Arc, time::Duration};
use tokio::time::Instant;

fn limits(chunk: usize) -> GzipLimits {
    GzipLimits {
        http: HttpBodyLimits {
            total_bytes: 65536,
            frame_bytes: 4096,
            frames: 65536,
            trailer_bytes: 256,
            trailer_fields: 8,
        },
        decode: DecodeLimits {
            chunk_bytes: chunk,
            encoded_bytes: 65536,
            decoded_bytes: 65536,
            header_bytes: 1024,
            members: 16,
        },
    }
}
fn encode(data: &[u8]) -> Vec<u8> {
    let mut encoder = Encoder::new(
        data,
        EncodeLimits {
            chunk_bytes: 37,
            decoded_bytes: 65536,
            encoded_bytes: 65536,
        },
    )
    .unwrap();
    let mut bytes = Vec::new();
    while let Some(chunk) = encoder.next_chunk().unwrap() {
        bytes.extend(chunk);
    }
    bytes
}
fn cases() -> Vec<Vec<u8>> {
    let mut cases = Vec::new();
    for source in [
        vec![],
        b"binary\0\xfftext".to_vec(),
        (0..=255u8).collect(),
        b"body".repeat(257),
    ] {
        let bytes = encode(&source);
        cases.push(bytes.clone());
        for length in 10..bytes.len() {
            cases.push(bytes[..length].to_vec());
        }
        for offset in bytes.len() - 8..bytes.len() {
            for bit in 0..8 {
                let mut corrupt = bytes.clone();
                corrupt[offset] ^= 1 << bit;
                cases.push(corrupt);
            }
        }
        for tail in [
            b"bad next member header".to_vec(),
            b"\x1f".to_vec(),
            vec![0; 10],
        ] {
            cases.push([bytes.clone(), tail].concat());
        }
        let next = encode(b"next member");
        for length in 0..=next.len() {
            cases.push([bytes.clone(), next[..length].to_vec()].concat());
        }
    }
    cases
}

#[test]
fn only_observed_body_diagnostics_map_and_legacy_conversion_preserves_the_error() {
    for error in [GzipError::Truncated, GzipError::Header, GzipError::Checksum] {
        assert_eq!(
            GzipError::from(GzipReadError::try_from(error).unwrap()),
            error
        );
    }
    for error in [
        GzipError::EmptyHeader,
        GzipError::Deflate,
        GzipError::Transport,
        GzipError::InvalidConfiguration,
        GzipError::InputLimit,
        GzipError::DecodedLimit,
        GzipError::HeaderLimit,
        GzipError::MemberLimit,
        GzipError::SourceNotFound,
        GzipError::SourceInvalid,
        GzipError::DeadlineExceeded,
        GzipError::Unavailable,
    ] {
        assert_eq!(GzipReadError::try_from(error), Err(error));
    }
}

#[tokio::test]
#[ignore = "requires explicit fixed Go upload reference and local loopback HTTP"]
async fn raw_gzip_decoded_prefix_and_read_failure_response_match_original_go() {
    let cases = cases();
    let mut reference = Reference::from_env("SANDPROOF_GUEST_UPLOAD_REFERENCE");
    for bytes in &cases {
        for chunk in [1, 7, 128] {
            let frames: Vec<_> = bytes
                .chunks(chunk)
                .map(|v| Ok::<_, std::io::Error>(Bytes::copy_from_slice(v)))
                .collect();
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut body = GzipBody::open(
                Body::from_stream(stream::iter(frames)),
                Some(bytes.len() as u64),
                limits(chunk),
                deadline,
                Arc::new(()),
            )
            .await
            .unwrap();
            let mut prefix = Vec::new();
            let error = loop {
                match body.next().await {
                    Ok(Some(bytes)) => prefix.extend_from_slice(bytes.as_ref()),
                    Ok(None) => break None,
                    Err(error) => break Some(GzipReadError::try_from(error).unwrap()),
                }
            };
            drop(body);
            let suffix = if let Some(error) = error {
                let response = upload_response::rejected(
                    Rejection::RawGzipBody(error),
                    Limits {
                        wire_bytes: 4096,
                        chunk_bytes: 7,
                    },
                    deadline,
                    Arc::new(()),
                )
                .await
                .unwrap();
                wire(response).await
            } else {
                "END".into()
            };
            reference.check(
                &format!("D\t{}", hex(bytes)),
                &format!("{}\t{suffix}", hex(&prefix)),
            );
        }
    }
    reference.finish();
    eprintln!(
        "Fixed Go raw gzip body: {} cases x 3 chunk sizes, exact decoded prefix and terminal response.",
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
