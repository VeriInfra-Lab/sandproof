use super::*;
use crate::{
    file_download::tests::reference::{Reference, hex},
    upload_response::{Limits, Rejection, rejected},
};
use axum::http::HeaderValue;
use http_body_util::BodyExt;
use sandproof_guest_runtime::multipart::{Decoder, Event, Limits as WireLimits, ReadError};
use std::{fmt::Write, sync::Arc, time::Duration};
use tokio::time::Instant;

fn limits(chunk: usize) -> WireLimits {
    WireLimits {
        chunk_bytes: chunk,
        boundary_bytes: 64,
        header_bytes: 4096,
        header_count: 32,
        line_bytes: 4096,
        parts: 32,
        total_bytes: 65536,
    }
}
fn header(name: &str, filename: &str, nl: &str) -> Vec<u8> {
    format!(
        "--B{nl}Content-Disposition: form-data; name=\"{name}\"; filename=\"/{filename}\"{nl}{nl}"
    )
    .into_bytes()
}
fn cases() -> Vec<Vec<u8>> {
    let mut cases = vec![
        vec![],
        b"preamble".to_vec(),
        b"preamble\r\n".to_vec(),
        b"--B".to_vec(),
        b"--B--".to_vec(),
    ];
    for nl in ["\r\n", "\n"] {
        for name in ["file", "ignored"] {
            let head = header(name, "first", nl);
            let full = [
                head.as_slice(),
                b"data\0\xff",
                nl.as_bytes(),
                b"--B--",
                nl.as_bytes(),
            ]
            .concat();
            for length in head.len()..=full.len() {
                cases.push(full[..length].to_vec());
            }
            for prefix in [
                b"".as_slice(),
                [b"data".as_slice(), nl.as_bytes()].concat().as_slice(),
            ] {
                for byte in (0..=255u8).filter(|b| *b != b'\n') {
                    cases.push(
                        [
                            head.as_slice(),
                            prefix,
                            b"--B ",
                            &[byte],
                            b"!",
                            nl.as_bytes(),
                        ]
                        .concat(),
                    );
                }
                for suffix in ["雪\u{2028}\u{ad}\u{10000}\u{10ffff}", "\"\\<&>", "\tbad"] {
                    cases.push(
                        [
                            head.as_slice(),
                            prefix,
                            b"--B--",
                            suffix.as_bytes(),
                            nl.as_bytes(),
                        ]
                        .concat(),
                    );
                }
            }
        }
        let first = [
            header("file", "first", nl),
            b"done".to_vec(),
            nl.as_bytes().to_vec(),
        ]
        .concat();
        for name in ["file", "ignored"] {
            cases.push(
                [
                    first.clone(),
                    header(name, "second", nl),
                    b"unfinished".to_vec(),
                ]
                .concat(),
            );
        }
        cases.push([first.clone(), b"--B \"".repeat(400), nl.as_bytes().to_vec()].concat());
        cases.push([first, b"--B--".to_vec()].concat());
    }
    cases
}

// Portable decoder consumer records only file-reader observations. Linux tests
// separately exercise the actual batch, metadata and original admission.
fn observe(input: &[u8], chunk: usize) -> (String, Option<FramingError>) {
    let mut decoder = Decoder::new("B", limits(chunk)).unwrap();
    let mut source = input.chunks(chunk);
    let mut writes: Vec<(Vec<u8>, bool)> = Vec::new();
    let mut file = false;
    let mut eof = false;
    let error = loop {
        match decoder.next_event_detailed() {
            Ok(Some(Event::Part(part))) => {
                file = part.name.as_deref() == Some("file");
                assert!(!part.quoted_printable);
                if file {
                    writes.push((Vec::new(), false));
                }
            }
            Ok(Some(Event::Data(bytes))) => {
                if file {
                    writes.last_mut().unwrap().0.extend(bytes);
                }
            }
            Ok(Some(Event::PartEnd)) => {
                if file {
                    writes.last_mut().unwrap().1 = true;
                }
                file = false;
            }
            Ok(Some(Event::End)) => break None,
            Ok(None) => {
                assert!(!eof, "EOF without End");
                if let Some(bytes) = source.next() {
                    decoder.feed(bytes).unwrap();
                } else {
                    eof = true;
                    decoder.end_input();
                }
            }
            Err(ReadError::Framing(error)) => {
                break Some(match error {
                    FramingError::BodyTruncated if !file => FramingError::NextPartEof,
                    error => error,
                });
            }
            Err(error) => panic!("unmapped {error:?} for {}", hex(input)),
        }
    };
    let mut effects = String::new();
    for (bytes, complete) in writes {
        write!(effects, "W:{}:{complete};", hex(&bytes)).unwrap();
    }
    (effects, error)
}

#[test]
fn error_budget_includes_go_quote_then_json_escaping() {
    for error in [
        FramingError::BodyTruncated,
        FramingError::NextPartEof,
        FramingError::Boundary {
            line: b"--B \"\xff\x07\r\n".to_vec(),
            after_separator: true,
        },
        FramingError::Boundary {
            line: "雪\u{2028}\u{10ffff}".as_bytes().to_vec(),
            after_separator: false,
        },
    ] {
        let bytes = payload(&error, 65536).unwrap();
        assert_eq!(payload(&error, bytes.len()).unwrap(), bytes);
        assert_eq!(payload(&error, bytes.len() - 1), Err(Error::ResponseLimit));
    }
}

#[test]
#[ignore = "requires explicit fixed Go upload reference"]
fn go_quote_matches_every_unicode_scalar_and_invalid_bytes() {
    let mut reference = Reference::from_env("SANDPROOF_GUEST_UPLOAD_REFERENCE");
    let mut input: Vec<_> = (0..=255u8).collect();
    let mut count = 0;
    for code in 0..=0x0010_ffff {
        if let Some(c) = char::from_u32(code) {
            input.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
        }
        if code % 512 == 511 {
            let mut output = Vec::new();
            quote::quoted(&input, &mut |bytes| {
                output.extend_from_slice(bytes);
                Ok(())
            })
            .unwrap();
            reference.check(&format!("Q\t{}", hex(&input)), &hex(&output));
            input.clear();
            count += 1;
        }
    }
    assert!(input.is_empty());
    reference.finish();
    eprintln!("Fixed Go Quote: all Unicode scalars and all byte values in {count} batches.");
}

#[tokio::test]
#[ignore = "requires explicit fixed Go upload reference and local loopback HTTP"]
async fn framing_stage_file_prefix_completion_and_http_match_original_go() {
    let cases = cases();
    let mut reference = Reference::from_env("SANDPROOF_GUEST_UPLOAD_REFERENCE");
    for input in &cases {
        for chunk in [1, 7, 128] {
            let (effects, error) = observe(input, chunk);
            let suffix = if let Some(error) = error {
                let response = rejected(
                    Rejection::MultipartRead(error),
                    Limits {
                        wire_bytes: 65536,
                        chunk_bytes: 37,
                    },
                    Instant::now() + Duration::from_secs(30),
                    Arc::new(()),
                )
                .await
                .unwrap();
                let mut wire = response.status().as_u16().to_string();
                for name in ["content-type", "content-length", "x-content-type-options"] {
                    write!(
                        wire,
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
                write!(wire, "\t{}", hex(&bytes)).unwrap();
                wire
            } else {
                "END".into()
            };
            reference.check(
                &format!("N\t{}", hex(input)),
                &format!("{effects}\t{suffix}"),
            );
        }
    }
    reference.finish();
    eprintln!(
        "Fixed Go multipart framing: {} cases x 3 chunks, exact file prefixes/completion and terminal HTTP.",
        cases.len()
    );
}
