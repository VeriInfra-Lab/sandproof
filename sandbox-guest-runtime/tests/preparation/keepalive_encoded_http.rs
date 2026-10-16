//! The HTTP encoder must retain the raw clock acknowledgement until its last
//! encoded byte owner releases it, including split gzip output.
use super::*;
use futures_util::FutureExt;

struct Wire {
    value: Value,
    last: Bytes,
}

fn length(raw: &[u8]) -> Option<usize> {
    let header: &[u8; 5] = raw.get(..5)?.try_into().unwrap();
    let length = 5 + u32::from_be_bytes(header[1..].try_into().unwrap()) as usize;
    assert!(length <= 8192 && raw.len() <= length);
    Some(length)
}

fn decoded(raw: &[u8], gzip: bool, flag: u8) -> Value {
    assert_eq!(Some(raw.len()), length(raw));
    assert_eq!(raw[0], flag | u8::from(gzip));
    let payload = if gzip {
        framing::expanded(&raw[5..])
    } else {
        raw[5..].to_vec()
    };
    serde_json::from_slice(&payload).unwrap()
}

async fn wire(body: &mut BodyDataStream, gzip: bool, flag: u8) -> Wire {
    let mut raw = Vec::new();
    loop {
        let part = frame(body).await;
        assert!(!part.is_empty());
        raw.extend_from_slice(&part);
        if length(&raw) == Some(raw.len()) {
            return Wire {
                value: decoded(&raw, gzip, flag),
                last: part,
            };
        }
        // Every non-final part must be released before polling the next part.
        drop(part);
    }
}

// Only this test runtime has one blocking worker. The sentinel runs behind any
// encoder queued by the first poll. A bounded synchronous barrier keeps the
// paused Tokio clock from auto-advancing while that real worker is running.
fn settled(body: &mut BodyDataStream) -> Option<Bytes> {
    let ready = body.next().now_or_never().or_else(|| {
        let (sent, received) = std::sync::mpsc::sync_channel(1);
        let _job = tokio::task::spawn_blocking(move || sent.send(()).unwrap());
        received.recv_timeout(Duration::from_secs(5)).unwrap();
        body.next().now_or_never()
    });
    ready.map(|result| result.expect("unexpected stream end").unwrap())
}

fn due(body: &mut BodyDataStream, gzip: bool) -> Wire {
    let mut raw = Vec::new();
    loop {
        let part = settled(body).expect("due keepalive was postponed after encoder completion");
        assert!(!part.is_empty());
        raw.extend_from_slice(&part);
        if length(&raw) == Some(raw.len()) {
            return Wire {
                value: decoded(&raw, gzip, 0),
                last: part,
            };
        }
        drop(part);
    }
}

struct Paused;
impl Drop for Paused {
    fn drop(&mut self) {
        tokio::time::resume();
    }
}

async fn held_interval(body: &mut BodyDataStream, first: Bytes, seconds: u64, gzip: bool) {
    let last = first.clone();
    drop(first);
    tokio::time::pause();
    let _paused = Paused;
    assert!(settled(body).is_none());
    tokio::time::advance(Duration::from_secs(seconds + 7)).await;
    assert!(
        settled(body).is_none(),
        "retained encoded bytes allowed prefetch"
    );
    drop(last);
    let released = Instant::now();
    // No body poll at release: the timer must start at last-owner destruction,
    // not when the pending HTTP payload is next asked for EOF.
    tokio::time::advance(Duration::from_millis(250)).await;
    assert!(
        settled(body).is_none(),
        "clock began before the final encoded owner released"
    );
    tokio::time::advance(
        Duration::from_secs(seconds)
            .checked_sub(Duration::from_millis(251))
            .unwrap(),
    )
    .await;
    assert!(settled(body).is_none(), "early encoded keepalive");
    // The original clock tests allow two milliseconds for Tokio timer-wheel
    // resolution. Encoding runs under the barrier without advancing fake time.
    tokio::time::advance(Duration::from_millis(3)).await;
    let ping = due(body, gzip);
    assert_eq!(ping.value, json!({"event":{"keepalive":{}}}));
    assert_eq!(
        Instant::now().duration_since(released),
        Duration::from_secs(seconds) + Duration::from_millis(2)
    );
    drop(ping.last);
    assert!(settled(body).is_none(), "unexpected keepalive burst");
}

fn pool() -> CommandRequests {
    CommandRequests::new(command_requests::Limits {
        in_flight: 1,
        timeout: Duration::from_mins(10),
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

async fn open(
    http: &Http,
    method: &str,
    payload: &Value,
    seconds: u64,
    gzip: bool,
) -> BodyDataStream {
    let mut input = request(method, payload);
    input.headers_mut().insert(
        "keepalive-ping-interval",
        seconds.to_string().parse().unwrap(),
    );
    if gzip {
        input
            .headers_mut()
            .insert("connect-accept-encoding", "gzip".parse().unwrap());
    }
    let response = http.handle(input).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["content-type"],
        "application/connect+json"
    );
    assert_eq!(
        response
            .headers()
            .get("connect-content-encoding")
            .map(axum::http::HeaderValue::as_bytes),
        gzip.then_some(b"gzip".as_slice())
    );
    response.into_body().into_data_stream()
}

#[test]
fn encoded_start_connect_and_data_keep_the_selected_clock_until_last_byte_release() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        for gzip in [false, true] {
            let fixture = fixture().await;
            let registry = registry();
            let ordinary = pool();
            let control = requests(1);
            let mut response = responses();
            response.http.chunk_bytes = 13;
            let http = CommandHttp::new(
                fixture.session.clone(),
                RequestPools {
                    ordinary: ordinary.clone(),
                    control: control.clone(),
                },
                accounts(&fixture),
                settings(),
                processes(&fixture, &registry, &PidRegistrar::new(store(&fixture))),
                response,
            )
            .unwrap();
            let mut body = open(
                &http,
                "Start",
                &json!({"process":{
                "cmd":"/bin/sh", "args":["-c","printf once >> effect; exec /bin/cat"]
            },"stdin":true}),
                1,
                gzip,
            )
            .await;
            let first = wire(&mut body, gzip, 0).await;
            let pid =
                u32::try_from(first.value["event"]["start"]["pid"].as_u64().unwrap()).unwrap();
            let command = Stop(registry.connectable(pid).unwrap());
            full(&ordinary);
            held_interval(&mut body, first.last, 1, gzip).await;
            drop(body);
            assert!(ordinary.admit().is_ok());

            let mut body = open(&http, "Connect", &json!({"process":{"pid":pid}}), 50, gzip).await;
            let first = wire(&mut body, gzip, 0).await;
            assert_eq!(first.value, json!({"event":{"start":{"pid":pid}}}));
            held_interval(&mut body, first.last, 50, gzip).await;
            assert!(command.0.status().completion.is_none());
            assert!(command.0.status().deadline_action.is_none());
            command
                .0
                .write_input(command.0.identity(), b"alive")
                .await
                .unwrap();
            let data = wire(&mut body, gzip, 0).await;
            assert_eq!(
                data.value,
                json!({"event":{"data":{"stdout":STANDARD.encode(b"alive")}}})
            );
            held_interval(&mut body, data.last, 50, gzip).await;
            command.0.close_input(command.0.identity()).unwrap();
            let end = wire(&mut body, gzip, 0).await;
            assert_eq!(
                end.value,
                json!({"event":{"end":{"exited":true,"status":"exit status 0"}}})
            );
            drop(end.last);
            let eos = wire(&mut body, gzip, 2).await;
            assert_eq!(eos.value, json!({}));
            drop(body);
            full(&ordinary);
            drop(eos.last);
            assert!(ordinary.admit().is_ok());
            assert!(control.admit().is_ok());
            clean(&fixture, &registry, command).await;
            assert_eq!(
                std::fs::read(fixture.directory.path().join("effect")).unwrap(),
                b"once"
            );
        }
    });
}
