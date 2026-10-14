use sandproof_guest_runtime::file_protocol::{
    Entry, FileType, ModifiedTime, Request, Response, decode_request, encode_response,
};
use sandproof_guest_runtime::protocol::ProtocolError;
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[test]
fn response_allocation_stays_within_wire_budget_including_small_and_escaped_payloads() {
    assert_eq!(encode_response(Response::Empty, 2).unwrap().capacity(), 2);
    for length in [0, 1, 7, 8, 9, 31, 32, 33, 1024] {
        let mut entry = entry();
        entry.name = "\"\\\n".repeat(length);
        let expected = encode_response(Response::Entry(&entry), 65536).unwrap();
        for limit in [expected.len(), expected.len() + 1, expected.len() + 256] {
            let encoded = encode_response(Response::Entry(&entry), limit).unwrap();
            assert!(encoded.capacity() <= limit);
            assert_eq!(encoded, expected);
        }
        assert_eq!(
            encode_response(Response::Entry(&entry), expected.len() - 1),
            Err(ProtocolError::ResourceExhausted)
        );
    }
}

fn entry() -> Entry {
    Entry {
        name: "sample.bin".into(),
        kind: FileType::File,
        path: "/tmp/sample.bin".into(),
        size: 4,
        mode: 0o644,
        permissions: "-rw-r--r--".into(),
        owner: "ubuntu".into(),
        group: "ubuntu".into(),
        modified_time: Some(ModifiedTime::new(1_790_812_800, 0).unwrap()),
        symlink_target: None,
        metadata: BTreeMap::new(),
    }
}

#[test]
fn fixed_shared_file_requests_and_stat_response_are_consumed() {
    let fixtures: Value = serde_json::from_str(include_str!(
        "../../tests/compat/guest-service-contract.json"
    ))
    .unwrap();
    let expected = [
        Request::Stat("/tmp/sample.bin".into()),
        Request::MakeDir("~/new/nested".into()),
        Request::ListDir {
            path: "/tmp".into(),
            depth: 2,
        },
        Request::Move {
            source: "/tmp/sample.bin".into(),
            destination: "/tmp/moved.bin".into(),
        },
        Request::Remove("/tmp/moved.bin".into()),
    ];
    let files: Vec<_> = fixtures["rpcs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v["service"] == "filesystem")
        .collect();
    assert_eq!(files.len(), expected.len());
    for (case, expected) in files.iter().zip(expected) {
        let bytes = serde_json::to_vec(&case["request"]).unwrap();
        assert_eq!(
            decode_request(case["method"].as_str().unwrap(), &bytes, bytes.len()),
            Ok(expected)
        );
    }
    let encoded = encode_response(Response::Entry(&entry()), 2048).unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&encoded).unwrap(),
        files[0]["responses"][0]
    );
}

#[test]
fn missing_null_and_zero_do_not_lose_defaulting_intent() {
    for body in [b"{}".as_slice(), br#"{"path":null,"depth":null}"#] {
        assert_eq!(
            decode_request("ListDir", body, 1024),
            Ok(Request::ListDir {
                path: String::new(),
                depth: 0
            })
        );
    }
    assert_eq!(
        decode_request("Move", br#"{"source":null}"#, 1024),
        Ok(Request::Move {
            source: String::new(),
            destination: String::new()
        })
    );
    for value in [json!(u32::MAX), json!(u32::MAX.to_string())] {
        let bytes = serde_json::to_vec(&json!({"path":"..","depth":value})).unwrap();
        assert_eq!(
            decode_request("ListDir", &bytes, 1024),
            Ok(Request::ListDir {
                path: "..".into(),
                depth: u32::MAX
            })
        );
    }
    assert_eq!(
        decode_request("Stat", br#"{"path":"a\u0000b"}"#, 1024),
        Ok(Request::Stat("a\0b".into()))
    );
}

#[test]
fn duplicate_array_unknown_and_stream_framed_messages_are_not_unary_requests() {
    for body in [
        br#"{"path":"a","path":"b"}"#.as_slice(),
        b"[]",
        b"null",
        br#"{"path":1}"#,
        br#"{"unknown":true}"#,
        b"{}{}",
        b"\0\0\0\0\x02{}",
    ] {
        assert_eq!(
            decode_request("Stat", body, 1024),
            Err(ProtocolError::InvalidArgument)
        );
    }
    for depth in ["-1", "4294967296", "1.5", "[]", "true", "\"no\""] {
        let body = format!("{{\"depth\":{depth}}}");
        assert_eq!(
            decode_request("ListDir", body.as_bytes(), 1024),
            Err(ProtocolError::InvalidArgument)
        );
    }
    for method in [
        "WatchDir",
        "CreateWatcher",
        "RemoveWatcher",
        "GetWatcherEvents",
        "Copy",
        "",
    ] {
        assert_eq!(
            decode_request(method, b"{}", 1024),
            Err(ProtocolError::Unimplemented)
        );
    }
}

#[test]
fn byte_budgets_reject_before_decoding_and_never_emit_a_partial_response() {
    let body = br#"{"path":"x"}"#;
    assert_eq!(
        decode_request("Stat", body, body.len() - 1),
        Err(ProtocolError::ResourceExhausted)
    );
    assert_eq!(
        decode_request("Stat", b"", 0),
        Err(ProtocolError::ResourceExhausted)
    );
    let mut entry = entry();
    entry.metadata.insert("escaped".into(), "\0\"\\\n雪".into());
    let encoded = encode_response(Response::Entry(&entry), 4096).unwrap();
    assert_eq!(
        encode_response(Response::Entry(&entry), encoded.len()).unwrap(),
        encoded
    );
    assert_eq!(
        encode_response(Response::Entry(&entry), encoded.len() - 1),
        Err(ProtocolError::ResourceExhausted)
    );
    assert_eq!(encode_response(Response::Empty, 2).unwrap(), b"{}");
    assert_eq!(
        encode_response(Response::Empty, 1),
        Err(ProtocolError::ResourceExhausted)
    );
    assert_eq!(
        encode_response(Response::Entries(&[]), 14).unwrap(),
        b"{\"entries\":[]}"
    );
}

#[test]
fn int64_sizes_and_optional_symlink_presence_survive_encoding() {
    let mut entry = entry();
    entry.size = i64::MAX;
    entry.symlink_target = Some(String::new());
    entry.modified_time = None;
    entry.kind = FileType::Unspecified;
    let bytes = encode_response(Response::Entry(&entry), 4096).unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["entry"]["size"], i64::MAX.to_string());
    assert_eq!(value["entry"]["symlinkTarget"], "");
    assert_eq!(value["entry"]["type"], "FILE_TYPE_UNSPECIFIED");
    assert!(value["entry"].get("modifiedTime").is_none());
    assert!(value["entry"].get("metadata").is_none());
    entry.size = i64::MIN;
    let bytes = encode_response(Response::Entry(&entry), 4096).unwrap();
    assert!(
        String::from_utf8(bytes)
            .unwrap()
            .contains("\"-9223372036854775808\"")
    );
}

#[test]
fn timestamp_canonical_fraction_and_protobuf_boundaries_are_preserved() {
    for (seconds, nanos, want) in [
        (0, 0, "1970-01-01T00:00:00Z"),
        (-1, 1, "1969-12-31T23:59:59.000000001Z"),
        (0, 100_000_000, "1970-01-01T00:00:00.100Z"),
        (0, 123_400_000, "1970-01-01T00:00:00.123400Z"),
        (-62_135_596_800, 0, "0001-01-01T00:00:00Z"),
        (
            253_402_300_799,
            999_999_999,
            "9999-12-31T23:59:59.999999999Z",
        ),
    ] {
        assert_eq!(
            serde_json::to_value(ModifiedTime::new(seconds, nanos).unwrap()).unwrap(),
            want
        );
    }
    for (seconds, nanos) in [
        (0, 1_000_000_000),
        (-62_135_596_801, 0),
        (253_402_300_800, 0),
        (i64::MAX, 0),
        (i64::MIN, 0),
    ] {
        assert_eq!(
            ModifiedTime::new(seconds, nanos),
            Err(ProtocolError::InvalidArgument)
        );
    }
}
