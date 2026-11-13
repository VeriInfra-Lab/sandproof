//! One center product: shared authority, original SDK parser, registered mTLS
//! hosts and separate finite lifecycle/material workers. No local VM assembly.
use crate::{
    artifacts,
    entry::{Envelope, registration_matches},
    observation,
};
use sandproof_api::admission::{RequestAdmission, RequestClass};
use sandproof_center_service::{
    CenterApplication, RegisteredHosts,
    business::{BusinessTemplates, SharedBusinessAuthority},
    deployment::CenterAssemblyInputs,
};
use sandproof_center_store::{RedisLedger, WriterLease, authority::CenterAuthority};
use sandproof_data_transport::GatewayListener;
use std::{sync::Arc, time::Duration};
use tokio::{sync::watch, task::JoinSet};

type Templates = BusinessTemplates<artifacts::ProtectedCandidates, RegisteredHosts>;
type Application = CenterApplication<SharedBusinessAuthority, RegisteredHosts, Templates>;

pub async fn serve(inputs: CenterAssemblyInputs) -> Result<(), &'static str> {
    let lease = WriterLease::acquire(
        &inputs.writer_root,
        inputs.provisioning.center.deployment().clone(),
    )
    .map_err(|_| "center writer lease unavailable")?;
    let (ledger, original) =
        RedisLedger::<Envelope>::open(inputs.redis_endpoint, inputs.store, lease)
            .map_err(|_| "original center authority unavailable")?;
    if !registration_matches(original.state(), &inputs.provisioning) {
        return Err("original center registration does not match protected configuration");
    }
    let authority = SharedBusinessAuthority::new(
        CenterAuthority::attach(ledger)
            .map_err(|_| "original center authority could not be durably confirmed")?,
    );
    let hosts = RegisteredHosts::new(inputs.hosts, tokio::runtime::Handle::current())
        .map_err(|_| "registered host transport rejected")?;
    let prepared = artifacts::assemble(
        authority.clone(),
        hosts.clone(),
        inputs.artifacts,
        tokio::runtime::Handle::current(),
    )
    .map_err(|_| "protected artifact service rejected")?;
    let background_candidates = prepared.candidates.clone();
    let templates = BusinessTemplates::new(authority.clone(), prepared.candidates, hosts.clone())
        .map_err(|_| "template authority assembly rejected")?;
    let background_templates = templates.clone();
    let observation = ObservationWork {
        authority: authority.clone(),
        hosts: hosts.clone(),
        policy: inputs.node_observation,
        handle: tokio::runtime::Handle::current(),
    };
    let material_deadline = inputs.application.reply_timeout;
    let application = Arc::new(
        CenterApplication::new(
            authority,
            hosts,
            templates,
            inputs.application,
            &inputs.token_key,
        )
        .map_err(|_| "center application assembly rejected")?
        .with_lifecycle_observation_policy(inputs.node_observation)
        .map_err(|_| "center lifecycle observation policy rejected")?,
    );
    let boundaries = crate::boundaries::bind(
        Arc::clone(&application),
        inputs.bind,
        inputs.credentials,
        inputs.private_build_boundary,
        Arc::clone(&inputs.admission),
        inputs.transfers.clone(),
    )
    .await?;
    let artifact_listener = prepared.service.listen().await?;
    let (shutdown, observed_shutdown) = watch::channel(false);
    let mut tasks = JoinSet::new();
    let write_idle = Duration::from_secs(inputs.transfers.write_idle_seconds);
    tasks.spawn(serve_listener(
        boundaries.public,
        shutdown.subscribe(),
        write_idle,
        "public center listener failed",
    ));
    tasks.spawn(async move {
        artifact_listener
            .serve(observed_shutdown.clone())
            .await
            .map_err(|_| "protected artifact listener failed")
    });
    if let Some(private) = boundaries.private {
        tasks.spawn(serve_listener(
            private,
            shutdown.subscribe(),
            write_idle,
            "private center listener failed",
        ));
    }
    spawn_backgrounds(
        &mut tasks,
        BackgroundWork {
            application,
            templates: background_templates,
            candidates: background_candidates,
            admission: inputs.admission,
            deadline: material_deadline,
            interval: inputs.expiry_scan_interval,
        },
        observation,
        &shutdown,
    );
    supervise(tasks, shutdown).await
}

struct BackgroundWork {
    application: Arc<Application>,
    templates: Templates,
    candidates: artifacts::ProtectedCandidates,
    admission: Arc<RequestAdmission>,
    deadline: Duration,
    interval: Duration,
}

fn spawn_backgrounds(
    tasks: &mut JoinSet<Result<(), &'static str>>,
    work: BackgroundWork,
    observation: ObservationWork,
    shutdown: &watch::Sender<bool>,
) {
    let mut background_shutdown = shutdown.subscribe();
    let mut material_shutdown = shutdown.subscribe();
    let material_application = Arc::clone(&work.application);
    let material_admission = Arc::clone(&work.admission);
    tasks.spawn(async move {
        coordinate_material(
            material_application,
            work.templates,
            work.candidates,
            material_admission,
            work.deadline,
            work.interval,
            &mut material_shutdown,
        )
        .await
    });
    tasks.spawn(async move {
        coordinate(
            work.application,
            observation,
            work.admission,
            work.interval,
            &mut background_shutdown,
        )
        .await
    });
}

async fn supervise(
    mut tasks: JoinSet<Result<(), &'static str>>,
    shutdown: watch::Sender<bool>,
) -> Result<(), &'static str> {
    let mut result = tokio::select! {
        result = termination_signal() => result,
        stopped = tasks.join_next() => match stopped {
            Some(Ok(Err(error))) => Err(error),
            _ => Err("center service stopped unexpectedly"),
        },
    };
    let _ = shutdown.send(true);
    // Closing center channels never claims VM termination. Accepted operations
    // remain in the original authority and the host continues its own expiry.
    while let Some(stopped) = tasks.join_next().await {
        let completion = match stopped {
            Ok(completion) => completion,
            Err(_) => Err("center worker stopped abnormally"),
        };
        result = result.and(completion);
    }
    result
}

async fn serve_listener(
    listener: crate::boundaries::Listener,
    mut shutdown: watch::Receiver<bool>,
    write_idle: Duration,
    failure: &'static str,
) -> Result<(), &'static str> {
    axum::serve(
        GatewayListener::new(listener.socket, Some(write_idle)),
        listener.router,
    )
    .with_graceful_shutdown(async move {
        wait_shutdown(&mut shutdown).await;
    })
    .await
    .map_err(|_| failure)
}

#[derive(Clone)]
struct ObservationWork {
    authority: SharedBusinessAuthority,
    hosts: RegisteredHosts,
    policy: sandproof_center_service::NodeObservationPolicy,
    handle: tokio::runtime::Handle,
}

async fn coordinate(
    application: Arc<Application>,
    observation: ObservationWork,
    admission: Arc<RequestAdmission>,
    interval: Duration,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<(), &'static str> {
    while !*shutdown.borrow() {
        let app = Arc::clone(&application);
        let observation = observation.clone();
        let keep_running = observation_round(&admission, interval, shutdown, move || {
            app.lifecycle_background_round(|| {
                observation::refresh(
                    &observation.authority,
                    &observation.hosts,
                    observation.policy,
                    &observation.handle,
                )
            });
        })
        .await?;
        if !keep_running {
            break;
        }
    }
    Ok(())
}

async fn observation_round(
    admission: &RequestAdmission,
    interval: Duration,
    shutdown: &mut watch::Receiver<bool>,
    work: impl FnOnce() + Send + 'static,
) -> Result<bool, &'static str> {
    // The configured scan interval starts with the round, not after its I/O.
    // A slow original status call must not consume most of its health window
    // and then wait another full interval. There is only one actual worker:
    // an overdue round proceeds once, without queuing missed intervals.
    let next = tokio::time::Instant::now() + interval;
    blocking_round(admission, RequestClass::Background, work).await?;
    tokio::select! {
        biased;
        () = wait_shutdown(shutdown) => Ok(false),
        () = tokio::time::sleep_until(next) => Ok(true),
    }
}

/// Material I/O is serial and finite within its own reserved slot. Work is
/// derived from the original catalog/Checking records, never another queue.
/// Cancellation retains the worker's slot until its actual work completes.
async fn coordinate_material(
    application: Arc<Application>,
    templates: Templates,
    candidates: artifacts::ProtectedCandidates,
    admission: Arc<RequestAdmission>,
    deadline: Duration,
    interval: Duration,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<(), &'static str> {
    while !*shutdown.borrow() {
        let app = Arc::clone(&application);
        let templates = templates.clone();
        let candidates = candidates.clone();
        blocking_round(&admission, RequestClass::Material, move || {
            let _scope = sandproof_center_store::DiagnosticScope::enter(
                sandproof_center_store::DiagnosticFlow::Background,
                sandproof_center_store::DiagnosticStep::Material,
            );
            let _ = candidates.prepare_static(deadline);
            let _ = templates.reconcile_once(app.as_ref());
        })
        .await?;
        tokio::select! {
            () = wait_shutdown(shutdown) => break,
            () = tokio::time::sleep(interval) => {},
        }
    }
    Ok(())
}

async fn blocking_round(
    admission: &RequestAdmission,
    class: RequestClass,
    work: impl FnOnce() + Send + 'static,
) -> Result<(), &'static str> {
    let lease = admission
        .try_acquire(class)
        .ok_or("center background budget unavailable")?;
    tokio::task::spawn_blocking(move || {
        let _lease = lease;
        work();
    })
    .await
    .map_err(|_| "center background worker failed")
}

async fn wait_shutdown(shutdown: &mut watch::Receiver<bool>) {
    while !*shutdown.borrow() {
        if shutdown.changed().await.is_err() {
            break;
        }
    }
}

async fn termination_signal() -> Result<(), &'static str> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|_| "center shutdown signal unavailable")?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result.map_err(|_| "center shutdown signal unavailable"),
        _ = terminate.recv() => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn assert_observation_schedule(work_seconds: u64) {
        let admission = Arc::new(RequestAdmission::default());
        let (shutdown, mut receiver) = watch::channel(false);
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let budget = Arc::clone(&admission);
        let before = tokio::time::Instant::now();
        let task = tokio::spawn(async move {
            observation_round(&budget, Duration::from_secs(2), &mut receiver, move || {
                started.send(()).unwrap();
                wait.recv().unwrap();
            })
            .await
            .unwrap()
        });
        ready.await.unwrap();
        assert!(admission.try_acquire(RequestClass::Background).is_none());
        tokio::time::advance(Duration::from_secs(work_seconds)).await;
        release.send(()).unwrap();
        assert!(task.await.unwrap());
        assert_eq!(before.elapsed(), Duration::from_secs(work_seconds.max(2)));
        assert!(admission.try_acquire(RequestClass::Background).is_some());
        // A subsequent short round waits one original interval. Missed time
        // cannot create a catch-up queue, parallel round, or burst of replays.
        let before = tokio::time::Instant::now();
        assert!(
            observation_round(
                &admission,
                Duration::from_secs(2),
                &mut shutdown.subscribe(),
                || {}
            )
            .await
            .unwrap()
        );
        assert_eq!(before.elapsed(), Duration::from_secs(2));
    }

    #[tokio::test(start_paused = true)]
    async fn short_observation_work_counts_toward_the_original_scan_interval() {
        assert_observation_schedule(1).await;
    }

    #[tokio::test(start_paused = true)]
    async fn slow_observation_round_does_not_add_another_interval_or_replay_missed_rounds() {
        assert_observation_schedule(6).await;
    }

    #[tokio::test(start_paused = true)]
    async fn shutdown_waits_for_original_observation_work_without_releasing_its_slot() {
        let admission = Arc::new(RequestAdmission::default());
        let (shutdown, mut receiver) = watch::channel(false);
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let budget = Arc::clone(&admission);
        let task = tokio::spawn(async move {
            observation_round(&budget, Duration::from_secs(2), &mut receiver, move || {
                started.send(()).unwrap();
                wait.recv().unwrap();
            })
            .await
            .unwrap()
        });
        ready.await.unwrap();
        tokio::time::advance(Duration::from_secs(6)).await;
        shutdown.send(true).unwrap();
        tokio::task::yield_now().await;
        assert!(!task.is_finished());
        assert!(admission.try_acquire(RequestClass::Background).is_none());
        release.send(()).unwrap();
        assert!(!task.await.unwrap());
        assert!(admission.try_acquire(RequestClass::Background).is_some());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn blocked_material_does_not_block_lifecycle_and_cancel_does_not_release_its_slot() {
        let admission = Arc::new(RequestAdmission::default());
        let (started, observed) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let (finished, completion) = tokio::sync::oneshot::channel();
        let budget = Arc::clone(&admission);
        let material = tokio::spawn(async move {
            blocking_round(&budget, RequestClass::Material, move || {
                started.send(()).unwrap();
                wait.recv().unwrap();
                finished.send(()).unwrap();
            })
            .await
        });
        observed.await.unwrap();
        material.abort();
        let _ = material.await;
        assert!(admission.try_acquire(RequestClass::Material).is_none());
        // Exact production dispatch helper while material is still running.
        blocking_round(&admission, RequestClass::Background, || {})
            .await
            .unwrap();
        assert!(admission.try_acquire(RequestClass::Control).is_some());
        release.send(()).unwrap();
        completion.await.unwrap();
    }
}
