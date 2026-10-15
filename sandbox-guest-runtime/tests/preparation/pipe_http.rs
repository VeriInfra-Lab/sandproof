use super::*;
use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};

const MARKER: &str = "SANDPROOF_PIPE_HTTP_CHILD";

#[tokio::test]
async fn real_pipe_failure_reaches_start_eos_without_publishing_pid_or_losing_capacity() {
    let mut child = tokio::process::Command::new(std::env::current_exe().unwrap());
    child.args(["--exact", "pid_registration::registry::requests::start::http_dispatch::pipes::isolated_http_pipe_failure", "--ignored", "--nocapture"])
        .env(MARKER, "1").kill_on_drop(true);
    let result = timeout(Duration::from_secs(20), child.output())
        .await
        .unwrap()
        .unwrap();
    assert!(
        result.status.success(),
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[tokio::test]
#[ignore = "isolated HTTP subprocess; lowers only its own descriptor limit"]
async fn isolated_http_pipe_failure() {
    assert_eq!(std::env::var(MARKER).unwrap(), "1");
    let fixture = fixture().await;
    let registry = registry();
    let ordinary = requests(1);
    let control = requests(1);
    let http = http(&fixture, &registry, &ordinary, &control);
    let source = std::fs::File::open("/dev/null").unwrap();
    let original = getrlimit(Resource::Nofile);
    assert!(original.current.is_none_or(|n| n >= 64));
    // Warm the original account/encoding worker before limiting this child.
    drop(bytes(http.handle(request("List", &json!({}))).await.unwrap()).await);
    for (stream, available) in [("stdout", 1), ("stderr", 3), ("stdin", 5)] {
        let request = request(
            "Start",
            &json!({"process":{"cmd":"/bin/sh","args":["-c", "printf forbidden > effect"]}}),
        );
        setrlimit(
            Resource::Nofile,
            Rlimit {
                current: Some(64),
                ..original
            },
        )
        .unwrap();
        let mut held = Vec::with_capacity(64);
        for _ in 0..64 {
            match rustix::io::fcntl_dupfd_cloexec(&source, 0) {
                Ok(fd) => held.push(fd),
                Err(rustix::io::Errno::MFILE) => break,
                Err(error) => panic!("isolated descriptor setup: {error}"),
            }
        }
        assert_eq!(
            rustix::io::fcntl_dupfd_cloexec(&source, 0).unwrap_err(),
            rustix::io::Errno::MFILE
        );
        for _ in 0..available {
            drop(held.pop().unwrap());
        }
        let response = http.handle(request).await.unwrap();
        // Restore only this child's original limit before unrelated assertions.
        held.clear();
        setrlimit(Resource::Nofile, original).unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let mut body = response.into_body().into_data_stream();
        let first = frame(&mut body).await;
        assert_eq!(
            json_frame(&first, 2),
            json!({"error":{"code":"internal", "message":format!("error creating {stream} pipe for command '/bin/sh -c printf forbidden > effect': pipe2: too many open files")}})
        );
        let last = first.clone();
        drop(first);
        drop(body);
        full(&ordinary);
        rotates(&fixture).await;
        capacity_restored(&fixture.owners).await;
        registry_capacity(&registry).await;
        assert_eq!(fixture.gate_count(), 0);
        fixture.assert_no_effect();
        drop(last);
        timeout(Duration::from_secs(5), async {
            while ordinary.admit().is_err() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            bytes(
                http.handle(super::request("List", &json!({})))
                    .await
                    .unwrap()
            )
            .await,
            "{}"
        );
    }
}
