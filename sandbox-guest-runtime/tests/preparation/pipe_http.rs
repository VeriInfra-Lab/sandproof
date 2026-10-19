use super::*;
use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};

const MARKER: &str = "SANDPROOF_PIPE_HTTP_CHILD";

#[tokio::test]
async fn real_pipe_failure_reaches_start_eos_without_publishing_pid_or_losing_capacity() {
    isolated("pipe").await;
}

#[tokio::test]
async fn real_pty_master_failure_reaches_start_eos_without_pid_effect_or_lost_capacity() {
    isolated("pty").await;
}

#[tokio::test]
async fn missing_cwd_precedes_real_pipe_and_pty_resource_failures() {
    for mode in ["cwd-pipe", "cwd-pty"] {
        isolated(mode).await;
    }
}

async fn isolated(mode: &str) {
    let mut child = tokio::process::Command::new(std::env::current_exe().unwrap());
    child.args(["--exact", "pid_registration::registry::requests::start::http_dispatch::pipes::isolated_http_pipe_failure", "--ignored", "--nocapture"])
        .env(MARKER, mode).kill_on_drop(true);
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
    let (pty, missing_cwd) = match std::env::var(MARKER).unwrap().as_str() {
        "pty" => (true, false),
        "pipe" => (false, false),
        "cwd-pty" => (true, true),
        "cwd-pipe" => (false, true),
        _ => panic!("private child mode required"),
    };
    let fixture = fixture().await;
    let registry = registry();
    let ordinary = requests(1);
    let control = requests(1);
    let http = http(&fixture, &registry, &ordinary, &control);
    let claims = fixture.directory.path().join("pid-claims.bin");
    let before = std::fs::read(&claims).unwrap();
    let cwd = fixture.directory.path().join("missing-'\"<>&");
    let source = std::fs::File::open("/dev/null").unwrap();
    let original = getrlimit(Resource::Nofile);
    assert!(original.current.is_none_or(|n| n >= 64));
    // Warm the original account/encoding worker before limiting this child.
    drop(bytes(http.handle(request("List", &json!({}))).await.unwrap()).await);
    let cases = if pty {
        &[("pty", 1)][..]
    } else {
        &[("stdout", 1), ("stderr", 3), ("stdin", 5)][..]
    };
    for &(stream, available) in cases {
        let mut input =
            json!({"process":{"cmd":"/bin/sh","args":["-c", "printf forbidden > effect"]}});
        if pty {
            input["pty"] = json!({"size":{"cols":65537,"rows":u32::MAX}});
            input["process"]["cwd"] = json!(fixture.directory.path());
        }
        if missing_cwd {
            input["process"]["cwd"] = json!(cwd);
        }
        let request = request("Start", &input);
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
        let expected = if missing_cwd {
            json!({"error":{"code":"invalid_argument", "message":format!("cwd '{}' does not exist", cwd.display())}})
        } else if pty {
            json!({"error":{"code":"resource_exhausted", "message":format!("error starting pty with command '/bin/sh -c printf forbidden > effect' in dir '{}' with '65537' cols and '4294967295' rows: open /dev/ptmx: too many open files", fixture.directory.path().display())}})
        } else {
            json!({"error":{"code":"internal", "message":format!("error creating {stream} pipe for command '/bin/sh -c printf forbidden > effect': pipe2: too many open files")}})
        };
        assert_eq!(json_frame(&first, 2), expected);
        let last = first.clone();
        drop(first);
        drop(body);
        full(&ordinary);
        rotates(&fixture).await;
        capacity_restored(&fixture.owners).await;
        registry_capacity(&registry).await;
        assert_eq!(fixture.gate_count(), 0);
        assert_eq!(std::fs::read(&claims).unwrap(), before);
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
