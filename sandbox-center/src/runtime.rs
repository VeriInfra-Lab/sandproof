//! One center product: shared authority, original SDK parser, registered mTLS
//! hosts and a single bounded background coordinator. No local VM assembly.
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
        candidates: background_candidates,
        policy: inputs.node_observation,
        handle: tokio::runtime::Handle::current(),
        reply_timeout: inputs.application.reply_timeout,
    };
    let application = Arc::new(
        CenterApplication::new(
            authority,
            hosts,
            templates,
            inputs.application,
            &inputs.token_key,
        )
        .map_err(|_| "center application assembly rejected")?,
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
    let mut background_shutdown = shutdown.subscribe();
    tasks.spawn(async move {
        coordinate(
            application,
            background_templates,
            observation,
            inputs.admission,
            inputs.expiry_scan_interval,
            &mut background_shutdown,
        )
        .await
    });
    supervise(tasks, shutdown).await
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
    candidates: artifacts::ProtectedCandidates,
    policy: sandproof_center_service::NodeObservationPolicy,
    handle: tokio::runtime::Handle,
    reply_timeout: Duration,
}

async fn coordinate(
    application: Arc<Application>,
    templates: Templates,
    observation: ObservationWork,
    admission: Arc<RequestAdmission>,
    interval: Duration,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<(), &'static str> {
    while !*shutdown.borrow() {
        let Some(lease) = admission.try_acquire(RequestClass::Background) else {
            return Err("center coordinator budget unavailable");
        };
        let app = Arc::clone(&application);
        let templates = templates.clone();
        let observation = observation.clone();
        tokio::task::spawn_blocking(move || {
            let _lease = lease;
            if observation::refresh(
                &observation.authority,
                &observation.hosts,
                observation.policy,
                &observation.handle,
            )
            .is_err()
            {
                let _ = app.recover_authority();
                return;
            }
            // Cache work is derived from the finite original catalog. Unknown
            // local I/O never fabricates availability or drops a lifecycle.
            let _ = observation
                .candidates
                .prepare_static(observation.reply_timeout);
            if app.reconcile_once().is_err() {
                // Only the original durable authority is recovered. An absent
                // or corrupt block is never reinitialized or cached away.
                let _ = app.recover_authority();
                return;
            }
            if templates.reconcile_once(app.as_ref()).is_ok() {
                let _ = app.retire_terminal_history_once();
            }
        })
        .await
        .map_err(|_| "center coordinator worker failed")?;
        tokio::select! {
            () = wait_shutdown(shutdown) => break,
            () = tokio::time::sleep(interval) => {},
        }
    }
    Ok(())
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
