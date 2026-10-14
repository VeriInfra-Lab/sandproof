use super::*;
use serde_json::{Value, json};

fn zero() -> Entry {
    Entry {
        name: String::new(),
        kind: FileType::Unspecified,
        path: String::new(),
        size: 0,
        mode: 0,
        permissions: String::new(),
        owner: String::new(),
        group: String::new(),
        modified_time: None,
        symlink_target: None,
        metadata: BTreeMap::new(),
    }
}
fn observation(entry: &Entry, time: Option<(i64, u32)>) -> Value {
    // This is an independently populated test record, not our encoded response
    // fed back into Go. Go constructs the original generated EntryInfo directly.
    json!({
        "Name":entry.name, "Type":match entry.kind {FileType::Unspecified=>0,FileType::File=>1,FileType::Directory=>2,FileType::Symlink=>3},
        "Path":entry.path, "Size":entry.size, "Mode":entry.mode,
        "Permissions":entry.permissions, "Owner":entry.owner,"Group":entry.group,
        "Time":time.map(|(seconds,nanos)|json!({"Seconds":seconds,"Nanos":nanos})),
        "Link":entry.symlink_target,"Metadata":entry.metadata
    })
}
fn cases() -> Vec<(Entry, Value)> {
    let mut cases = Vec::new();
    let mut add = |entry: Entry, time| {
        let observed = observation(&entry, time);
        cases.push((entry, observed));
    };
    add(zero(), None);
    for kind in [
        FileType::Unspecified,
        FileType::File,
        FileType::Directory,
        FileType::Symlink,
    ] {
        add(Entry { kind, ..zero() }, None);
    }
    for size in [0, 1, -1, i64::MIN, i64::MAX] {
        add(
            Entry {
                size,
                mode: if size == 0 { 0 } else { u32::MAX },
                ..zero()
            },
            None,
        );
    }
    for link in [None, Some(String::new()), Some("../target/雪".into())] {
        add(
            Entry {
                symlink_target: link,
                ..zero()
            },
            None,
        );
    }
    for value in [
        String::new(),
        "\0\u{1}\u{b}\u{1f}\t\n\r\u{8}\u{c}\"\\".into(),
        "<>&/雪\u{2028}\u{2029}😀".into(),
        "a".repeat(500),
    ] {
        let mut metadata = BTreeMap::new();
        metadata.insert(String::new(), String::new());
        metadata.insert("z".into(), value.clone());
        metadata.insert(value.clone(), "first".into());
        add(
            Entry {
                name: value.clone(),
                path: value.clone(),
                permissions: value.clone(),
                owner: value.clone(),
                group: value,
                metadata,
                ..zero()
            },
            None,
        );
    }
    for (seconds, nanos) in [
        (0, 0),
        (-1, 1),
        (0, 100_000_000),
        (0, 123_400_000),
        (-62_135_596_800, 0),
        (253_402_300_799, 999_999_999),
    ] {
        add(
            Entry {
                modified_time: Some(ModifiedTime::new(seconds, nanos).unwrap()),
                ..zero()
            },
            Some((seconds, nanos)),
        );
    }
    cases
}

#[tokio::test]
#[ignore = "explicit original generated protobuf + Connect property HTTP reference; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
async fn original_generated_properties_match_compact_json_and_http_headers() {
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    let mut responses = vec![
        (Records::Empty, json!({"Shape":"Empty","Entries":[]})),
        (
            Records::Many(vec![]),
            json!({"Shape":"Entries","Entries":[]}),
        ),
    ];
    for (entry, observed) in cases() {
        responses.push((
            Records::One(entry.clone()),
            json!({"Shape":"Entry","Entries":[observed.clone()]}),
        ));
        responses.push((
            Records::Many(vec![zero(), entry]),
            json!({"Shape":"Entries","Entries":[observation(&zero(),None),observed]}),
        ));
    }
    let count = responses.len();
    for (records, observed) in responses {
        let (retained, slots) = retain(records);
        let response = send(retained, view, limits(), deadline()).await.unwrap();
        let mut output = vec![response.status().as_u16().to_string()];
        for name in [
            "content-type",
            "content-length",
            "accept-encoding",
            "content-encoding",
            "x-content-type-options",
            "vary",
        ] {
            let bytes = response
                .headers()
                .get(name)
                .map_or(b"".as_slice(), |v| v.as_bytes());
            output.push(hex(if name == "content-length" && !bytes.is_empty() {
                b"valid"
            } else {
                bytes
            }));
        }
        let length = response.headers().get("content-length").cloned();
        let body = drain(response.into_body()).await;
        if let Some(length) = length {
            assert_eq!(length, body.len().to_string());
        }
        output.push(hex(&body));
        oracle.check(
            &format!("P\t{}", hex(observed.to_string().as_bytes())),
            &output.join("\t"),
        );
        released(slots).await;
    }
    oracle.finish();
    eprintln!(
        "{count} original generated protobuf/default Connect HTTP property cases; compact JSON, synthetic attributes."
    );
}
