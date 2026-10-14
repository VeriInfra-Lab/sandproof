use super::*;

#[tokio::test]
async fn unsupported_encoding_keeps_account_priority_original_slots_and_no_file_effects() {
    let root = tempfile::tempdir().unwrap();
    for (wire_bytes, authorization, expected_calls, status) in [
        (65536, None, 0, 501),
        (65536, Some("Basic dXNlcjo="), 1, 501),
        (65536, Some("Basic bWlzc2luZzo="), 1, 401),
        (2, None, 0, 501),
    ] {
        let f = fixture(root.path(), wire_bytes).await;
        let polls = Arc::new(AtomicUsize::new(0));
        let counted = polls.clone();
        let body = Body::from_stream(futures_util::stream::poll_fn(move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
            std::task::Poll::Ready(Some(Err::<Bytes, _>(std::io::Error::other("unread"))))
        }));
        let mut req = request("MakeDir", body);
        for (name, value) in [
            ("content-encoding", "br"),
            ("accept-encoding", "gzip"),
            ("connect-protocol-version", "2"),
            ("connect-timeout-ms", "invalid"),
        ] {
            req.headers_mut().insert(name, value.parse().unwrap());
        }
        if let Some(value) = authorization {
            req.headers_mut()
                .insert("authorization", value.parse().unwrap());
        }
        let result = f.http.handle(req).await;
        assert_eq!(polls.load(Ordering::SeqCst), 0);
        assert_eq!(f.calls.load(Ordering::SeqCst), expected_calls);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        full(&f);
        file_full(&f).await;
        f.session
            .initialize(br#"{"accessToken":"token","defaultUser":"user"}"#)
            .await
            .unwrap();
        if wire_bytes == 2 {
            let failure = result.unwrap_err();
            assert_eq!(
                failure.error(),
                &Error::Response(file_rpc_response::Error::ResponseLimit)
            );
            assert!(
                failure.deadline().unwrap() <= tokio::time::Instant::now() + Duration::from_secs(5)
            );
            drop(failure);
        } else {
            let response = result.unwrap();
            assert_eq!(response.status(), status);
            assert!(!response.headers().contains_key("content-encoding"));
            let body: serde_json::Value =
                serde_json::from_slice(&drain(response.into_body()).await).unwrap();
            assert_eq!(
                body["code"],
                if status == 401 {
                    "unauthenticated"
                } else {
                    "unimplemented"
                }
            );
            assert_eq!(
                body["message"],
                if status == 401 {
                    "invalid username: 'missing'"
                } else {
                    "unknown compression \"br\": supported encodings are gzip"
                }
            );
        }
        released(&f).await;
    }
}
