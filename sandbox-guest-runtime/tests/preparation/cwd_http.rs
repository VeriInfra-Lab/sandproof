use super::*;

#[tokio::test]
async fn missing_cwd_replies_from_original_preparation_without_start_or_effects() {
    for dangling in [false, true] {
        let fixture = fixture().await;
        let registry = registry();
        let ordinary = requests(1);
        let control = requests(1);
        let http = http(&fixture, &registry, &ordinary, &control);
        let cwd = fixture.directory.path().join("missing-'\"<>&");
        if dangling {
            std::os::unix::fs::symlink("absent-target", &cwd).unwrap();
        }
        let response = http
            .handle(request(
                "Start",
                &json!({"process":{
                    "cmd":"/bin/sh", "args":["-c","printf forbidden > effect"], "cwd":cwd,
                }}),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let mut body = response.into_body().into_data_stream();
        let first = frame(&mut body).await;
        assert_eq!(
            json_frame(&first, 2),
            json!({"error":{
                "code":"invalid_argument", "message":format!("cwd '{}' does not exist",cwd.display())
            }})
        );
        let copy = first.clone();
        drop(first);
        full(&ordinary);
        rotates(&fixture).await;
        drop(body);
        full(&ordinary);
        drop(copy);
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
        capacity_restored(&fixture.owners).await;
        registry_capacity(&registry).await;
        assert_eq!(fixture.gate_count(), 0);
        fixture.assert_no_effect();
        assert_eq!(
            bytes(http.handle(request("List", &json!({}))).await.unwrap()).await,
            "{}"
        );
    }
}
