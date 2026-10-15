use super::*;

fn launch(fixture: &Fixture, case: u8, pty: bool) -> (Value, String) {
    let cwd = fixture.directory.path().to_str().unwrap();
    let mut payload = json!({"process":{"cmd":"/bin/sh","args":["-c","printf forbidden > effect"],"cwd":cwd,"envs":{}}});
    let process = &mut payload["process"];
    match case {
        0 => process["cmd"] = json!("bad\0cmd"),
        1 => process["args"][1] = json!("printf forbidden > effect\0tail"),
        2 => process["cwd"] = json!(format!("{cwd}/bad\0cwd")),
        3 => process["envs"] = json!({"A\0B":"value"}),
        4 => process["envs"] = json!({"A":"secret\0value"}),
        5..=7 => {
            process["cmd"] = json!("bad\0cmd");
            process["envs"] = json!({"A":"secret\0value"});
            process["cwd"] = json!(match case {
                5 => format!("{cwd}/bad\0cwd"),
                6 => format!("{cwd}/missing"),
                _ => "~other\0".into(),
            });
        }
        _ => unreachable!(),
    }
    let cwd = process["cwd"].as_str().unwrap();
    let message = if case == 6 {
        format!("cwd '{cwd}' does not exist")
    } else if case == 7 {
        "failed to expand path '' for user 'fixture': cannot expand user-specific home dir".into()
    } else {
        let mut command = process["cmd"].as_str().unwrap().to_owned();
        for arg in process["args"].as_array().unwrap() {
            command.push(' ');
            command.push_str(arg.as_str().unwrap());
        }
        let suffix = if case >= 3 {
            "exec: environment variable contains NUL"
        } else {
            "fork/exec /bin/sh: invalid argument"
        };
        if pty {
            format!(
                "error starting pty with command '{command}' in dir '{cwd}' with '65537' cols and '4294967295' rows: {suffix}"
            )
        } else {
            format!("error starting process '{command}': {suffix}")
        }
    };
    if pty {
        payload["pty"] = json!({"size":{"cols":65537,"rows":u32::MAX}});
    }
    (payload, message)
}

#[tokio::test]
async fn original_worker_nul_rejections_preserve_order_no_effects_and_unburned_pid() {
    for pty in [false, true] {
        for case in 0..8 {
            let fixture = fixture().await;
            let registry = registry();
            let ordinary = requests(1);
            let control = requests(1);
            let http = http(&fixture, &registry, &ordinary, &control);
            let claims = fixture.directory.path().join("pid-claims.bin");
            let before = std::fs::read(&claims).unwrap();
            let (payload, message) = launch(&fixture, case, pty);
            let response = http.handle(request("Start", &payload)).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let mut body = response.into_body().into_data_stream();
            let first = frame(&mut body).await;
            assert_eq!(
                json_frame(&first, 2),
                json!({"error":{"code":"invalid_argument","message":message}}),
                "case={case}, pty={pty}"
            );
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
                loop {
                    if ordinary.admit().is_ok() {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            assert_eq!(
                bytes(http.handle(request("List", &json!({}))).await.unwrap()).await,
                "{}"
            );
        }
    }
}
