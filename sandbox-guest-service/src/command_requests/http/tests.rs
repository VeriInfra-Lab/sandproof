use super::*;
use std::time::Duration;

mod reference;

fn headers(stream: bool) -> HeaderMap {
    HeaderMap::from_iter([(
        "content-type".parse().unwrap(),
        if stream {
            "application/connect+json"
        } else {
            "application/json"
        }
        .parse()
        .unwrap(),
    )])
}

#[test]
fn selected_json_keeps_first_fields_and_original_header_failure_order() {
    let now = Instant::now();
    let original = now + Duration::from_secs(30);
    for rpc in [
        Rpc::Start,
        Rpc::Connect,
        Rpc::List,
        Rpc::SendInput,
        Rpc::CloseStdin,
        Rpc::SendSignal,
        Rpc::Update,
    ] {
        let mut h = headers(rpc.is_stream());
        let encoding = if rpc.is_stream() {
            "connect-content-encoding"
        } else {
            "content-encoding"
        };
        h.insert(encoding, "br".parse().unwrap());
        h.insert("connect-protocol-version", "bad".parse().unwrap());
        h.insert("connect-timeout-ms", "bad".parse().unwrap());
        assert_eq!(
            select(&h, &Method::GET, rpc, 4096, now, original)
                .unwrap_err()
                .0,
            Error::Method
        );
        let content = h
            .insert("content-type", "invalid".parse().unwrap())
            .unwrap();
        assert_eq!(
            select(&h, &Method::POST, rpc, 4096, now, original)
                .unwrap_err()
                .0,
            Error::ContentType
        );
        h.insert("content-type", content);
        assert!(
            matches!(select(&h, &Method::POST, rpc, 4096, now, original),
            Err((Error::Compression(value), at, _)) if value.value()==b"br" && at==original)
        );
        h.insert(encoding, "identity".parse().unwrap());
        assert!(matches!(
            select(&h, &Method::POST, rpc, 4096, now, original),
            Err((Error::Version(_), _, _))
        ));
        h.insert("connect-protocol-version", "1".parse().unwrap());
        assert!(matches!(
            select(&h, &Method::POST, rpc, 4096, now, original),
            Err((Error::Timeout(_), _, _))
        ));
        // Invalid later duplicates never replace the first values.
        h.append(encoding, "br".parse().unwrap());
        h.append("connect-protocol-version", "bad".parse().unwrap());
        h.insert("connect-timeout-ms", "100".parse().unwrap());
        h.append("connect-timeout-ms", "bad".parse().unwrap());
        let plan = select(&h, &Method::POST, rpc, 4096, now, original).unwrap();
        assert_eq!(plan.request_encoding, Encoding::Identity);
        assert_eq!(plan.response_encoding, Encoding::Identity);
        assert_eq!(plan.deadline, now + Duration::from_millis(100));
        assert_eq!(
            plan.content_type(),
            if rpc.is_stream() {
                "application/connect+json"
            } else {
                "application/json"
            }
        );
        h.insert("connect-protocol-version", "bad".parse().unwrap());
        assert_eq!(
            select(&h, &Method::POST, rpc, 4096, now, original)
                .unwrap_err()
                .1,
            plan.deadline
        );
    }
}

#[test]
fn stream_and_unary_encoding_names_never_cross_and_accept_has_no_http_q_weights() {
    let now = Instant::now();
    for rpc in [Rpc::Start, Rpc::List] {
        let mut h = headers(rpc.is_stream());
        let (sent, accept, other_sent, other_accept) = if rpc.is_stream() {
            (
                "connect-content-encoding",
                "connect-accept-encoding",
                "content-encoding",
                "accept-encoding",
            )
        } else {
            (
                "content-encoding",
                "accept-encoding",
                "connect-content-encoding",
                "connect-accept-encoding",
            )
        };
        h.insert(other_sent, "br".parse().unwrap());
        h.insert(other_accept, "gzip".parse().unwrap());
        for (value, gzip) in [
            ("gzip", true),
            ("br,gzip", true),
            ("br gzip", true),
            ("gzip;q=0", false),
            ("GZIP", false),
            ("br\tgzip", false),
            ("identity", false),
            ("", false),
        ] {
            h.insert(accept, value.parse().unwrap());
            h.append(accept, "gzip".parse().unwrap());
            let plan = select(
                &h,
                &Method::POST,
                rpc,
                4096,
                now,
                now + Duration::from_secs(30),
            )
            .unwrap();
            assert_eq!(
                plan.response_encoding,
                if gzip {
                    Encoding::Gzip
                } else {
                    Encoding::Identity
                }
            );
            assert_eq!(plan.request_encoding, Encoding::Identity);
        }
        h.insert(sent, "gzip".parse().unwrap());
        h.insert(accept, "identity".parse().unwrap());
        let plan = select(
            &h,
            &Method::POST,
            rpc,
            4096,
            now,
            now + Duration::from_secs(30),
        )
        .unwrap();
        assert_eq!(
            (plan.request_encoding, plan.response_encoding),
            (Encoding::Gzip, Encoding::Gzip)
        );
    }
}

#[test]
fn mime_aliases_zero_and_negative_deadlines_remain_observations_not_a_new_execution_clock() {
    let now = Instant::now();
    let original = now + Duration::from_secs(1);
    for rpc in [Rpc::Start, Rpc::List] {
        let mut h = headers(rpc.is_stream());
        let prefix = if rpc.is_stream() {
            "APPLICATION/CONNECT+JSON"
        } else {
            "APPLICATION/JSON"
        };
        h.insert(
            "content-type",
            format!("{prefix}; CHARSET*=utf-8''UTF%2D8")
                .parse()
                .unwrap(),
        );
        for (value, expired) in [
            ("", false),
            ("9999999999", false),
            ("0", true),
            ("-1", true),
        ] {
            h.insert("connect-timeout-ms", value.parse().unwrap());
            let plan = select(&h, &Method::POST, rpc, 4096, now, original).unwrap();
            assert_eq!(plan.deadline, if expired { now } else { original });
            assert!(plan.content_type().ends_with("; charset=utf-8"));
        }
        assert!(matches!(
            select(&h, &Method::POST, rpc, 4096, original, original),
            Err((Error::Request(super::super::Error::DeadlineExceeded), _, _))
        ));
    }
    assert!(!format!("{:?}", RejectedValue(b"private-field".to_vec())).contains("private-field"));
}
