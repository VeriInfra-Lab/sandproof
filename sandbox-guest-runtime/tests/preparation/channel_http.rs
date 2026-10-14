use super::*;

#[tokio::test]
async fn channel_failures_keep_original_mode_ownership_and_later_valid_io() {
    for pty in [false, true] {
        let fixture = fixture().await;
        let registry = registry();
        let ordinary = requests(2);
        let control = requests(1);
        let http = http(&fixture, &registry, &ordinary, &control);
        let mut payload = json!({"process":{"cmd":"/bin/sh","args":["-c",if pty {"stty -echo; printf ready; exec /bin/cat"} else {"printf ready; exec /bin/cat"}]},"stdin":true});
        if pty {
            payload["pty"] = json!({"size":{"cols":80,"rows":24}});
        }
        let mut output = http
            .handle(request("Start", &payload))
            .await
            .unwrap()
            .into_body()
            .into_data_stream();
        let command = started(&mut output, &registry).await;
        let pid = command.0.identity().pid;
        data(&mut output, if pty { "pty" } else { "stdout" }, b"ready").await;
        let failures = if pty {
            [
                (
                    "SendInput",
                    json!({"input":{"stdin":STANDARD.encode(b"not-delivered")}}),
                    "internal",
                    "error writing to stdin: tty assigned to process — input should be written to the pty, not the stdin",
                ),
                (
                    "CloseStdin",
                    json!({}),
                    "unknown",
                    "error closing stdin: cannot close stdin for PTY process — send Ctrl+D (0x04) instead",
                ),
            ]
        } else {
            [
                (
                    "SendInput",
                    json!({"input":{"pty":STANDARD.encode(b"not-delivered")}}),
                    "internal",
                    "error writing to tty: tty not assigned to process — input should be written to the stdin, not the tty",
                ),
                (
                    "Update",
                    json!({"pty":{"size":{"cols":100,"rows":30}}}),
                    "internal",
                    "error resizing tty: tty not assigned to process",
                ),
            ]
        };
        for (method, mut input, code, message) in failures {
            input["process"] = json!({"pid":pid});
            let mut probe = request(method, &input);
            probe
                .headers_mut()
                .insert("accept-encoding", "gzip".parse().unwrap());
            let reply = http.handle(probe).await.unwrap();
            assert_eq!(reply.status(), StatusCode::INTERNAL_SERVER_ERROR);
            let bytes = bytes(reply).await;
            assert_eq!(
                serde_json::from_slice::<Value>(&bytes).unwrap(),
                json!({"code":code,"message":message})
            );
            let last = bytes.clone();
            drop(bytes);
            full(if method == "SendInput" {
                &ordinary
            } else {
                &control
            });
            rotates(&fixture).await;
            assert!(registry.live(pid).is_ok());
            drop(last);
        }
        let (method, input) = if pty {
            (
                "SendInput",
                json!({"process":{"pid":pid},"input":{"pty":STANDARD.encode(b"\x04")}}),
            )
        } else {
            ("CloseStdin", json!({"process":{"pid":pid}}))
        };
        assert_eq!(
            bytes(http.handle(request(method, &input)).await.unwrap()).await,
            "{}"
        );
        end(&mut output).await;
        drop(output);
        clean(&fixture, &registry, command).await;
        assert!(ordinary.admit().is_ok());
        assert!(control.admit().is_ok());
    }
}
