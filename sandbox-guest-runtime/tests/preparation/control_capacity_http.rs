//! A blocked ordinary executor must not consume the reserved control executor.
use super::*;
use std::sync::mpsc;

struct Release(Vec<mpsc::Sender<()>>);
impl Drop for Release {
    fn drop(&mut self) {
        for sender in self.0.drain(..) {
            let _ = sender.send(());
        }
    }
}

fn basic(mut request: HttpRequest<Body>) -> HttpRequest<Body> {
    request.headers_mut().insert(
        "authorization",
        "Basic Zml4dHVyZTppZ25vcmVk".parse().unwrap(),
    );
    request
}

fn rejected(case: usize) -> HttpRequest<Body> {
    let method = ["CloseStdin", "SendSignal", "Update"][case % 3];
    let mut request = basic(request(method, &json!({})));
    match case {
        0..=2 => {} // Original missing-selector errors for all control routes.
        3 => *request.body_mut() = Body::from("{"),
        4 => {
            request.headers_mut().remove("x-access-token");
        }
        5 => {
            request
                .headers_mut()
                .insert("authorization", "Basic !!!".parse().unwrap());
        }
        6 => {
            request
                .headers_mut()
                .insert("content-encoding", "compress".parse().unwrap());
        }
        _ => unreachable!(),
    }
    request
}

async fn response_bytes(response: axum::response::Response) -> (StatusCode, Vec<u8>) {
    (response.status(), bytes(response).await.to_vec())
}

async fn occupy_ordinary_workers() -> (Release, Vec<tokio::task::JoinHandle<()>>) {
    let mut release = Release(Vec::new());
    let mut occupied = Vec::new();
    for _ in 0..3 {
        let (entered, began) = tokio::sync::oneshot::channel();
        let (sender, gate) = mpsc::channel();
        release.0.push(sender);
        occupied.push(tokio::task::spawn_blocking(move || {
            entered.send(()).unwrap();
            gate.recv_timeout(Duration::from_secs(15)).unwrap();
        }));
        timeout(Duration::from_secs(5), began)
            .await
            .unwrap()
            .unwrap();
    }
    (release, occupied)
}

async fn close_after_inspection(http: &Http, pid: u32) {
    let prepared = timeout(
        Duration::from_secs(1),
        http.prepare(basic(request(
            "CloseStdin",
            &json!({"process":{"pid":pid}}),
        ))),
    )
    .await
    .unwrap()
    .unwrap();
    let Prepared::Ready(ready) = prepared else {
        panic!("control was rejected")
    };
    // A separate task does not inherit the original prepare scope. Ready
    // itself must preserve the selected executor for unary response work.
    let response = timeout(Duration::from_secs(1), tokio::spawn(ready.execute()))
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(bytes(response).await, "{}");
}

#[test]
fn reserved_control_prepare_and_later_execution_survive_full_ordinary_blocking_capacity() {
    let ordinary = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(3)
        .enable_all()
        .build()
        .unwrap();
    let reserved = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    ordinary.block_on(async {
        let fixture = fixture().await;
        let registry = registry();
        let requests = requests(2);
        let controls = super::requests(1);
        let http = http(&fixture, &registry, &requests, &controls)
            .with_control_executor(reserved.handle().clone())
            .unwrap();
        // Compare against these same handlers before blocking, including the
        // original error bytes. The missing selectors cannot affect a command.
        let mut expected = Vec::new();
        for case in 0..7 {
            let response = http
                .handle(rejected(case))
                .await
                .unwrap_or_else(|failure| panic!("baseline case {case}: {failure:?}"));
            expected.push(response_bytes(response).await);
        }
        let mut output = http
            .handle(request(
                "Start",
                &json!({"process":{
            "cmd":"/bin/sh", "args":["-c","printf once >> effect; printf ready; exec /bin/cat"]
        },"stdin":true}),
            ))
            .await
            .unwrap()
            .into_body()
            .into_data_stream();
        let command = started(&mut output, &registry).await;
        let pid = command.0.identity().pid;
        data(&mut output, "stdout", b"ready").await;

        let (release, occupied) = occupy_ordinary_workers().await;
        let mut waiting = Box::pin(http.prepare(request("List", &json!({}))));
        assert!(
            std::future::poll_fn(|cx| Poll::Ready(waiting.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        full(&requests);
        for (case, expected) in expected.into_iter().enumerate() {
            let response = timeout(Duration::from_secs(1), http.handle(rejected(case)))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(response_bytes(response).await, expected, "case {case}");
        }
        close_after_inspection(&http, pid).await;
        assert!(
            std::future::poll_fn(|cx| Poll::Ready(waiting.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        drop(release);
        for job in occupied {
            job.await.unwrap();
        }
        drop(
            timeout(Duration::from_secs(5), waiting)
                .await
                .unwrap()
                .unwrap(),
        );
        end(&mut output).await;
        drop(output);
        clean(&fixture, &registry, command).await;
        assert_eq!(
            std::fs::read(fixture.directory.path().join("effect")).unwrap(),
            b"once"
        );
        assert!(requests.admit().is_ok());
        assert!(controls.admit().is_ok());
    });
}
