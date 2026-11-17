//! A silent command stream must not leave the exact relay alive after its
//! subscriber disconnects. The guest command itself is outside this process.
use super::*;
use http_body_util::BodyExt;

#[test]
fn completed_responses_do_not_refund_routes_before_the_reaper_runs() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        // Hold the only blocking worker at a barrier. HTTP completion remains
        // possible, but exact-child wait cannot finish yet. No timing race.
        let (entered, started) = tokio::sync::oneshot::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        let worker = tokio::task::spawn_blocking(move || {
            entered.send(()).unwrap();
            blocked.recv().unwrap();
        });
        started.await.unwrap();
        let limits = fixture_limits();
        let gateway = Arc::new(Gateway::new(limits.clone()).unwrap());
        let (authority, _) = watch::channel(false);
        let drain = DataLease::default();
        for _ in 0..limits.max_transfers_per_vm - limits.control_reserve {
            let child = Command::new("/bin/sh")
                .args([
                    "-c",
                    concat!(
                        "cr=$(printf '\r'); while IFS= read -r line; do ",
                        "[ \"$line\" = \"$cr\" ] && break; done; ",
                        "printf 'HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n'",
                    ),
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let response = forward_via_relay(
                Permit {
                    revocation: authority.subscribe(),
                    drain: drain.clone(),
                },
                drain::RelayProcess::new(RelayReaper::new().unwrap().attach(child)),
                request("/process.Process/Start", true, Body::empty()),
                Some(&limits),
                Some(gateway.acquire((7, 3), false).unwrap()),
            )
            .await;
            assert_eq!(response.status(), StatusCode::NO_CONTENT);
            response.into_body().collect().await.unwrap();
        }
        assert!(!drain.drained());
        assert!(matches!(
            gateway.acquire((7, 3), false),
            Err(StatusCode::TOO_MANY_REQUESTS)
        ));
        assert!(gateway.acquire((7, 3), true).is_ok());
        release.send(()).unwrap();
        worker.await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            while !drain.drained() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        // The same unchanged budget admits all ordinary slots again.
        let available: Vec<_> = (0..limits.max_transfers_per_vm - limits.control_reserve)
            .map(|_| gateway.acquire((7, 3), false).unwrap())
            .collect();
        assert_eq!(available.len(), 3);
    });
}

#[tokio::test]
async fn disconnected_silent_response_reaps_relay_and_returns_transfer_capacity() {
    let limits = fixture_limits();
    let gateway = Arc::new(Gateway::new(limits.clone()).unwrap());
    let (revocation, _) = watch::channel(false);
    let drain = DataLease::default();
    // More completed requests than the unchanged per-VM budget. Each child
    // returns headers/data, then remains silent until the gateway closes stdin.
    for _ in 0..8 {
        let child = Command::new("/bin/sh")
            .args([
                "-c",
                concat!(
                    "cr=$(printf '\r'); while IFS= read -r line; do ",
                    "[ \"$line\" = \"$cr\" ] && break; done; ",
                    "printf 'HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n1\r\nx\r\n'; ",
                    "cat >/dev/null",
                ),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let child = drain::RelayProcess::new(RelayReaper::new().unwrap().attach(child));
        let permit = Permit {
            revocation: revocation.subscribe(),
            drain: drain.clone(),
        };
        let lease = gateway.acquire((7, 3), false).unwrap();
        let response = forward_via_relay(
            permit,
            child,
            request("/process.Process/Start", true, Body::empty()),
            Some(&limits),
            Some(lease),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let mut body = response.into_body();
        assert_eq!(
            body.frame().await.unwrap().unwrap().into_data().unwrap(),
            "x"
        );
        assert!(!drain.drained());
        drop(body); // Client disconnect; no guest EOF or authority revocation.
        let returned = tokio::time::timeout(Duration::from_secs(2), async {
            while !drain.drained() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .is_ok();
        if !returned {
            // Clean the original test child even when testing the broken code.
            revocation.send_replace(true);
            tokio::time::timeout(Duration::from_secs(5), async {
                while !drain.drained() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        }
        assert!(returned, "disconnected response retained its exact relay");
    }
    let available: Vec<_> = (0..limits.max_transfers_per_vm - limits.control_reserve)
        .map(|_| gateway.acquire((7, 3), false).unwrap())
        .collect();
    assert!(gateway.acquire((7, 3), false).is_err());
    assert!(gateway.acquire((7, 3), true).is_ok());
    drop(available);
}
