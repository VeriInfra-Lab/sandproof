use super::*;

async fn release_fifo(fixture: &Fixture) {
    use std::io::Write;
    timeout(Duration::from_secs(5), async {
        loop {
            match rustix::fs::open(
                fixture.directory.path().join("finish"),
                rustix::fs::OFlags::WRONLY
                    | rustix::fs::OFlags::NONBLOCK
                    | rustix::fs::OFlags::NOFOLLOW,
                rustix::fs::Mode::empty(),
            ) {
                Ok(fd) => {
                    std::fs::File::from(fd).write_all(b"done\n").unwrap();
                    break;
                }
                Err(rustix::io::Errno::NXIO) => tokio::task::yield_now().await,
                Err(error) => panic!("private FIFO: {error}"),
            }
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn disabled_and_closed_stdin_are_observed_without_misclassifying_live_broken_pipes() {
    for mode in ["disabled", "closed", "broken"] {
        let fixture = fixture().await;
        let registry = registry();
        let ordinary = requests(2);
        let control = requests(1);
        let http = http(&fixture, &registry, &ordinary, &control);
        // A private FIFO keeps the same shell alive independently of stdin.
        // The ready output confirms both the FIFO and closed child FD exist.
        let script = if mode == "broken" {
            "exec 0<&-; mkfifo finish; printf ready; read done < finish"
        } else {
            "mkfifo finish; printf ready; read done < finish"
        };
        let mut output = http
            .handle(request(
                "Start",
                &json!({"process":{
                "cmd":"/bin/sh","args":["-c",script]
            },"stdin":mode != "disabled"}),
            ))
            .await
            .unwrap()
            .into_body()
            .into_data_stream();
        let command = started(&mut output, &registry).await;
        let pid = command.0.identity().pid;
        data(&mut output, "stdout", b"ready").await;
        if mode == "closed" {
            for _ in 0..2 {
                assert_eq!(
                    bytes(
                        http.handle(request("CloseStdin", &json!({"process":{"pid":pid}})))
                            .await
                            .unwrap()
                    )
                    .await,
                    "{}"
                );
            }
        }
        for input in [b"".as_slice(), b"not-delivered"] {
            let result = http
                .handle(request(
                    "SendInput",
                    &json!({
                        "process":{"pid":pid},"input":{"stdin":STANDARD.encode(input)}
                    }),
                ))
                .await;
            let reply = result.unwrap();
            assert_eq!(
                reply.status(),
                if mode == "broken" && input.is_empty() {
                    StatusCode::OK
                } else {
                    StatusCode::INTERNAL_SERVER_ERROR
                }
            );
            let bytes = bytes(reply).await;
            assert_eq!(
                serde_json::from_slice::<Value>(&bytes).unwrap(),
                if mode == "broken" && input.is_empty() {
                    json!({})
                } else if mode == "broken" {
                    json!({"code":"internal", "message":format!("error writing to stdin: error writing to stdin of process '{pid}': write |1: broken pipe")})
                } else {
                    json!({
                        "code":"internal",
                        "message":"error writing to stdin: stdin not enabled or closed"
                    })
                }
            );
            let last = bytes.clone();
            drop(bytes);
            full(&ordinary);
            rotates(&fixture).await;
            drop(last);
            assert!(registry.live(pid).is_ok());
        }
        // Legacy callers retain their coarse error, independently of HTTP.
        assert_eq!(
            command.0.write_input(command.0.identity(), b"x").await,
            Err(if mode == "disabled" {
                CommandError::ClosedInput
            } else {
                CommandError::Unavailable
            })
        );
        release_fifo(&fixture).await;
        end(&mut output).await;
        drop(output);
        clean(&fixture, &registry, command).await;
        assert!(ordinary.admit().is_ok());
        assert!(control.admit().is_ok());
    }
}
