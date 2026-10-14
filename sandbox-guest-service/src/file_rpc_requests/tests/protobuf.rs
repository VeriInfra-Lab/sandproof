use super::*;
use serde_json::json;

fn messages() -> Vec<Vec<u8>> {
    let mut cases = [
        "",
        " ",
        "{}",
        " {}\n",
        "[]",
        "null",
        "true",
        "1",
        "{}{}",
        "{",
        "{,}",
        r#"{"path":null,"source":null,"destination":null,"depth":null}"#,
        r#"{"path":null,"path":"again"}"#,
        r#"{"path":"first","pa\u0074h":null}"#,
        r#"{"depth":null,"depth":null}"#,
        r#"{"source":null,"source":null}"#,
        r#"{"destination":null,"destination":null}"#,
        r#"{"future":null,"future":{"path":1,"path":false},"huge":1e9999999999999999}"#,
        r#"{"future":[true,null,{},[],"value"],"[extension]":null}"#,
        r#"{"future":[1,],"path":"bad"}"#,
        r#"{"Path":99,"Depth":[],"Source":true,"Destination":{}}"#,
    ]
    .into_iter()
    .map(|s| s.as_bytes().to_vec())
    .collect::<Vec<_>>();
    for key in ["path", "source", "destination", "depth", "future"] {
        for value in [
            "null",
            "true",
            "false",
            "0",
            "[]",
            "{}",
            r#""""#,
            r#""../snow/雪\u0000""#,
            r#""\ud83d\ude00""#,
            r#""\ud800""#,
            r#""\udc00""#,
            r#""\ud800\u0041""#,
            r#""\ud800\ud800""#,
            r#""\udfff\udfff""#,
            r#""\x41""#,
        ] {
            cases.push(format!(r#"{{"{key}":{value}}}"#).into_bytes());
        }
    }
    for nesting in [127, 128, 9998, 9999, 10_000] {
        for (open, close) in [("[", "]"), (r#"{"x":"#, "}")] {
            cases.push(
                format!(
                    r#"{{"future":{}0{}}}"#,
                    open.repeat(nesting),
                    close.repeat(nesting)
                )
                .into_bytes(),
            );
        }
    }
    cases
}

fn numbers() -> Vec<String> {
    let mut values = vec![
        "null".into(),
        "true".into(),
        "[]".into(),
        "{}".into(),
        r#""\u0031e2""#.into(),
    ];
    for number in [
        "0",
        "-0",
        "1",
        "-1",
        "4294967295",
        "4294967296",
        "01",
        "+1",
        " 1",
        "1 ",
        "1\n",
        "\u{a0}1",
        "1.0",
        "1.5",
        "1.00000000000000000000001",
        "4294967295.0000000001",
        "42949672950000000000000000000e-19",
        "0.00000000000000000001e20",
        "0.000000000000000000001e21",
        "NaN",
        "Infinity",
        "null",
        "1e",
        "1e+",
        "1e-",
        "",
        ".0",
        "0.",
    ] {
        values.push(number.into());
        values.push(serde_json::to_string(number).unwrap());
    }
    for mantissa in [
        "0",
        "-0.0",
        "1",
        "10",
        "100.00",
        "0.001",
        "4.294967295",
        "42949672950",
    ] {
        for exponent in [
            "0",
            "+0",
            "-0",
            "1",
            "-1",
            "9",
            "-9",
            "20",
            "-20",
            "2147483647",
            "2147483648",
            "-2147483648",
            "-2147483649",
            "99999999999999999999999999",
        ] {
            let number = format!("{mantissa}e{exponent}");
            values.push(serde_json::to_string(&number).unwrap());
            values.push(number);
        }
    }
    values
}

fn observation(result: Result<Request, ProtocolError>) -> String {
    match result {
        Ok(Request::Stat(path) | Request::MakeDir(path) | Request::Remove(path)) => {
            format!("OK\t{}\t0\t\t", hex(path.as_bytes()))
        }
        Ok(Request::ListDir { path, depth }) => {
            format!("OK\t{}\t{depth}\t\t", hex(path.as_bytes()))
        }
        Ok(Request::Move {
            source,
            destination,
        }) => format!(
            "OK\t\t0\t{}\t{}",
            hex(source.as_bytes()),
            hex(destination.as_bytes())
        ),
        Err(ProtocolError::InvalidArgument) => "ERR\tinvalid_argument".into(),
        other => panic!("unexpected request observation {other:?}"),
    }
}

#[test]
#[ignore = "explicit original generated request/default Connect HTTP reference; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
fn original_generated_file_request_decoding_matches_values_and_rejections() {
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    let mut count = 0;
    let mut check = |method: &str, body: &[u8]| {
        let input = json!({"Method": method, "Body": hex(body)});
        let actual = file_protocol::decode_request(method, body, body.len().max(1));
        oracle.check(
            &format!("J\t{}", hex(input.to_string().as_bytes())),
            &observation(actual),
        );
        count += 1;
    };
    for method in ["Stat", "MakeDir", "ListDir", "Move", "Remove"] {
        for body in messages() {
            check(method, &body);
        }
    }
    for number in numbers() {
        check("ListDir", format!(r#"{{"depth":{number}}}"#).as_bytes());
    }
    for (method, body) in exponent_messages() {
        check(method, &body);
    }
    // Every individual raw byte in both a known and a discarded string, plus
    // discarded object keys. Invalid UTF-8 cannot become a replacement character.
    for byte in 0..=255 {
        for (prefix, suffix) in [
            (b"{\"path\":\"".as_slice(), b"\"}".as_slice()),
            (b"{\"future\":\"", b"\"}"),
            (b"{\"future\":{\"", b"\":0}}"),
        ] {
            let body = [prefix, &[byte], suffix].concat();
            check("Stat", &body);
        }
    }
    oracle.finish();
    eprintln!(
        "{count} original generated/default Connect request HTTP cases; decoded fields and invalid_argument classification, not error text or file effects."
    );
}

#[test]
#[ignore = "explicit original Connect malformed-number reference; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
fn original_generated_bare_exponents_match_known_and_discarded_fields() {
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    for (body, observed) in [
        (r#"{"depth":1e}"#, "OK\t\t1\t\t"),
        (r#"{"future":1e}"#, "OK\t\t0\t\t"),
        (r#"{"future":1e+}"#, "OK\t\t0\t\t"),
        (r#"{"future":1e-}"#, "OK\t\t0\t\t"),
    ] {
        let input = json!({"Method":"ListDir", "Body":hex(body.as_bytes())});
        oracle.check(
            &format!("J\t{}", hex(input.to_string().as_bytes())),
            observed,
        );
        assert_eq!(
            observation(file_protocol::decode_request(
                "ListDir",
                body.as_bytes(),
                body.len()
            )),
            observed
        );
    }
    oracle.finish();
    eprintln!(
        "4 previously recorded malformed-exponent gaps now match the original Connect decoder."
    );
}

fn exponent_messages() -> Vec<(&'static str, Vec<u8>)> {
    let mut cases = Vec::new();
    for mantissa in [
        "0",
        "-0",
        "0.0",
        "-0.000",
        "1",
        "1.5",
        "1.00",
        "4294967295",
        "4294967296",
        "1000000000000000000000000",
        "-1",
        "01",
        "+1",
        "1.",
        "-.0",
    ] {
        for exponent in ["e", "E", "e+", "e-", "e0", "e+0", "e-0", "ee", "e_"] {
            let number = format!("{mantissa}{exponent}");
            for gap in ["", " ", "\t", "\r\n"] {
                for body in [
                    format!(r#"{{"depth":{number}{gap}}}"#),
                    format!(r#"{{"depth":{number}{gap},"path":"p"}}"#),
                    format!(r#"{{"depth":{number}{gap}"path":"p"}}"#),
                    format!(r#"{{"future":{number}{gap}}}"#),
                    format!(r#"{{"future":[{number}{gap}]}}"#),
                    format!(r#"{{"future":{{"n":{number}{gap}}}}}"#),
                    format!(r#"{{"path":{number}{gap}}}"#),
                ] {
                    cases.push(("ListDir", body.into_bytes()));
                }
            }
            cases.push((
                "ListDir",
                format!(r#"{{"depth":{}}}"#, serde_json::to_string(&number).unwrap()).into_bytes(),
            ));
            for key in ["source", "destination"] {
                cases.push(("Move", format!(r#"{{"{key}":{number}}}"#).into_bytes()));
            }
        }
    }
    for byte in 0..=255 {
        for (prefix, suffix) in [
            (b"{\"depth\":1e".as_slice(), b"}".as_slice()),
            (b"{\"future\":[0e+", b"]}"),
        ] {
            cases.push(("ListDir", [prefix, &[byte], suffix].concat()));
        }
    }
    cases
}

#[tokio::test]
async fn protobuf_decoding_retains_original_admission_deadline_and_failure_capacity() {
    let session = session().await;
    let pool = FileRpcRequests::new(limits()).unwrap();
    for (body, expected) in [
        (
            r#"{"future":{"path":1},"depth":"1e2"}"#,
            Ok(Request::ListDir {
                path: String::new(),
                depth: 100,
            }),
        ),
        (
            r#"{"future":0e+,"depth":1e}"#,
            Ok(Request::ListDir {
                path: String::new(),
                depth: 1,
            }),
        ),
        (r#"{"depth":0e+}"#, Err(ProtocolError::InvalidArgument)),
        (
            r#"{"future":"\ud800"}"#,
            Err(ProtocolError::InvalidArgument),
        ),
        (
            r#"{"depth":null,"depth":null}"#,
            Err(ProtocolError::InvalidArgument),
        ),
    ] {
        let admitted = pool.admit().unwrap();
        let end = admitted.deadline();
        let auth = admitted.authenticate(&session, headers()).await.unwrap();
        let body = Body::from_stream(stream::iter(
            body.as_bytes()
                .chunks(16)
                .map(|bytes| Ok::<_, std::io::Error>(Bytes::copy_from_slice(bytes)))
                .collect::<Vec<_>>(),
        ));
        let result = auth.decode("ListDir", body).await;
        full(&pool);
        match (result, expected) {
            (Ok(decoded), Ok(expected)) => {
                assert_eq!(decoded.request(), &expected);
                assert_eq!(decoded.deadline(), end);
                drop(decoded);
            }
            (Err(failure), Err(expected)) => {
                assert_eq!(failure.error(), &Error::Protocol(expected));
                assert_eq!(failure.deadline(), end);
                assert!(!failure.has_response());
                drop(failure);
            }
            _ => panic!("wrong request decoding result"),
        }
        released(&pool).await;
    }
}
