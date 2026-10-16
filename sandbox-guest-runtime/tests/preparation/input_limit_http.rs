use super::*;

#[tokio::test]
async fn input_limit_is_observed_before_writing_and_a_later_valid_input_still_works() {
    let fixture = fixture().await;
    let registry = registry();
    let ordinary = requests(2);
    let control = requests(1);
    let http = http(&fixture, &registry, &ordinary, &control);
    let mut output = http.handle(request("Start", &json!({
        "process":{"cmd":"/bin/sh","args":["-c","printf once >> effect; printf ready; exec /bin/cat"]},
        "stdin":true,
    }))).await.unwrap().into_body().into_data_stream();
    let command = started(&mut output, &registry).await;
    let pid = command.0.identity().pid;
    data(&mut output, "stdout", b"ready").await;
    let rejected = http
        .handle(request(
            "SendInput",
            &json!({
                "process":{"pid":pid},"input":{"stdin":STANDARD.encode(vec![b'X';8193])},
            }),
        ))
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::TOO_MANY_REQUESTS);
    let reply = bytes(rejected).await;
    assert_eq!(
        serde_json::from_slice::<Value>(&reply).unwrap(),
        json!({
            "code":"resource_exhausted","message":"command input exceeds configured byte limit",
        })
    );
    full(&ordinary);
    drop(reply);
    assert!(registry.live(pid).is_ok());
    assert_eq!(
        bytes(
            http.handle(request(
                "SendInput",
                &json!({
                    "process":{"pid":pid},"input":{"stdin":STANDARD.encode(b"only-valid")},
                })
            ))
            .await
            .unwrap()
        )
        .await,
        "{}"
    );
    // The first visible bytes are the later input, never a prefix of the
    // rejected call. The original process/PID was not restarted or adopted.
    data(&mut output, "stdout", b"only-valid").await;
    assert_eq!(
        std::fs::read(fixture.directory.path().join("effect")).unwrap(),
        b"once"
    );
    assert_eq!(
        bytes(
            http.handle(request("CloseStdin", &json!({"process":{"pid":pid}})))
                .await
                .unwrap()
        )
        .await,
        "{}"
    );
    end(&mut output).await;
    drop(output);
    clean(&fixture, &registry, command).await;
    assert!(ordinary.admit().is_ok());
    assert!(control.admit().is_ok());
}
