use super::*;
use crate::file_download::tests::reference::{Reference, hex};

fn unhex(value: &str) -> Vec<u8> {
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
fn check(oracle: &mut Reference, method: &str, wire: &[u8], gzip: bool) {
    let input = serde_json::json!({"Method":method,"Body":hex(wire),"Gzip":gzip});
    let record = format!("E\t{}", hex(&serde_json::to_vec(&input).unwrap()));
    let original = oracle.observe(&record);
    let result = if gzip {
        decode_now(method, wire, 65536)
    } else {
        protocol::decode_request(method, wire, 65536).map_err(Error::from)
    };
    if let Some(json) = original.strip_prefix("OK\t") {
        let json = unhex(json);
        let identity = if matches!(method, "Start" | "Connect") {
            frame(0, &json)
        } else {
            json
        };
        assert_eq!(
            result,
            protocol::decode_request(method, &identity, 65536).map_err(Error::from),
            "{record}"
        );
    } else {
        assert!(original.starts_with("ERR\t"), "{original}");
        assert!(result.is_err(), "accepted {record}; original {original}");
    }
}

#[test]
#[ignore = "explicit fixed Go command gzip oracle; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
fn fixed_go_command_gzip_matches_decoded_requests_and_rejections() {
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    let member = compressed(b"{}");
    let mut payloads = vec![
        vec![],
        b"{}".to_vec(),
        member.clone(),
        compressed(b""),
        [compressed(b"{"), compressed(b"}")].concat(),
        [compressed(b""), member.clone()].concat(),
        [member.clone(), b"trailing".to_vec()].concat(),
    ];
    payloads.extend((1..member.len()).map(|n| member[..n].to_vec()));
    for index in [0, 2, 3, member.len() - 8, member.len() - 4] {
        let mut broken = member.clone();
        broken[index] ^= 255;
        payloads.push(broken);
    }
    payloads.extend([
        compressed(b"null"),
        compressed(b"[]"),
        compressed(b"{}{}"),
        compressed(br#"{"process":{"cmd":"echo","args":["x"]}}"#),
        compressed(br#"{"process":{"pid":42}}"#),
    ]);
    let mut count = 0;
    for method in [
        "Start",
        "Connect",
        "List",
        "SendInput",
        "CloseStdin",
        "SendSignal",
        "Update",
    ] {
        let stream = matches!(method, "Start" | "Connect");
        for payload in &payloads {
            for gzip in [false, true] {
                if stream {
                    for flag in [0, 1] {
                        check(&mut oracle, method, &frame(flag, payload), gzip);
                        count += 1;
                    }
                } else {
                    check(&mut oracle, method, payload, gzip);
                    count += 1;
                }
            }
        }
        if !stream {
            continue;
        }
        for gzip in [false, true] {
            for flag in 0..=255 {
                check(&mut oracle, method, &frame(flag, &member), gzip);
                check(
                    &mut oracle,
                    method,
                    &[frame(0, b"{}"), frame(flag, &member)].concat(),
                    gzip,
                );
                count += 2;
            }
            for first in [
                frame(0, b"{}"),
                frame(1, &member),
                frame(1, b""),
                frame(1, &compressed(b"")),
            ] {
                for end in [
                    b"{}".as_slice(),
                    br#"{"metadata":{"key":["value"]},"error":{"message":"\ud800"}}"#,
                    b"{\"metadata\":{\"key\":[\"\xff\"]}}",
                    b"null",
                    b"[]",
                    b"",
                ] {
                    for compressed_end in [false, true] {
                        let tail = if compressed_end {
                            frame(3, &compressed(end))
                        } else {
                            frame(2, end)
                        };
                        let wire = [first.clone(), tail].concat();
                        check(&mut oracle, method, &wire, gzip);
                        check(&mut oracle, method, &[wire, vec![0]].concat(), gzip);
                        count += 2;
                    }
                }
            }
        }
    }
    oracle.finish();
    eprintln!(
        "{count} fixed Go command identity/gzip request acceptance and decoded-value cases; public failure text is separate."
    );
}
