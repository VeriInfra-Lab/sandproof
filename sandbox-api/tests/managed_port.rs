use std::{collections::BTreeMap, sync::Arc};

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use sandbox_controller_core as core;
use sandproof_api::{
    ApiCredential, ConnectRequest, ControllerPort, CreateRequest, PortError,
    managed::{HostAdapter, ManagedConfig, ManagedPort},
    router,
};
use sandproof_controller_service::{
    AdmissionLimits, ControllerJournal, HostEvidence, NeverCrash, WorkerTransportError,
    finish_admitted_create_v1,
};
use sandproof_protocol::{
    ResourceEvidence, ResourceRef, UnknownReason, WorkAction, WorkOutcome, WorkReceipt,
};
use serde::{Deserialize, Serialize};
use tempfile::TempDir;
use tower::ServiceExt;

const NAMESPACE: &str = "0123456789abcdef";
const KEY_ONE: &str = "tenant-one-test-key-00000000000000001";
const KEY_TWO: &str = "tenant-two-test-key-00000000000000002";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Evidence {
    resource: ResourceRef,
    command: core::Command,
}

impl ResourceEvidence for Evidence {
    fn resource(&self) -> &ResourceRef {
        &self.resource
    }
}

impl HostEvidence for Evidence {
    fn validates(&self, command: &core::Command) -> bool {
        &self.command == command
    }
}

struct FakeHost {
    secrets: BTreeMap<u64, String>,
    free_bytes: u64,
    observed_prepare: bool,
}

impl Default for FakeHost {
    fn default() -> Self {
        Self {
            secrets: BTreeMap::new(),
            free_bytes: 10_000,
            observed_prepare: false,
        }
    }
}

impl HostAdapter for FakeHost {
    type Evidence = Evidence;

    fn available_disk_bytes(&self) -> Result<u64, PortError> {
        Ok(self.free_bytes)
    }

    fn put_secret(&mut self, sandbox: core::SandboxId, token: &str) -> Result<(), PortError> {
        if self.secrets.insert(sandbox.0, token.to_owned()).is_some() {
            return Err(PortError::Conflict);
        }
        Ok(())
    }

    fn has_secret(&self, sandbox: core::SandboxId) -> Result<bool, PortError> {
        Ok(self.secrets.contains_key(&sandbox.0))
    }

    fn token_for(&self, sandbox: core::SandboxId) -> Result<String, PortError> {
        self.secrets.get(&sandbox.0).cloned().ok_or(PortError::Busy)
    }

    fn execute(
        &mut self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<WorkReceipt<Evidence>, WorkerTransportError> {
        Ok(WorkReceipt {
            identity: action.identity.clone(),
            outcome: WorkOutcome::ObservedSuccess(Evidence {
                resource: action.identity.resource.clone(),
                command: action.command.command().clone(),
            }),
        })
    }

    fn observe(&self, action: &WorkAction<core::EffectId>) -> Option<WorkReceipt<Evidence>> {
        if self.observed_prepare
            && matches!(action.command.command(), core::Command::PrepareVm { .. })
        {
            Some(WorkReceipt {
                identity: action.identity.clone(),
                outcome: WorkOutcome::ObservedSuccess(Evidence {
                    resource: action.identity.resource.clone(),
                    command: action.command.command().clone(),
                }),
            })
        } else {
            None
        }
    }
}

fn config() -> ManagedConfig {
    ManagedConfig {
        limits: AdmissionLimits {
            max_active_global: 2,
            max_active_per_tenant: 1,
            max_disk_bytes_global: 4_000,
            max_disk_bytes_per_tenant: 1_000,
            cleanup_reserve_bytes: 1_000,
            max_timeout_secs: 86_400,
        },
        disk_bytes: 1_000,
        file_policy_digest: core::PolicyDigest(1),
        default_timeout_secs: 300,
        cpu_count: 1,
        memory_mb: 512,
        disk_size_mb: 1_024,
        envd_version: "0.9.0".into(),
        domain: "example.invalid".into(),
    }
}

fn setup_with_host(host: FakeHost) -> (TempDir, Arc<ManagedPort<FakeHost>>) {
    let directory = TempDir::new().unwrap();
    let journal =
        ControllerJournal::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    (
        directory,
        Arc::new(ManagedPort::new(journal, host, config())),
    )
}

fn setup() -> (TempDir, Arc<ManagedPort<FakeHost>>) {
    setup_with_host(FakeHost::default())
}

#[test]
fn startup_recovers_admitted_create_before_credential_write() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("controller.db");
    let mut journal = ControllerJournal::open(&path, NAMESPACE).unwrap();
    let sandbox = core::SandboxId(1);
    journal
        .admit_create(
            core::CreateRequest {
                actor: core::Actor::Tenant(core::TenantId(1)),
                owner: core::TenantId(1),
                sandbox,
                workspace: core::FsId(1),
                file_policy: core::FilePolicyRef {
                    digest: core::PolicyDigest(1),
                },
                policy: core::NetworkPolicy::DenyAll,
                operation: core::OperationId(1),
            },
            i64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis(),
            )
            .unwrap(),
            300,
            1_000,
            config().limits,
            &mut NeverCrash,
        )
        .unwrap();
    drop(journal); // crash after admission, before the first credential file exists
    let port = ManagedPort::new(
        ControllerJournal::open(&path, NAMESPACE).unwrap(),
        FakeHost::default(),
        config(),
    );
    let recovery = port.recover_on_startup().unwrap();
    assert_eq!(recovery.receipts_recovered, 0);
    assert_eq!(recovery.workflows_completed, 1);
    assert_eq!(recovery.unresolved_sandboxes, 0);
    assert_eq!(
        port.connect(
            "1",
            &format!("sb-{NAMESPACE}-1"),
            ConnectRequest { timeout: None }
        )
        .unwrap()
        .envd_access_token
        .len(),
        64
    );
}

#[test]
fn startup_does_not_replace_a_missing_credential_after_vm_reservation() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("controller.db");
    let mut journal = ControllerJournal::open(&path, NAMESPACE).unwrap();
    let sandbox = core::SandboxId(1);
    journal
        .admit_create(
            core::CreateRequest {
                actor: core::Actor::Tenant(core::TenantId(1)),
                owner: core::TenantId(1),
                sandbox,
                workspace: core::FsId(1),
                file_policy: core::FilePolicyRef {
                    digest: core::PolicyDigest(1),
                },
                policy: core::NetworkPolicy::DenyAll,
                operation: core::OperationId(1),
            },
            i64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis(),
            )
            .unwrap(),
            300,
            1_000,
            config().limits,
            &mut NeverCrash,
        )
        .unwrap();
    journal
        .apply(
            core::ControllerInput::Scheduler(core::SchedulerInput::ReserveStartup(
                core::OperationKey {
                    sandbox,
                    operation: core::OperationId(1),
                },
            )),
            &mut NeverCrash,
        )
        .unwrap();
    drop(journal);
    let port = ManagedPort::new(
        ControllerJournal::open(&path, NAMESPACE).unwrap(),
        FakeHost::default(),
        config(),
    );
    let recovery = port.recover_on_startup().unwrap();
    assert_eq!(recovery.workflows_completed, 0);
    assert_eq!(recovery.unresolved_sandboxes, 1);
    assert!(matches!(
        port.get("1", &format!("sb-{NAMESPACE}-1")),
        Err(PortError::NotFound)
    ));
}

#[test]
fn startup_reconciles_lost_prepare_receipt_before_resuming_create() {
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("controller.db");
    let mut journal = ControllerJournal::open(&path, NAMESPACE).unwrap();
    let sandbox = core::SandboxId(1);
    journal
        .admit_create(
            core::CreateRequest {
                actor: core::Actor::Tenant(core::TenantId(1)),
                owner: core::TenantId(1),
                sandbox,
                workspace: core::FsId(1),
                file_policy: core::FilePolicyRef {
                    digest: core::PolicyDigest(1),
                },
                policy: core::NetworkPolicy::DenyAll,
                operation: core::OperationId(1),
            },
            i64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis(),
            )
            .unwrap(),
            300,
            1_000,
            config().limits,
            &mut NeverCrash,
        )
        .unwrap();
    let attempted =
        finish_admitted_create_v1(&mut journal, sandbox, core::OperationId(1), &mut |action| {
            if matches!(action.command.command(), core::Command::PrepareVm { .. }) {
                Ok(WorkReceipt::<Evidence> {
                    identity: action.identity.clone(),
                    outcome: WorkOutcome::Unknown {
                        reason: UnknownReason::ObservationUnavailable,
                        last_observation: None,
                    },
                })
            } else {
                FakeHost::default().execute(action)
            }
        });
    assert!(attempted.is_err());
    drop(journal);
    let mut host = FakeHost {
        observed_prepare: true,
        ..FakeHost::default()
    };
    host.put_secret(sandbox, &"a".repeat(64)).unwrap();
    let port = ManagedPort::new(
        ControllerJournal::open(&path, NAMESPACE).unwrap(),
        host,
        config(),
    );
    let recovery = port.recover_on_startup().unwrap();
    assert_eq!(recovery.receipts_recovered, 1);
    assert_eq!(recovery.workflows_completed, 1);
    assert_eq!(recovery.unresolved_sandboxes, 0);
    assert_eq!(
        port.get("1", &format!("sb-{NAMESPACE}-1")).unwrap().state,
        "running"
    );
    let again = port.recover_on_startup().unwrap();
    assert_eq!(again.receipts_recovered, 0);
    assert_eq!(again.workflows_completed, 0);
    assert_eq!(again.unresolved_sandboxes, 0);
}

fn request() -> CreateRequest {
    CreateRequest {
        template_id: "base".into(),
        timeout: Some(300),
        allow_internet_access: Some(false),
        metadata: None,
        env_vars: None,
    }
}

#[test]
fn public_calls_use_the_durable_reducer_and_owner_lifetime_checks() {
    let (_directory, port) = setup();
    let created = port.create("1", request()).unwrap();
    assert_eq!(created.sandbox_id, format!("sb-{NAMESPACE}-1"));
    assert_eq!(created.envd_access_token.len(), 64);
    for kind in [
        sandproof_api::managed::DataKind::Exec,
        sandproof_api::managed::DataKind::FileRead,
        sandproof_api::managed::DataKind::FileWrite,
    ] {
        assert_eq!(
            port.authorize_data(&created.envd_access_token, kind)
                .unwrap()
                .generation,
            1
        );
    }
    assert_eq!(
        port.authorize_data(&"0".repeat(64), sandproof_api::managed::DataKind::Exec),
        Err(PortError::NotFound)
    );
    assert!(matches!(
        port.create("1", request()),
        Err(PortError::QuotaExceeded)
    ));
    assert_eq!(port.get("2", &created.sandbox_id), Err(PortError::NotFound));
    assert_eq!(
        port.connect("2", &created.sandbox_id, ConnectRequest::default()),
        Err(PortError::NotFound)
    );
    assert_eq!(
        port.kill("2", &created.sandbox_id),
        Err(PortError::NotFound)
    );
    assert!(port.list("2", None, None).unwrap().items.is_empty());

    let info = port.get("1", &created.sandbox_id).unwrap();
    assert_eq!(info.state, "running");
    assert!(info.started_at.ends_with('Z'));
    assert!(info.end_at.ends_with('Z'));
    assert_eq!(port.list("1", None, None).unwrap().items, vec![info]);
    assert_eq!(
        port.connect("1", &created.sandbox_id, ConnectRequest { timeout: None })
            .unwrap()
            .envd_access_token,
        created.envd_access_token
    );
    assert_eq!(port.set_timeout("1", &created.sandbox_id, 60), Ok(()));
    assert_eq!(port.kill("1", &created.sandbox_id), Ok(()));
    assert_eq!(
        port.authorize_data(
            &created.envd_access_token,
            sandproof_api::managed::DataKind::Exec
        ),
        Err(PortError::NotFound)
    );
    assert_eq!(port.get("1", &created.sandbox_id), Err(PortError::NotFound));
    assert!(port.list("1", None, None).unwrap().items.is_empty());
}

#[test]
fn physical_disk_shortage_rejects_before_any_admission() {
    let (_directory, port) = setup_with_host(FakeHost {
        free_bytes: 1_999,
        ..FakeHost::default()
    });
    assert_eq!(port.create("1", request()), Err(PortError::QuotaExceeded));
    assert!(port.list("1", None, None).unwrap().items.is_empty());
}

#[test]
fn direct_port_cannot_bypass_public_subset_guards() {
    let (_directory, port) = setup();
    let mut unsupported = request();
    unsupported.env_vars = Some(BTreeMap::from([("SECRET".into(), "x".into())]));
    assert_eq!(port.create("1", unsupported), Err(PortError::Invalid));
    let mut unsupported = request();
    unsupported.allow_internet_access = Some(true);
    assert_eq!(port.create("1", unsupported), Err(PortError::Invalid));
    assert!(port.list("1", None, None).unwrap().items.is_empty());
}

#[test]
fn two_active_sandboxes_page_without_duplicate_or_cross_tenant_leak() {
    let directory = TempDir::new().unwrap();
    let journal =
        ControllerJournal::open(directory.path().join("controller.db"), NAMESPACE).unwrap();
    let mut settings = config();
    settings.limits.max_active_per_tenant = 2;
    settings.limits.max_disk_bytes_per_tenant = 2_000;
    let port = ManagedPort::new(journal, FakeHost::default(), settings);
    let first = port.create("1", request()).unwrap();
    let second = port.create("1", request()).unwrap();
    let page = port.list("1", Some(1), None).unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].sandbox_id, first.sandbox_id);
    let following = port.list("1", Some(1), page.next_token).unwrap();
    assert_eq!(following.items.len(), 1);
    assert_eq!(following.items[0].sandbox_id, second.sandbox_id);
    assert!(following.next_token.is_none());
    assert!(port.list("2", None, None).unwrap().items.is_empty());
    port.kill("1", &first.sandbox_id).unwrap();
    port.kill("1", &second.sandbox_id).unwrap();
}

#[tokio::test]
async fn router_reaches_the_same_port_without_request_selected_identity() {
    let (_directory, port) = setup();
    let app = router(
        port,
        vec![
            ApiCredential {
                tenant: "1".into(),
                secret: KEY_ONE.into(),
            },
            ApiCredential {
                tenant: "2".into(),
                secret: KEY_TWO.into(),
            },
        ],
    )
    .unwrap();
    let create = Request::builder()
        .method("POST")
        .uri("/v2/sandboxes")
        .header("x-api-key", KEY_ONE)
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"templateID":"base","allow_internet_access":false}"#,
        ))
        .unwrap();
    let response = app.clone().oneshot(create).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    let sandbox = body["sandboxID"].as_str().unwrap();
    let foreign = Request::builder()
        .method("GET")
        .uri(format!("/sandboxes/{sandbox}"))
        .header("x-api-key", KEY_TWO)
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.clone().oneshot(foreign).await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
    let kill = Request::builder()
        .method("DELETE")
        .uri(format!("/sandboxes/{sandbox}"))
        .header("x-api-key", KEY_ONE)
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.oneshot(kill).await.unwrap().status(),
        StatusCode::NO_CONTENT
    );
}
