use super::*;

async fn hold_fifo(fixture: &Fixture) -> std::fs::File {
    timeout(Duration::from_secs(5), async {
        loop {
            match rustix::fs::open(
                fixture.directory.path().join("finish"),
                rustix::fs::OFlags::WRONLY
                    | rustix::fs::OFlags::NONBLOCK
                    | rustix::fs::OFlags::NOFOLLOW,
                rustix::fs::Mode::empty(),
            ) {
                Ok(fd) => break std::fs::File::from(fd),
                Err(rustix::io::Errno::NXIO) => tokio::task::yield_now().await,
                Err(error) => panic!("original FIFO: {error}"),
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn hung_up_pty_readiness_never_fabricates_a_native_syscall_failure() {
    use std::io::Write;
    let fixture = fixture().await;
    let registry = registry();
    let ordinary = requests(2);
    let control = requests(1);
    let http = http(&fixture, &registry, &ordinary, &control);
    let mut output = http.handle(request("Start", &json!({"process":{
        "cmd":"/bin/sh", "args":["-c","mkfifo finish; printf ready; exec 0<&- 1>&- 2>&-; read done < finish"]
    },"pty":{}}))).await.unwrap().into_body().into_data_stream();
    let command = started(&mut output, &registry).await;
    let pid = command.0.identity().pid;
    data(&mut output, "pty", b"ready").await;
    // The original shell opens this FIFO only after all slave FDs close. Keep
    // it waiting on this same reader while testing the retained master writer.
    let mut finish = hold_fifo(&fixture).await;
    // The retained master can still resize after every slave closes. A write
    // readiness rejection does not establish an ioctl failure or command exit.
    for pty in [json!({}), json!({"size":{"cols":65537,"rows":u32::MAX}})] {
        let response = http
            .handle(request("Update", &json!({"process":{"pid":pid},"pty":pty})))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let first = bytes(response).await;
        assert_eq!(first, "{}");
        let last = first.clone();
        drop(first);
        full(&control);
        rotates(&fixture).await;
        assert!(command.0.status().completion.is_none());
        drop(last);
        timeout(Duration::from_secs(5), async {
            loop {
                if control.admit().is_ok() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
    assert_eq!(
        bytes(
            http.handle(request(
                "SendInput",
                &json!({"process":{"pid":pid},"input":{"pty":""}})
            ))
            .await
            .unwrap()
        )
        .await,
        "{}"
    );
    let failure = http
        .handle(request(
            "SendInput",
            &json!({"process":{"pid":pid},"input":{"pty":STANDARD.encode(b"not-delivered")}}),
        ))
        .await
        .unwrap_err();
    assert!(
        matches!(&failure, Failure::Operation(f) if f.request_error() == Error::Command(CommandError::Unavailable) && f.error() == command_requests::http::reply::Error::UnclassifiedFault)
    );
    full(&ordinary);
    rotates(&fixture).await;
    assert!(registry.live(pid).is_ok());
    drop(failure);
    finish.write_all(b"done\n").unwrap();
    drop(finish);
    end(&mut output).await;
    drop(output);
    clean(&fixture, &registry, command).await;
    assert!(ordinary.admit().is_ok());
    assert!(control.admit().is_ok());
}
