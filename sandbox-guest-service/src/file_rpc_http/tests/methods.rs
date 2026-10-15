use super::*;
use std::{
    future::{Future, poll_fn},
    task::Poll,
};

#[tokio::test]
async fn non_post_methods_follow_basic_preflight_and_precede_codec_headers_and_body() {
    let root = tempfile::tempdir().unwrap();
    let f = fixture(root.path(), 65536).await;
    let mut expected_calls = 0;
    for rpc in ["Stat", "MakeDir", "ListDir", "Move", "Remove"] {
        for method in [
            "GET", "HEAD", "PUT", "PATCH", "DELETE", "OPTIONS", "get", "post",
        ] {
            for auth in [None, Some("Basic dXNlcjo="), Some("Basic bWlzc2luZzo=")] {
                let body = Body::from_stream(futures_util::stream::poll_fn(
                    |_| -> std::task::Poll<Option<Result<Bytes, std::io::Error>>> {
                        panic!("method/Basic rejection polled request body")
                    },
                ));
                let mut req = request(rpc, body);
                *req.method_mut() = method.parse().unwrap();
                if let Some(auth) = auth {
                    req.headers_mut()
                        .insert("authorization", auth.parse().unwrap());
                    expected_calls += 1;
                } else {
                    // Method rejection also precedes unsupported media type.
                    req.headers_mut()
                        .insert("content-type", "application/proto".parse().unwrap());
                }
                for (name, value) in [
                    ("content-encoding", "br"),
                    ("connect-protocol-version", "bad"),
                    ("connect-timeout-ms", "0"),
                ] {
                    req.headers_mut().insert(name, value.parse().unwrap());
                }
                let response = f.http.handle(req).await.unwrap();
                assert_eq!(f.calls.load(Ordering::SeqCst), expected_calls);
                full(&f);
                file_full(&f).await;
                f.session
                    .initialize(br#"{"accessToken":"token","defaultUser":"user"}"#)
                    .await
                    .unwrap();
                if auth == Some("Basic bWlzc2luZzo=") {
                    assert_eq!(response.status(), 401);
                    let data = drain(response.into_body()).await;
                    let value: serde_json::Value = serde_json::from_slice(&data).unwrap();
                    assert_eq!(value["message"], "invalid username: 'missing'");
                } else {
                    assert_eq!(response.status(), 405);
                    assert_eq!(response.headers()["allow"], "POST");
                    assert_eq!(
                        response.headers().len(),
                        if method == "HEAD" { 1 } else { 2 }
                    );
                    assert!(drain(response.into_body()).await.is_empty());
                }
                released(&f).await;
            }
        }
    }
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn failed_method_response_retains_original_two_slots_and_shorter_deadline() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let root = tempfile::tempdir().unwrap();
        let f = fixture(root.path(), 65536).await;
        // Valid construction is mandatory. Hold the sole blocking thread so
        // the real method-response encoder cannot run before its deadline.
        let (entered, started) = tokio::sync::oneshot::channel();
        let (release, resumed) = std::sync::mpsc::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            entered.send(()).unwrap();
            resumed.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        started.await.unwrap();
        tokio::time::pause();
        let start = Instant::now();
        let mut req = request("MakeDir", Body::from(r#"{"path":"refused"}"#));
        *req.method_mut() = axum::http::Method::GET;
        let mut response = Box::pin(f.http.handle(req));
        assert!(poll_fn(|cx| Poll::Ready(response.as_mut().poll(cx).is_pending())).await);
        // Advance past the deadline: Tokio timers round to millisecond ticks.
        // Awaiting exactly at the boundary can leave the timer pending while
        // the occupied blocking pool prevents automatic clock advancement.
        tokio::time::advance(Duration::from_secs(6)).await;
        let Poll::Ready(result) = poll_fn(|cx| Poll::Ready(response.as_mut().poll(cx))).await
        else {
            panic!("method response did not expire while its encoder was queued");
        };
        let failure = result.unwrap_err();
        assert_eq!(
            failure.error(),
            &Error::Response(file_rpc_response::Error::DeadlineExceeded)
        );
        // Original file-pool deadline (5s), not the request pool's 30s.
        assert_eq!(failure.deadline(), Some(start + Duration::from_secs(5)));
        tokio::time::resume();
        full(&f);
        file_full(&f).await;
        assert_eq!(f.calls.load(Ordering::SeqCst), 0);
        f.session
            .initialize(br#"{"accessToken":"token","defaultUser":"user"}"#)
            .await
            .unwrap();
        assert!(!root.path().join("refused").exists());
        drop(failure);
        // Even after the HTTP failure is dropped, queued encoder work owns
        // both slots until it actually observes cancellation/expiry and ends.
        full(&f);
        file_full(&f).await;
        release.send(()).unwrap();
        blocker.await.unwrap();
        released(&f).await;
        let response = f
            .file_probe
            .handle(request("MakeDir", Body::from(r#"{"path":"recovered"}"#)))
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        drain(response.into_body()).await;
        assert!(root.path().join("recovered").is_dir());
    });
}
