//! A queued persistent writer must produce a known refusal, not close HTTP.
use super::*;

#[test]
fn queued_pid_writer_refuses_start_without_effect_and_recovers_after_actual_exit() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let first = fixture().await;
        let second = fixture().await;
        let registrar = PidRegistrar::new(store(&first));
        let registry = registry();
        let ordinary = requests(1);
        let control = requests(1);
        let http = CommandHttp::new(
            second.session.clone(),
            RequestPools {
                ordinary: ordinary.clone(),
                control,
            },
            accounts(&second),
            settings(),
            processes(&second, &registry, &registrar),
            responses(),
        )
        .unwrap();
        let payload = json!({"process":{"cmd":"/bin/sh", "args":["-c",
            "printf once >> effect; printf ready; exec /bin/cat"]}, "stdin":true});
        let mut input = request("Start", &payload);
        // Resolve the original account before occupying the blocking pool.
        input
            .headers_mut()
            .insert("authorization", "Basic Zml4dHVyZTo=".parse().unwrap());
        let Prepared::Ready(ready) = http.prepare(input).await.unwrap() else {
            panic!("not ready")
        };
        let mut pending = first.pending(first.spec()).await;
        let identity = pending.prepared().await.unwrap();
        let _guard = kernel_guard(identity);
        let mut observer = pending.observer();
        let (occupied, occupied_rx) = tokio::sync::oneshot::channel();
        let (release, gate) = std::sync::mpsc::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            occupied.send(()).unwrap();
            let _ = gate.recv();
        });
        occupied_rx.await.unwrap();
        let mut registering = Box::pin(pending.register_pid(registrar.clone(), binding()));
        assert!(
            std::future::poll_fn(|cx| Poll::Ready(registering.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        // Dropping the waiter must not return the queued writer's slot.
        drop(registering);
        let mut response = Box::pin(ready.execute());
        let polled = std::future::poll_fn(|cx| Poll::Ready(response.as_mut().poll(cx))).await;
        first.assert_no_effect();
        second.assert_no_effect();
        release.send(()).unwrap();
        blocker.await.unwrap();
        let result = match polled {
            Poll::Ready(value) => value,
            Poll::Pending => response.await,
        };
        assert_eq!(stopped(&mut observer).await, CommandError::Cancelled);
        capacity_restored(&first.owners).await;
        capacity_restored(&second.owners).await;
        assert_eq!(first.gate_count(), 0);
        assert_eq!(second.gate_count(), 0);
        assert_eq!(registry.available_slots(), 1);
        // On the original implementation this is an unmapped Storage(Busy)
        // failure, so the real listener closes and the gateway emits 502.
        let response = result.expect("PID writer capacity must be an explicit Start refusal");
        let contract: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../tests/compat/guest-service-contract.json"
        )))
        .unwrap();
        let sample = contract["start_refusals"]
            .as_array()
            .unwrap()
            .iter()
            .find(|sample| sample["id"] == "pid-writer-capacity")
            .unwrap();
        assert_eq!(
            u64::from(response.status().as_u16()),
            sample["status"].as_u64().unwrap()
        );
        assert_eq!(
            response.headers()["content-type"],
            sample["content_type"].as_str().unwrap()
        );
        let body = bytes(response).await;
        assert_eq!(json_frame(&body, 2), sample["end_stream"]);
        let last = body.clone();
        drop(body);
        full(&ordinary);
        drop(last);
        assert!(ordinary.admit().is_ok());
        first.assert_no_effect();
        second.assert_no_effect();
        let mut body = http
            .handle(request("Start", &payload))
            .await
            .unwrap()
            .into_body()
            .into_data_stream();
        let command = started(&mut body, &registry).await;
        data(&mut body, "stdout", b"ready").await;
        command.0.close_input(command.0.identity()).unwrap();
        end(&mut body).await;
        drop(body);
        clean(&second, &registry, command).await;
        assert_eq!(
            std::fs::read(second.directory.path().join("effect")).unwrap(),
            b"once"
        );
    });
}
