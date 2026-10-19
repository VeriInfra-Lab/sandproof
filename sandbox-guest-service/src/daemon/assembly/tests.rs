use super::*;
use std::{future::Future, sync::mpsc, task::Poll, time::Duration};

#[test]
fn reserving_control_execution_preserves_all_configured_account_admissions() {
    let ordinary = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let control = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    let mut value: serde_json::Value =
        serde_json::from_str(include_str!("../tests/configuration.json")).unwrap();
    value["paths"]["owner_uid"] = serde_json::json!(startup_owner());
    value["accounts"]["in_flight"] = serde_json::json!(4);
    let configuration = Configuration::decode(&serde_json::to_vec(&value).unwrap()).unwrap();
    let prepared = Prepared::new(configuration)
        .unwrap()
        .with_control_executor(control.handle().clone())
        .unwrap();
    ordinary.block_on(async {
        let (entered, began) = tokio::sync::oneshot::channel();
        let (release, gate) = mpsc::channel();
        let held = tokio::task::spawn_blocking(move || {
            entered.send(()).unwrap();
            // Dropping the sender on an assertion failure also releases the worker.
            let _ = gate.recv_timeout(Duration::from_secs(10));
        });
        tokio::time::timeout(Duration::from_secs(5), began)
            .await
            .unwrap()
            .unwrap();
        let mut admitted = Vec::new();
        for index in 0..4 {
            let mut lookup = Box::pin(prepared.accounts.lookup("root".into()));
            let state = std::future::poll_fn(|cx| Poll::Ready(lookup.as_mut().poll(cx))).await;
            assert!(
                state.is_pending(),
                "configured lookup {index} was refused: {state:?}"
            );
            admitted.push(lookup);
        }
        assert_eq!(
            prepared.accounts.lookup("root".into()).await,
            Err(crate::ContextError::ResourceExhausted)
        );
        release.send(()).unwrap();
        held.await.unwrap();
        for lookup in admitted {
            assert_eq!(lookup.await.unwrap().name, "root");
        }
        assert_eq!(
            prepared.accounts.lookup("root".into()).await.unwrap().name,
            "root"
        );
    });
}

fn startup_owner() -> u32 {
    sandproof_guest_runtime::startup_files::current_owner()
}
