use super::*;

#[tokio::test]
async fn absent_empty_and_missing_user_commands_emit_start_then_actual_shell_exit() {
    for case in 0..4 {
        let fixture = fixture().await;
        let registry = registry();
        let ordinary = requests(2);
        let control = requests(1);
        let http = http(&fixture, &registry, &ordinary, &control);
        let payload = match case {
            0 => json!({}),
            1 => json!({"process":null}),
            2 => json!({"process":{}}),
            _ => json!({"process":{"cmd":fixture.directory.path().join("missing-program")}}),
        };
        let mut body = http
            .handle(request("Start", &payload))
            .await
            .unwrap()
            .into_body()
            .into_data_stream();
        let command = started(&mut body, &registry).await;
        let mut stderr = Vec::new();
        let exit_code;
        loop {
            let event = json_frame(&frame(&mut body).await, 0);
            let event = &event["event"];
            if let Some(end) = event.get("end") {
                exit_code = end["exitCode"].as_i64().unwrap();
                assert!((1..=255).contains(&exit_code));
                if case == 3 {
                    assert_eq!(exit_code, 127);
                }
                assert_eq!(end["exited"], true);
                let status = format!("exit status {exit_code}");
                assert_eq!(end["status"], status);
                assert_eq!(end["error"], status);
                break;
            }
            if event.get("keepalive").is_some() {
                continue;
            }
            let data = &event["data"];
            assert_eq!(data.as_object().unwrap().len(), 1);
            stderr.extend(STANDARD.decode(data["stderr"].as_str().unwrap()).unwrap());
            assert!(stderr.len() <= 4096);
        }
        assert!(!stderr.is_empty());
        assert_eq!(json_frame(&frame(&mut body).await, 2), json!({}));
        assert!(
            timeout(Duration::from_secs(5), body.next())
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(i64::from(command.0.wait().await.unwrap().code), exit_code);
        drop(body);
        clean(&fixture, &registry, command).await;
        fixture.assert_no_effect();
        assert!(ordinary.admit().is_ok());
        assert!(control.admit().is_ok());
    }
}
