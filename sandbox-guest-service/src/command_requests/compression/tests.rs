use super::*;
use crate::command_requests::CompressionStage;
use crate::file_rpc_requests::tests::gzip::compressed;
use sandproof_guest_runtime::protocol::ProtocolError;
use std::time::Duration;

mod reference;

fn frame(flag: u8, bytes: &[u8]) -> Vec<u8> {
    let mut result = vec![flag];
    result.extend(u32::try_from(bytes.len()).unwrap().to_be_bytes());
    result.extend(bytes);
    result
}
fn decode_now(method: &str, bytes: &[u8], limit: usize) -> Result<Request, Error> {
    let (_cancel, cancelled) = watch::channel(false);
    decode(
        method,
        bytes,
        limit,
        7,
        Instant::now() + Duration::from_secs(30),
        &cancelled,
    )
}

#[test]
fn environment_refusal_keeps_original_gzip_json_and_tail_observations() {
    let payload = br#"{"process":{"envs":{"a":"","b":""}}}"#;
    let valid = frame(1, &compressed(payload));
    let mut corrupt = compressed(payload);
    *corrupt.last_mut().unwrap() ^= 1;
    let (_cancel, cancelled) = watch::channel(false);
    let deadline = Instant::now() + Duration::from_secs(30);
    for wire in [
        frame(
            1,
            &compressed(br#"{"process":{"envs":{"a":"","b":"","\u0061":""}}}"#),
        ),
        frame(1, &corrupt),
        [valid.clone(), frame(2, b"!")].concat(),
        [valid.clone(), frame(3, b"bad")].concat(),
        [valid, frame(0, b"{}")].concat(),
    ] {
        let original = decode_selected("Start", &wire, 4096, 7, deadline, &cancelled, true);
        assert!(original.is_err());
        assert_eq!(
            decode_limited(
                "Start",
                &wire,
                4096,
                7,
                deadline,
                &cancelled,
                (
                    true,
                    Some(protocol::StartLimits {
                        arguments: 2,
                        env_entries: 1
                    })
                ),
            ),
            original,
        );
    }
}

#[test]
fn compact_environment_sorting_cannot_reorder_public_json_failures() {
    let (_cancel, cancelled) = watch::channel(false);
    let deadline = Instant::now() + Duration::from_secs(30);
    for payload in [
        // Duplicate before a malformed value or a late syntax failure.
        br#"{"process":{"envs":{"z":"","z":"","a":null}}}"#.as_slice(),
        br#"{"process":{"envs":{"z":"","z":"","a":!}}}"#,
        br#"{"process":{"envs":{"z":"","z":""},"cwd":[]}}"#,
        // Wire order wins over sorted key order, including escaped spellings.
        br#"{"process":{"envs":{"z":"","a":"","\u007a":"","a":""}}}"#,
        br#"{"process":{"envs":{"z":"","a":null,"z":""}}}"#,
        br#"{"process":{"envs":{"\ud83d\ude00":"","a":"","\ud83d\ude00":null}}}"#,
        br#"{"process":{"envs":{"a123456789012345":"","\u0061123456789012345":"","b":false}}}"#,
        br#"{"process":{"envs":{"a":"","b":""},"envs":null}}"#,
    ] {
        for gzip in [false, true] {
            let wire = if gzip {
                frame(1, &compressed(payload))
            } else {
                frame(0, payload)
            };
            let original = decode_selected("Start", &wire, 4096, 7, deadline, &cancelled, gzip);
            assert!(matches!(original, Err(Error::Json(_))));
            assert_eq!(
                decode_limited(
                    "Start",
                    &wire,
                    4096,
                    7,
                    deadline,
                    &cancelled,
                    (
                        gzip,
                        Some(protocol::StartLimits {
                            arguments: 2,
                            env_entries: 1,
                        }),
                    ),
                ),
                original,
            );
        }
    }
}

#[test]
fn compressed_command_data_and_eos_keep_empty_wire_and_first_failure_semantics() {
    for method in ["Start", "Connect"] {
        let expected = protocol::decode_request(method, &frame(0, b"{}"), 1024).unwrap();
        for wire in [frame(1, b""), frame(1, &compressed(b"{}")), frame(0, b"{}")] {
            assert_eq!(decode_now(method, &wire, 1024).unwrap(), expected);
            let with_end = [
                wire,
                frame(3, &compressed(br#"{"error":{"code":"not_found"}}"#)),
            ]
            .concat();
            assert_eq!(decode_now(method, &with_end, 1024).unwrap(), expected);
        }
        assert!(matches!(
            decode_now(method, &frame(1, &compressed(b"")), 1024),
            Err(Error::Json(_))
        ));
        assert!(matches!(
            decode_now(method, &[frame(0, b"bad"), frame(3, b"bad")].concat(), 1024),
            Err(Error::Json(_))
        ));
        // A compressed special frame is expanded before checking trailing data.
        assert!(matches!(
            decode_now(
                method,
                &[frame(0, b"{}"), frame(3, b"bad"), vec![0]].concat(),
                1024
            ),
            Err(Error::Compression {
                stage: CompressionStage::Header,
                ..
            })
        ));
        assert_eq!(
            decode_now(
                method,
                &[frame(0, b"{}"), frame(3, &compressed(b"{}")), vec![0]].concat(),
                1024
            ),
            Err(Error::Framing(protocol::FramingError::ExtraBytes(1)))
        );
    }
    for wire in [b"".to_vec(), compressed(b"")] {
        assert!(matches!(
            decode_now("List", &wire, 1024),
            Err(Error::Json(_))
        ));
    }
    assert_eq!(
        decode_now("List", &[compressed(b"{"), compressed(b"}")].concat(), 1024).unwrap(),
        Request::List
    );
}

#[test]
fn command_gzip_distinguishes_header_body_and_independent_wire_and_expanded_limits() {
    assert!(matches!(
        decode_now("List", b"not gzip", 1024),
        Err(Error::Compression {
            stage: CompressionStage::Header,
            ..
        })
    ));
    let mut broken = compressed(b"{}");
    let checksum = broken.len() - 8;
    broken[checksum] ^= 1;
    assert!(matches!(
        decode_now("List", &broken, 1024),
        Err(Error::Compression {
            stage: CompressionStage::Body,
            error: GzipError::Checksum
        })
    ));
    let large = compressed(format!("{{\"unknown\":\"{}\"}}", "a".repeat(1024)).as_bytes());
    assert!(large.len() < 128);
    assert_eq!(decode_now("List", &large, 128), Err(Error::BodyLimit));
    assert_eq!(
        decode_now("List", &compressed(b"{}"), 2),
        Err(Error::Protocol(ProtocolError::ResourceExhausted))
    );
    assert_eq!(
        decode_now("Start", &frame(1, &large), 128),
        Err(Error::BodyLimit)
    );
    assert_eq!(
        decode_now("Start", &frame(1, &compressed(b"{}")), 2),
        Err(Error::Protocol(ProtocolError::ResourceExhausted))
    );
}

#[test]
fn shared_expander_checks_original_cancellation_and_deadline_before_even_empty_input() {
    for payload in [
        vec![],
        compressed(b"{}"),
        [compressed(b""), compressed(b"{}")].concat(),
    ] {
        let (cancel, cancelled) = watch::channel(false);
        let now = Instant::now();
        assert_eq!(
            request_gzip::expand(&payload, 1024, 7, now, &cancelled)
                .unwrap_err()
                .error,
            GzipError::DeadlineExceeded
        );
        cancel.send(true).unwrap();
        assert_eq!(
            request_gzip::expand(&payload, 1024, 7, now + Duration::from_secs(30), &cancelled)
                .unwrap_err()
                .error,
            GzipError::Unavailable
        );
    }
}
