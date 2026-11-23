use super::*;
use sandproof_controller_service::application::branching::{
    BranchAccessIdentity, BranchAccessSelector, BranchControlAction, BranchControlRequest,
    BranchControlStatus, BranchProviderReply, BranchProviderRequest, BranchStartFailure,
};
use sandproof_controller_service::application::{
    ApplicationError, ControllerApplication, DataKind,
};
use sandproof_controller_service::application::{HostBranchBatchKey, HostBranchLimits};
use sandproof_protocol::lifecycle::AuthenticatedCenterContext;
use serde_json::{Value, json};

struct Fixture {
    dir: tempfile::TempDir,
    state: Arc<Mutex<local::State>>,
    app: Arc<ControllerApplication<local::TestHost>>,
    lifecycle: HostLifecycle<local::TestHost>,
    request: BranchControlRequest,
    context: AuthenticatedCenterContext,
}

impl Fixture {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let state = Arc::new(Mutex::new(local::state()));
        let app = Self::open(dir.path(), &state, true);
        let lifecycle = HostLifecycle::new(Arc::clone(&app));
        let create = local::request();
        let context = local::context();
        lifecycle
            .record_lifecycle(create.clone(), context.clone())
            .await
            .unwrap();
        lifecycle
            .advance(create.clone(), Some(local::template()), local::compute())
            .await
            .unwrap();
        let parent = local::access(&create);
        let (key, targets) = super::branch_data::branch_plan(&parent);
        Self {
            dir,
            state,
            app,
            lifecycle,
            request: BranchControlRequest {
                parent,
                operation: key.operation,
                batch: key.batch,
                targets: targets.to_vec(),
                action: BranchControlAction::Start {},
            },
            context,
        }
    }

    fn open(
        path: &std::path::Path,
        state: &Arc<Mutex<local::State>>,
        provision: bool,
    ) -> Arc<ControllerApplication<local::TestHost>> {
        let app = local::application(
            &path.join("db"),
            Arc::clone(state),
            Arc::new(local::TestClock(AtomicI64::new(10))),
            provision,
        );
        app.configure_host_branching(HostBranchLimits {
            max_batches: 2,
            max_branches_per_vm: 2,
        })
        .unwrap();
        app
    }

    fn restarted(self) -> Self {
        let Self {
            dir,
            state,
            app,
            lifecycle,
            request,
            context,
        } = self;
        drop(lifecycle);
        drop(app);
        let app = Self::open(dir.path(), &state, false);
        let lifecycle = HostLifecycle::new(Arc::clone(&app));
        Self {
            dir,
            state,
            app,
            lifecycle,
            request,
            context,
        }
    }

    fn route_open(&self, index: usize) -> bool {
        let identity = BranchAccessIdentity {
            parent: self.request.parent.clone(),
            selector: BranchAccessSelector {
                operation: self.request.operation.clone(),
                batch: self.request.batch.clone(),
                branch: self.request.targets[index].sandbox.clone(),
            },
        };
        self.app
            .lifecycle_branch_data_authorization(&identity, &self.context, DataKind::FileRead)
            .is_ok()
    }

    fn release_first(&mut self) {
        self.request.action = BranchControlAction::Release {
            branch: Some(self.request.targets[0].sandbox.clone()),
        };
    }

    async fn exchange(
        &self,
        reply: impl FnOnce(Value) -> Result<Value, ApplicationError> + Send + 'static,
    ) -> Result<BranchControlStatus, sandproof_internal::ReceiveError> {
        let mut reply = Some(reply);
        self.lifecycle
            .drive_branch_control(
                self.request.clone(),
                self.context.clone(),
                |_, _| Ok(Some(())),
                move |(), request| match request {
                    BranchProviderRequest::Guest(bytes) => {
                        reply.take().unwrap()(serde_json::from_slice(bytes).unwrap()).map(|value| {
                            BranchProviderReply::Guest(serde_json::to_vec(&value).unwrap())
                        })
                    }
                    BranchProviderRequest::ConfirmParentExit(_) => {
                        Err(ApplicationError::OutcomeUnknown)
                    }
                },
            )
            .await
            .map(|reply| {
                assert_eq!(reply.original, self.request);
                reply.status
            })
    }
}

fn complete_ready() -> Value {
    json!({
        "batch_id": "batch", "status": "ready", "released": [],
        "branches": [
            {"branch_id": "first", "port": 31000},
            {"branch_id": "sibling", "port": 31001},
        ],
    })
}

fn partial_ready() -> Value {
    json!({
        "batch_id": "batch", "status": "ready", "released": ["first"],
        "branches": [{"branch_id": "sibling", "port": 31001}],
    })
}

#[tokio::test]
async fn exact_capacity_refusal_survives_reopen_without_restarting_work() {
    let fixture = Fixture::new().await;
    assert_eq!(
        fixture
            .exchange(|command| {
                assert_eq!(command["operation"], "start");
                Ok(json!({"batch_id": "old", "status": "never_started"}))
            })
            .await,
        Ok(BranchControlStatus::Pending {}),
    );
    let refusal = json!({"batch_id": "batch", "status": "never_started"});
    let original = refusal.clone();
    assert_eq!(
        fixture.exchange(move |_| Ok(original)).await,
        Ok(BranchControlStatus::Failed {
            reason: BranchStartFailure::GuestCapacity
        })
    );
    assert!(!fixture.route_open(0));
    let fixture = fixture.restarted();
    assert_eq!(
        fixture
            .exchange(move |command| {
                assert_eq!(command["operation"], "observe");
                Ok(refusal)
            })
            .await,
        Ok(BranchControlStatus::Failed {
            reason: BranchStartFailure::GuestCapacity
        })
    );
    assert!(!fixture.route_open(0));
}

#[tokio::test]
async fn observed_batch_loss_closes_both_original_routes() {
    let mut fixture = Fixture::new().await;
    assert_eq!(
        fixture.exchange(|_| Ok(complete_ready())).await,
        Ok(BranchControlStatus::Ready {})
    );
    assert!(fixture.route_open(0) && fixture.route_open(1));
    fixture.request.action = BranchControlAction::Observe {};
    assert_eq!(
        fixture
            .exchange(|_| Ok(json!({"batch_id": "batch", "status": "lost"})))
            .await,
        Ok(BranchControlStatus::Pending {})
    );
    assert_eq!(
        fixture
            .exchange(|_| Ok(json!({"batch_id": "batch", "status": "failed_clean"})))
            .await,
        Ok(BranchControlStatus::Failed {
            reason: BranchStartFailure::GuestSetupFailed
        })
    );
    assert!(!fixture.route_open(0) && !fixture.route_open(1));
}

#[tokio::test]
async fn native_branch_control_start_reuses_plan_and_requires_exact_ready() {
    let mut fixture = Fixture::new().await;
    let (sent, received) = oneshot::channel();
    assert_eq!(
        fixture
            .exchange(|command| {
                assert_eq!(command["operation"], "start");
                assert_eq!(command["plan"]["batch_id"], "batch");
                assert_eq!(command["plan"]["checkpoint_id"], "checkpoint");
                let first = command;
                let secret = first["plan"]["branches"][0]["access_token"]
                    .as_str()
                    .unwrap();
                assert_eq!(secret.len(), 64);
                assert_ne!(secret, first["plan"]["branches"][1]["access_token"]);
                sent.send(first).unwrap();
                Err(ApplicationError::OutcomeUnknown)
            })
            .await,
        Ok(BranchControlStatus::Pending {}),
    );
    assert!(!fixture.route_open(0));
    let original = received.await.unwrap();
    let port = fixture.request.targets[0].port;
    fixture.request.targets[0].port = 31_002.try_into().unwrap();
    assert_eq!(
        fixture
            .exchange(|_| panic!("a changed durable plan reached the guest"))
            .await,
        Err(sandproof_internal::ReceiveError::Unavailable),
    );
    fixture.request.targets[0].port = port;
    for invalid in [
        json!({"batch_id": "old", "status": "failed_clean"}),
        json!({"batch_id": "batch", "status": "rejected"}),
        json!({"batch_id": "batch", "status": "ready", "released": ["other"],
               "branches": [{"branch_id": "first", "port": 31000}]}),
        json!({"batch_id": "batch", "status": "ready", "released": [],
               "branches": [{"branch_id": "first", "port": 31001},
                            {"branch_id": "sibling", "port": 31000}]}),
        partial_ready(),
    ] {
        let original = original.clone();
        assert_eq!(
            fixture
                .exchange(move |command| {
                    assert_eq!(command, original);
                    Ok(invalid)
                })
                .await,
            Ok(BranchControlStatus::Pending {}),
        );
        assert!(!fixture.route_open(0));
    }
    assert_eq!(
        fixture.exchange(|_| Ok(complete_ready())).await,
        Ok(BranchControlStatus::Ready {}),
    );
    assert!(fixture.route_open(0) && fixture.route_open(1));
}

#[tokio::test]
async fn native_branch_control_release_fences_before_exchange_and_waits_for_census() {
    let mut fixture = Fixture::new().await;
    fixture.exchange(|_| Ok(complete_ready())).await.unwrap();
    fixture.release_first();
    let state = Arc::clone(&fixture.state);
    let app = Arc::clone(&fixture.app);
    let identity = BranchAccessIdentity {
        parent: fixture.request.parent.clone(),
        selector: BranchAccessSelector {
            operation: fixture.request.operation.clone(),
            batch: fixture.request.batch.clone(),
            branch: fixture.request.targets[0].sandbox.clone(),
        },
    };
    let context = fixture.context.clone();
    assert_eq!(
        fixture
            .exchange(move |command| {
                assert_eq!(command["operation"], "release");
                assert_eq!(command["branch_id"], "first");
                assert!(
                    app.lifecycle_branch_data_authorization(&identity, &context, DataKind::Exec)
                        .is_err()
                );
                state.lock().unwrap().live_branch_relay = Some(31_000);
                Ok(json!({"batch_id": "batch", "status": "lost"}))
            })
            .await,
        Ok(BranchControlStatus::CleaningUp {}),
    );
    assert!(!fixture.route_open(0) && fixture.route_open(1));
    assert_eq!(
        fixture.exchange(|_| Ok(complete_ready())).await,
        Ok(BranchControlStatus::CleaningUp {}),
    );
    assert_eq!(
        fixture.exchange(|_| Ok(partial_ready())).await,
        Err(sandproof_internal::ReceiveError::Unavailable),
    );
    fixture.state.lock().unwrap().live_branch_relay = None;
    assert_eq!(
        fixture.exchange(|_| Ok(partial_ready())).await,
        Ok(BranchControlStatus::Released {}),
    );

    let mut fixture = fixture.restarted();
    assert!(!fixture.route_open(0) && !fixture.route_open(1));
    fixture.request.action = BranchControlAction::Observe {};
    fixture
        .exchange(|command| {
            assert_eq!(
                command,
                json!({"operation": "observe", "batch_id": "batch"})
            );
            Ok(partial_ready())
        })
        .await
        .unwrap();
    assert!(!fixture.route_open(0) && fixture.route_open(1));
}

#[tokio::test]
async fn native_branch_control_fails_only_after_original_cleanup_or_exact_pin_refusal() {
    let mut fixture = Fixture::new().await;
    let reply = fixture
        .lifecycle
        .drive_branch_control(
            fixture.request.clone(),
            fixture.context.clone(),
            |_, _| Ok(None::<()>),
            |(), _| panic!("a rejected checkpoint must never reach guest control"),
        )
        .await
        .unwrap();
    assert_eq!(
        reply.status,
        BranchControlStatus::Failed {
            reason: BranchStartFailure::CheckpointUnavailable,
        },
    );
    let key = HostBranchBatchKey {
        operation: fixture.request.operation.clone(),
        batch: fixture.request.batch.clone(),
    };
    assert!(
        fixture
            .app
            .pending_host_branch_guest_plan(&fixture.request.parent, &fixture.context, &key)
            .is_err()
    );

    fixture.state.lock().unwrap().live_branch_relay = Some(31_001);
    assert_eq!(
        fixture
            .exchange(|_| Ok(json!({"batch_id": "batch", "status": "failed_clean"})))
            .await,
        Err(sandproof_internal::ReceiveError::Unavailable),
    );
    fixture.state.lock().unwrap().live_branch_relay = None;
    assert_eq!(
        fixture
            .exchange(|command| {
                assert_eq!(command["operation"], "observe");
                Ok(json!({"batch_id": "batch", "status": "failed_clean"}))
            })
            .await,
        Ok(BranchControlStatus::Failed {
            reason: BranchStartFailure::GuestSetupFailed,
        }),
    );

    // Cleanup can race ahead of any Start receipt. Only the original guest's
    // durable terminal tombstone settles the newly reserved local batch.
    fixture.request.operation.sequence = 10.try_into().unwrap();
    fixture.request.batch.batch = "early-close".try_into().unwrap();
    fixture.request.targets[0].sandbox = "new-first".try_into().unwrap();
    fixture.request.targets[1].sandbox = "new-sibling".try_into().unwrap();
    fixture.request.action = BranchControlAction::Release { branch: None };
    assert_eq!(
        fixture
            .exchange(|command| {
                assert_eq!(
                    command,
                    json!({"operation": "release", "batch_id": "early-close"})
                );
                Ok(json!({"batch_id": "early-close", "status": "released"}))
            })
            .await,
        Ok(BranchControlStatus::Released {}),
    );
    fixture.request.action = BranchControlAction::Start {};
    assert_eq!(
        fixture
            .exchange(|command| {
                assert_eq!(
                    command,
                    json!({"operation": "observe", "batch_id": "early-close"})
                );
                Ok(json!({"batch_id": "early-close", "status": "released"}))
            })
            .await,
        Ok(BranchControlStatus::Pending {}),
    );
}

#[tokio::test]
async fn exact_parent_exit_fences_original_native_routes_without_releasing_capacity() {
    let mut fixture = Fixture::new().await;
    assert_eq!(
        fixture.exchange(|_| Ok(complete_ready())).await,
        Ok(BranchControlStatus::Ready {})
    );
    fixture.request.action = BranchControlAction::Observe {};
    assert_eq!(
        fixture
            .exchange(|_| Err(ApplicationError::OutcomeUnknown))
            .await,
        Ok(BranchControlStatus::Pending {}),
    );
    assert!(fixture.route_open(0) && fixture.route_open(1));

    let exact = fixture
        .lifecycle
        .drive_branch_control(
            fixture.request.clone(),
            fixture.context.clone(),
            |_, _| Ok(Some(())),
            |(), request| match request {
                BranchProviderRequest::Guest(_) => Err(ApplicationError::OutcomeUnknown),
                BranchProviderRequest::ConfirmParentExit(source) => {
                    assert!(
                        source.pid != 0
                            && source.start_time_ticks != 0
                            && !source.boot_id.is_empty()
                    );
                    Ok(BranchProviderReply::ParentExited)
                }
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        exact.status,
        BranchControlStatus::ParentExited { .. }
    ));
    assert!(!fixture.route_open(0) && !fixture.route_open(1));

    let durable = fixture.restarted();
    let reply = durable
        .lifecycle
        .drive_branch_control(
            durable.request.clone(),
            durable.context.clone(),
            |_, _| Ok(Some(())),
            |(), _| panic!("persisted exit evidence must replay without invoking the provider"),
        )
        .await
        .unwrap();
    assert_eq!(reply, exact);
    let key = HostBranchBatchKey {
        operation: durable.request.operation.clone(),
        batch: durable.request.batch.clone(),
    };
    assert!(
        durable
            .app
            .change_host_branch_close(
                &durable.request.parent,
                &durable.context,
                &key,
                sandproof_controller_service::application::HostBranchClose::ConfirmGone(None),
            )
            .is_err()
    );
    let mut successor = key;
    successor.operation.sequence = 11.try_into().unwrap();
    successor.batch.batch = "new-batch".try_into().unwrap();
    let mut target = durable.request.targets[0].clone();
    target.sandbox = "new-branch".try_into().unwrap();
    target.port = 31_002.try_into().unwrap();
    assert_eq!(
        durable.app.reserve_host_branch_batch(
            &durable.request.parent,
            &durable.context,
            &successor,
            &[target],
        ),
        Err(ApplicationError::NotFound)
    );
}
