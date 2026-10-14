use super::*;
use futures_util::StreamExt;
use sandproof_guest_runtime::gzip::{EncodeLimits, Encoder};

#[test]
#[ignore = "explicit fixed Go Connect request gzip reference; set SANDPROOF_GUEST_CONNECT_REFERENCE"]
fn fixed_connect_decompression_and_response_selection_keep_first_encoding_field() {
    let mut oracle = Reference::from_env("SANDPROOF_GUEST_CONNECT_REFERENCE");
    let (_signal, cancelled) = watch::channel(false);
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut count = 0;
    for sent in [
        vec![],
        vec![""],
        vec!["identity", "gzip"],
        vec!["gzip"],
        vec!["gzip", "identity"],
    ] {
        for accept in [
            vec![],
            vec!["identity"],
            vec!["br"],
            vec!["gzip;q=0"],
            vec!["gzip"],
        ] {
            let mut headers = HeaderMap::new();
            for value in &sent {
                headers.append("content-encoding", value.parse().unwrap());
            }
            for value in &accept {
                headers.append("accept-encoding", value.parse().unwrap());
            }
            let gzip = selection::is_gzip_request(&headers);
            for (i, raw) in [
                Vec::new(),
                b"{}".to_vec(),
                br#"{"path":"/file"}"#.to_vec(),
                format!(r#"{{"path":"{}"}}"#, "a".repeat(4000)).into_bytes(),
            ]
            .into_iter()
            .enumerate()
            {
                let encoded = if !gzip || raw.is_empty() {
                    raw.clone()
                } else if i == 2 {
                    [
                        compressed(b""),
                        compressed(&raw[..5]),
                        compressed(&raw[5..]),
                    ]
                    .concat()
                } else {
                    compressed(&raw)
                };
                let decoded = if gzip {
                    compression::expand(&encoded, 65536, 13, deadline, &cancelled).unwrap()
                } else {
                    encoded.clone()
                };
                assert_eq!(decoded, raw);
                let encoding = if selection::response_encoding(&headers)
                    == crate::file_rpc_response::Encoding::Gzip
                {
                    "gzip"
                } else {
                    ""
                };
                let input = serde_json::json!({"Sent":sent,"Accept":accept,"Data":hex(&encoded)});
                oracle.check(
                    &format!("Z\t{}", hex(input.to_string().as_bytes())),
                    &format!("200\t{}\t{}", hex(encoding.as_bytes()), hex(&decoded)),
                );
                count += 1;
            }
        }
    }
    oracle.finish();
    eprintln!(
        "{count} fixed Connect request decompression/response encoding cases; passthrough codec, not protobuf or filesystem."
    );
}

pub(crate) fn compressed(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = Encoder::new(
        bytes,
        EncodeLimits {
            chunk_bytes: 17,
            decoded_bytes: 65536,
            encoded_bytes: 65536,
        },
    )
    .unwrap();
    let mut output = Vec::new();
    while let Some(chunk) = encoder.next_chunk().unwrap() {
        output.extend(chunk);
    }
    output
}
fn gzip_headers() -> HeaderMap {
    let mut h = headers();
    h.insert("content-encoding", "gzip".parse().unwrap());
    h
}
fn chunks(encoded: &[u8]) -> Body {
    Body::from_stream(stream::iter(
        encoded
            .chunks(17)
            .map(|b| Ok::<_, std::io::Error>(Bytes::copy_from_slice(b)))
            .collect::<Vec<_>>(),
    ))
}

#[tokio::test]
async fn gzip_request_decodes_all_methods_and_members_under_original_lease() {
    let session = session().await;
    let pool = FileRpcRequests::new(limits()).unwrap();
    for (method, raw) in [
        ("Stat", r#"{"path":"/file"}"#),
        ("MakeDir", r#"{"path":"/dir"}"#),
        ("ListDir", r#"{"path":"/dir","depth":2}"#),
        ("Move", r#"{"source":"/a","destination":"/b"}"#),
        ("Remove", r#"{"path":"/file"}"#),
    ] {
        // Split valid JSON across distinct gzip members.
        let pieces = if method == "Stat" {
            vec![&raw.as_bytes()[..5], &raw.as_bytes()[5..]]
        } else {
            vec![raw.as_bytes()]
        };
        let encoded: Vec<_> = pieces.into_iter().flat_map(compressed).collect();
        let auth = pool
            .admit()
            .unwrap()
            .authenticate(&session, gzip_headers())
            .await
            .unwrap();
        let end = auth.deadline();
        let decoded = auth.decode(method, chunks(&encoded)).await.ok().unwrap();
        assert_eq!(
            decoded.request(),
            &file_protocol::decode_request(method, raw.as_bytes(), 64).unwrap()
        );
        assert_eq!(decoded.deadline(), end);
        full(&pool);
        let mut init = Box::pin(session.initialize(br#"{"accessToken":"token"}"#));
        pending(init.as_mut()).await;
        drop(decoded);
        init.await.unwrap();
        released(&pool).await;
    }
}

#[tokio::test]
async fn gzip_request_limits_and_real_failures_remain_distinct_and_keep_capacity() {
    let session = session().await;
    let raw = format!(r#"{{"path":"{}"}}"#, "a".repeat(100));
    let encoded = compressed(raw.as_bytes());
    assert!(encoded.len() < 64);
    // Both encoded and decoded caps are exact, independently enforced.
    for (raw, cap) in [
        (raw.as_bytes(), raw.len()),
        (b"{}".as_slice(), compressed(b"{}").len()),
    ] {
        for delta in [0, 1] {
            let mut l = limits();
            l.body.total_bytes = (cap - delta) as u64;
            let pool = FileRpcRequests::new(l).unwrap();
            let auth = pool
                .admit()
                .unwrap()
                .authenticate(&session, gzip_headers())
                .await
                .unwrap();
            let result = auth.decode("Stat", chunks(&compressed(raw))).await;
            if delta == 0 {
                drop(result.ok().unwrap());
            } else {
                let failure = result.err().unwrap();
                assert_eq!(failure.error(), &Error::BodyLimit);
                full(&pool);
                drop(failure);
            }
            released(&pool).await;
        }
    }
    let pool = FileRpcRequests::new(limits()).unwrap();
    let mut crc = compressed(b"{}");
    let n = crc.len();
    crc[n - 8] ^= 1;
    let mut tail = compressed(b"{}");
    tail.extend(b"bad");
    for (data, expected) in [
        (encoded, Error::BodyLimit),
        (
            b"bad".to_vec(),
            Error::Compression {
                stage: CompressionStage::Header,
                error: GzipError::Truncated,
            },
        ),
        (
            crc,
            Error::Compression {
                stage: CompressionStage::Body,
                error: GzipError::Checksum,
            },
        ),
        (
            tail,
            Error::Compression {
                stage: CompressionStage::Body,
                error: GzipError::Truncated,
            },
        ),
        (Vec::new(), Error::Protocol(ProtocolError::InvalidArgument)),
    ] {
        let auth = pool
            .admit()
            .unwrap()
            .authenticate(&session, gzip_headers())
            .await
            .unwrap();
        let end = auth.deadline();
        let failure = auth.decode("Stat", chunks(&data)).await.err().unwrap();
        assert_eq!(failure.error(), &expected);
        assert_eq!(failure.deadline(), end);
        full(&pool);
        session
            .initialize(br#"{"accessToken":"token"}"#)
            .await
            .unwrap();
        drop(failure);
        released(&pool).await;
    }
}

#[tokio::test]
async fn gzip_requires_http_completion_and_headers_precede_decompression() {
    let session = session().await;
    let pool = FileRpcRequests::new(limits()).unwrap();
    let encoded = compressed(b"{}");
    let (body, _) = body(vec![data(&encoded)], Some(encoded.len() as u64 + 1));
    let failure = pool
        .admit()
        .unwrap()
        .authenticate(&session, gzip_headers())
        .await
        .unwrap()
        .decode("Stat", body)
        .await
        .err()
        .unwrap();
    assert_eq!(failure.error(), &Error::Transport);
    drop(failure);
    for version in [true, false] {
        let mut h = gzip_headers();
        h.insert(
            if version {
                "connect-protocol-version"
            } else {
                "connect-timeout-ms"
            },
            "invalid".parse().unwrap(),
        );
        let (body, polls) = super::body(vec![data(b"bad")], Some(3));
        let failure = pool
            .admit()
            .unwrap()
            .authenticate(&session, h)
            .await
            .unwrap()
            .decode("Stat", body)
            .await
            .err()
            .unwrap();
        assert!(matches!(
            failure.error(),
            Error::Version(_) | Error::Timeout(_)
        ));
        assert_eq!(polls.load(Ordering::SeqCst), 0);
        drop(failure);
    }
    let source =
        stream::iter([Ok::<_, std::io::Error>(Bytes::from(encoded))]).chain(stream::pending());
    let auth = pool
        .admit()
        .unwrap()
        .authenticate(&session, gzip_headers())
        .await
        .unwrap();
    let mut waiting = Box::pin(auth.decode("Stat", Body::from_stream(source)));
    pending(waiting.as_mut()).await;
    full(&pool);
    drop(waiting);
    session
        .initialize(br#"{"accessToken":"token"}"#)
        .await
        .unwrap();
    released(&pool).await;
}

#[test]
fn cancelled_or_expired_gzip_decoder_keeps_original_auth_until_queued_worker_exits() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        for expire in [false, true] {
            let session = session().await;
            let pool = FileRpcRequests::new(limits()).unwrap();
            let (entered, started) = tokio::sync::oneshot::channel();
            let (resume, resumed) = std::sync::mpsc::channel();
            let occupied = tokio::task::spawn_blocking(move || {
                entered.send(()).unwrap();
                resumed.recv_timeout(Duration::from_secs(5)).unwrap();
            });
            started.await.unwrap();
            let auth = pool
                .admit()
                .unwrap()
                .authenticate(&session, gzip_headers())
                .await
                .unwrap();
            let mut decoding = Box::pin(auth.decode("Stat", chunks(&compressed(b"{}"))));
            pending(decoding.as_mut()).await;
            if expire {
                tokio::time::pause();
                tokio::time::advance(Duration::from_secs(31)).await;
                let failure = decoding.await.err().unwrap();
                assert_eq!(failure.error(), &Error::DeadlineExceeded);
                drop(failure);
                tokio::time::resume();
            } else {
                drop(decoding);
            }
            full(&pool);
            let mut init = Box::pin(session.initialize(br#"{"accessToken":"token"}"#));
            pending(init.as_mut()).await;
            resume.send(()).unwrap();
            occupied.await.unwrap();
            init.await.unwrap();
            released(&pool).await;
        }
    });
}
