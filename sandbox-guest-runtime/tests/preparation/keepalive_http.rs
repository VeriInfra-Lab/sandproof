use super::*;
use futures_util::FutureExt;
use sandproof_guest_service::command_requests::http::Rpc;

#[tokio::test]
async fn keepalive_http_rejects_before_body_or_process_effect_and_keeps_original_reply_capacity() {
    let fixture = fixture().await;
    let registry = registry();
    let ordinary = requests(1);
    let control = requests(1);
    let http = http(&fixture, &registry, &ordinary, &control);
    let claims = fixture.directory.path().join("pid-claims.bin");
    let before = std::fs::read(&claims).unwrap();
    for method in ["Start", "Connect"] {
        for value in ["0", "-1", "9223372037", "-9223372037", "20211507185753197"] {
            let mut rejected = request(method, &json!({}));
            rejected
                .headers_mut()
                .append("keepalive-ping-interval", value.parse().unwrap());
            rejected
                .headers_mut()
                .append("keepalive-ping-interval", "50".parse().unwrap());
            unread(&mut rejected);
            let reply = http.handle(rejected).await.unwrap();
            assert_eq!(reply.status(), StatusCode::OK);
            assert_eq!(reply.headers()["content-type"], "application/connect+json");
            let payload = bytes(reply).await;
            assert_eq!(
                json_frame(&payload, 2),
                json!({"error":{
                    "code":"invalid_argument", "message":"keepalive interval must be between 1 and 9223372036 seconds"
                }})
            );
            let copy = payload.clone();
            drop(payload);
            full(&ordinary);
            rotates(&fixture).await;
            drop(copy);
            assert!(ordinary.admit().is_ok());
            fixture.assert_no_effect();
            assert_eq!(fixture.gate_count(), 0);
            assert_eq!(registry.available_slots(), 1);
            assert_eq!(std::fs::read(&claims).unwrap(), before);
        }
    }
    // The stream-only policy cannot disable the reserved control path or List.
    let mut list = request("List", &json!({}));
    list.headers_mut()
        .insert("keepalive-ping-interval", "0".parse().unwrap());
    assert_eq!(bytes(http.handle(list).await.unwrap()).await, "{}");
    let mut close = request("CloseStdin", &json!({"process":{"pid":42}}));
    close
        .headers_mut()
        .insert("keepalive-ping-interval", "0".parse().unwrap());
    let reply = http.handle(close).await.unwrap();
    assert_eq!(reply.status(), StatusCode::NOT_FOUND);
    let payload: Value = serde_json::from_slice(&bytes(reply).await).unwrap();
    assert_eq!(payload["code"], "not_found");
    assert!(control.admit().is_ok());
    assert!(fixture.owners.reserve().is_ok());
}

#[tokio::test]
async fn keepalive_http_does_not_precede_token_or_explicit_account_preflight() {
    let fixture = fixture().await;
    let registry = registry();
    let ordinary = requests(1);
    let control = requests(1);
    let http = http(&fixture, &registry, &ordinary, &control);
    for token in ["wrong", "token"] {
        let mut rejected = request("Start", &json!({}));
        let h = rejected.headers_mut();
        h.insert("x-access-token", token.parse().unwrap());
        h.insert(
            "authorization",
            format!("Basic {}", STANDARD.encode("missing:"))
                .parse()
                .unwrap(),
        );
        h.insert("keepalive-ping-interval", "0".parse().unwrap());
        unread(&mut rejected);
        let reply = http.handle(rejected).await.unwrap();
        assert_eq!(
            reply.status(),
            if token == "wrong" {
                StatusCode::UNAUTHORIZED
            } else {
                StatusCode::OK
            }
        );
        let payload = bytes(reply).await;
        if token == "wrong" {
            let decoded: Value = serde_json::from_slice(&payload).unwrap();
            assert_eq!(decoded["code"], 401);
        } else {
            assert_eq!(
                json_frame(&payload, 2),
                json!({"error":{
                    "code":"unauthenticated", "message":"invalid username: 'missing'"
                }})
            );
        }
        drop(payload);
        assert!(ordinary.admit().is_ok());
        fixture.assert_no_effect();
        assert_eq!(fixture.gate_count(), 0);
    }
}

fn long_requests() -> CommandRequests {
    CommandRequests::new(command_requests::Limits {
        in_flight: 2,
        timeout: Duration::from_mins(2),
        headers: HeaderLimits {
            bytes: 1024,
            fields: 16,
        },
        body: HttpBodyLimits {
            total_bytes: 16384,
            frame_bytes: 16384,
            frames: 8,
            trailer_bytes: 64,
            trailer_fields: 2,
        },
    })
    .unwrap()
}

async fn selected(
    pool: &CommandRequests,
    fixture: &Fixture,
    rpc: Rpc,
    payload: &Value,
    value: Option<&str>,
) -> Ready {
    let mut request = request(rpc.method(), payload);
    if let Some(value) = value {
        request
            .headers_mut()
            .append("keepalive-ping-interval", value.parse().unwrap());
        request
            .headers_mut()
            .append("keepalive-ping-interval", "0".parse().unwrap());
    }
    let (parts, body) = request.into_parts();
    pool.admit()
        .unwrap()
        .authenticate(&fixture.session, parts.headers)
        .await
        .unwrap()
        .prepare(&accounts(fixture), settings().limits)
        .await
        .unwrap()
        .select_json(&parts.method, rpc)
        .unwrap()
        .decode(body)
        .await
        .unwrap()
}

async fn interval(body: &mut BodyDataStream, first: Bytes, seconds: u64) {
    // Test the real selected command body, without HTTP's blocking compression
    // worker or a real network clock. Keep the last raw frame until time pauses.
    tokio::time::pause();
    drop(first);
    assert!(body.next().now_or_never().is_none());
    let start = Instant::now();
    tokio::time::advance(
        Duration::from_secs(seconds)
            .checked_sub(Duration::from_millis(1))
            .unwrap(),
    )
    .await;
    assert!(body.next().now_or_never().is_none(), "early keepalive");
    tokio::time::advance(Duration::from_millis(1)).await;
    let ping = timeout(Duration::from_millis(2), body.next())
        .await
        .expect("selected interval was not used")
        .unwrap()
        .unwrap();
    assert_eq!(json_frame(&ping, 0), json!({"event":{"keepalive":{}}}));
    assert!(
        Instant::now().duration_since(start)
            <= Duration::from_secs(seconds) + Duration::from_millis(2)
    );
    drop(ping);
    tokio::time::resume();
}

#[tokio::test]
async fn selected_keepalive_reaches_real_start_and_connect_bodies_without_changing_execution_lifetime()
 {
    for (start_value, start_seconds, connect_value, connect_seconds) in [
        (Some("+0001"), 1, Some("50"), 50),
        (None, 90, Some("bad"), 90),
        (Some("9223372036854775808"), 90, Some(""), 90),
    ] {
        let fixture = fixture().await;
        let registry = registry();
        let registrar = PidRegistrar::new(store(&fixture));
        let pool = long_requests();
        let start = selected(
            &pool,
            &fixture,
            Rpc::Start,
            &json!({"process":{
                "cmd":"/bin/sh", "args":["-c","printf once >> effect; exec /bin/cat"]
        },"stdin":true}),
            start_value,
        )
        .await;
        let reply = start
            .start_pid(
                &accounts(&fixture),
                &settings(),
                preparation(
                    &fixture,
                    &registry,
                    &registrar,
                    &fixture.directory.path().join("gates"),
                ),
                response(),
            )
            .await
            .unwrap();
        let mut body = reply.into_body().into_data_stream();
        let first = frame(&mut body).await;
        let pid = u32::try_from(
            json_frame(&first, 0)["event"]["start"]["pid"]
                .as_u64()
                .unwrap(),
        )
        .unwrap();
        let command = Stop(registry.connectable(pid).unwrap());
        interval(&mut body, first, start_seconds).await;
        drop(body);
        let pid = command.0.identity().pid;
        let connect = selected(
            &pool,
            &fixture,
            Rpc::Connect,
            &json!({"process":{"pid":pid}}),
            connect_value,
        )
        .await;
        let reply = connect.execute_pid(&registry, response()).await.unwrap();
        let mut body = reply.into_body().into_data_stream();
        let first = frame(&mut body).await;
        assert_eq!(
            json_frame(&first, 0),
            json!({"event":{"start":{"pid":pid}}})
        );
        interval(&mut body, first, connect_seconds).await;
        assert!(command.0.status().completion.is_none());
        assert!(command.0.status().deadline_action.is_none());
        command
            .0
            .write_input(command.0.identity(), b"alive")
            .await
            .unwrap();
        data(&mut body, "stdout", b"alive").await;
        command.0.close_input(command.0.identity()).unwrap();
        end(&mut body).await;
        drop(body);
        clean(&fixture, &registry, command).await;
        assert_eq!(
            std::fs::read(fixture.directory.path().join("effect")).unwrap(),
            b"once"
        );
        assert!(pool.admit().is_ok());
    }
}
