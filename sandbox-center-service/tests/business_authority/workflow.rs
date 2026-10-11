//! Actual Redis and original coordinator; external artifact/VM ports here are
//! controlled. Authenticated original host evidence has separate HTTPS tests.
use super::{Engine, authority, candidate, local_certificate, reply_loss, revision, wire};
use sandproof_api::PortError;
use sandproof_center_service::{
    CenterApplication, CenterConfig, HostPort, TemplateProvider,
    business::{
        BusinessTemplates, CandidatePort, PreparationError, ProtectedCandidate,
        SharedBusinessAuthority, ValidationAdmission, ValidationEvidencePort,
    },
};
use sandproof_center_store::ControlState;
use sandproof_controller_service::{
    application::{ApplicationError, Clock},
    template_registry::{
        PublicationFailure, PublicationState, ValidationEvidence, ValidationOutcome,
    },
};
use sandproof_internal::template::ValidationRequest;
use sandproof_protocol::{
    TenantId,
    internal::{
        lifecycle::{Intent, LifecycleFact, LifecycleRequest, LifecycleResponse},
        window::{WindowCloseAck, WindowCloseRequest},
    },
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

#[derive(Clone)]
pub(super) struct Candidates(SharedBusinessAuthority);
impl CandidatePort for Candidates {
    fn verified(&self, tenant: &TenantId, id: &str) -> Result<ProtectedCandidate, PortError> {
        // The controlled verifier, not the public request, owns the descriptor.
        self.0.snapshot()?;
        if tenant.as_str() != "8" || id != candidate().candidate_id {
            return Err(PortError::NotFound);
        }
        ProtectedCandidate::new(candidate(), revision())
    }
    fn prepare(&self, value: &ProtectedCandidate, _: Duration) -> Result<(), PreparationError> {
        // Reentrant fresh access also fails the test if I/O is under the lock.
        let state = self.0.snapshot().map_err(|_| PreparationError::Unknown)?;
        assert_eq!(value.publication, candidate());
        assert_eq!(state.metadata.registry().records().len(), 1);
        super::cache_candidate(&self.0);
        Ok(())
    }
}

#[derive(Clone)]
pub(super) struct Evidence {
    authority: SharedBusinessAuthority,
    admitted: bool,
}
impl ValidationEvidencePort for Evidence {
    fn evidence(
        &self,
        request: &ValidationRequest,
        _: Duration,
    ) -> Result<ValidationEvidence, PortError> {
        let state = self.authority.snapshot()?;
        let record = &state.center.sandboxes()[0];
        let cleaned = record.operations.last().unwrap().response.clone().unwrap();
        assert_eq!(cleaned.identity, request.terminal);
        assert_eq!(cleaned.fact, LifecycleFact::Cleaned);
        let outcome = if self.admitted {
            let (local, certificate) = local_certificate(true);
            ValidationOutcome::Admitted {
                local: Box::new(local),
                certificate: Box::new(certificate),
            }
        } else {
            // This controlled port does not itself prove original inbox
            // non-admission. The real authenticated host producer does that.
            ValidationOutcome::NotAdmitted
        };
        Ok(ValidationEvidence {
            create: request.create.clone(),
            cleaned,
            outcome,
        })
    }
}

pub(super) struct Time(AtomicU64);
impl Clock for Time {
    fn now_ms(&self) -> Result<i64, ApplicationError> {
        i64::try_from(self.0.load(Ordering::SeqCst)).map_err(|_| ApplicationError::Busy)
    }
}
pub(super) struct Host {
    authority: SharedBusinessAuthority,
    admitted: bool,
    observations: AtomicU64,
}
impl HostPort for Host {
    fn send(
        &self,
        request: &LifecycleRequest,
        _: Duration,
    ) -> Result<LifecycleResponse, PortError> {
        let state = self.authority.snapshot()?;
        assert_eq!(
            state.center.sandboxes()[0]
                .operations
                .last()
                .unwrap()
                .request,
            *request
        );
        let mut view = self.authority.clone();
        let public = sandproof_center_service::AuthorityPort::public_snapshot(&mut view).unwrap();
        assert!(!public.visible(&public.state.sandboxes()[0]));
        let fact = match request.intent {
            Intent::Create if self.admitted => LifecycleFact::Ready {
                started_at_ms: 1,
                end_at_ms: 1 + request.create.timeout_ms,
                max_end_at_ms: 1 + request.create.max_lifetime_ms,
            },
            Intent::Create => LifecycleFact::Recorded,
            Intent::Destroy => LifecycleFact::Cleaned,
            Intent::Renew { .. } => panic!("publication does not renew its validation VM"),
        };
        Ok(wire::response(
            request,
            self.observations.fetch_add(1, Ordering::SeqCst) + 1,
            fact,
        ))
    }
    fn confirm(
        &self,
        response: &LifecycleResponse,
        _: Duration,
    ) -> Result<LifecycleResponse, PortError> {
        Ok(response.clone())
    }
    fn close(
        &self,
        request: &WindowCloseRequest,
        _: Duration,
    ) -> Result<WindowCloseAck, PortError> {
        Ok(WindowCloseAck {
            identity: request.identity.clone(),
        })
    }
}
type Templates = BusinessTemplates<Candidates, Evidence>;
type Application = CenterApplication<SharedBusinessAuthority, Host, Templates>;
pub(super) fn application(
    authority: &SharedBusinessAuthority,
    admitted: bool,
) -> (Templates, Application, Arc<Time>) {
    let templates = BusinessTemplates::new(
        authority.clone(),
        Candidates(authority.clone()),
        Evidence {
            authority: authority.clone(),
            admitted,
        },
    )
    .unwrap();
    let clock = Arc::new(Time(AtomicU64::new(0)));
    let app = CenterApplication::new(
        authority.clone(),
        Host {
            authority: authority.clone(),
            admitted,
            observations: AtomicU64::new(0),
        },
        templates.clone(),
        CenterConfig {
            domain: "sandbox.test".into(),
            max_timeout_secs: 300,
            max_lifetime_secs: 300,
            startup_timeout_secs: 1,
            reply_timeout: Duration::from_millis(200),
            poll_interval: Duration::from_millis(1),
            public_ipv4_enabled: false,
        },
        &[7; 32],
    )
    .unwrap()
    .with_clock(clock.clone());
    (templates, app, clock)
}

#[test]
fn actual_publication_uses_original_driver_and_queries_aliases_exact_bindings() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = authority(&fixture, &proxy);
    let (templates, app, _) = application(&authority, true);
    let value = templates
        .publish(&app, "8", &candidate().candidate_id)
        .unwrap();
    assert_eq!(value.state, PublicationState::Ready);
    assert!(value.template.is_some());
    let tenant = "8".try_into().unwrap();
    let immutable = candidate().binding.template_id;
    let accepted = templates.resolve(&tenant, &immutable).unwrap();
    assert_eq!(accepted.revision(), &revision());
    assert_eq!(
        templates.bound(&revision(), &accepted.digest()).unwrap(),
        accepted
    );
    assert!(matches!(
        templates.bound(&revision(), &[0; 32]),
        Err(PortError::NotFound)
    ));
    assert!(matches!(
        templates.get("9", &immutable),
        Err(PortError::NotFound)
    ));
    assert!(matches!(
        templates.resolve(&tenant, "missing"),
        Err(PortError::NotFound)
    ));
    templates
        .set_alias("8", "working", &immutable, None)
        .unwrap();
    assert_eq!(templates.resolve(&tenant, "working").unwrap(), accepted);
    templates
        .set_alias("8", "working", &immutable, None)
        .unwrap(); // original same-target replay is idempotent
    assert!(matches!(
        templates.set_alias("8", "working", &immutable, Some("tpl-missing")),
        Err(PortError::Conflict)
    ));
    assert!(matches!(
        templates.set_alias("8", "base", &immutable, None),
        Err(PortError::Invalid)
    ));
    assert!(matches!(
        templates.set_alias("8", "bad/name", &immutable, None),
        Err(PortError::Invalid)
    ));
    let page = templates.list("8", 1, None).unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].aliases, vec!["working"]);
    assert!(page.next_cursor.is_none());
    assert!(templates.list("9", 1, None).unwrap().items.is_empty());
    templates.delete_alias("8", "working").unwrap();
    assert!(matches!(
        templates.resolve(&tenant, "working"),
        Err(PortError::NotFound)
    ));
    assert_eq!(
        templates.bound(&revision(), &accepted.digest()).unwrap(),
        accepted
    );
    let saved = authority.snapshot().unwrap();
    assert!(saved.is_valid());
    let validation = &saved.center.sandboxes()[0];
    assert!(validation.released);
    assert!(matches!(
        app.get(
            "8",
            validation.operations[0]
                .request
                .identity
                .sandbox_id
                .as_str()
        ),
        Err(PortError::NotFound)
    ));
}

#[test]
fn actual_never_admitted_candidate_cleanup_settles_failed_then_allows_window_retirement() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = authority(&fixture, &proxy);
    let (templates, app, clock) = application(&authority, false);
    assert_eq!(
        templates
            .publish(&app, "8", &candidate().candidate_id)
            .unwrap()
            .state,
        PublicationState::Checking
    );
    let pending = authority.snapshot().unwrap();
    assert!(!pending.center.sandboxes()[0].released);
    assert!(
        pending.metadata.registry().records()[0]
            .validation()
            .unwrap()
            .settlement
            .is_none()
    );
    clock.0.store(1001, Ordering::SeqCst);
    assert_eq!(templates.reconcile_once(&app).unwrap(), 1);
    let settled = authority.snapshot().unwrap();
    let record = &settled.metadata.registry().records()[0];
    assert_eq!(record.value().state, PublicationState::Failed);
    assert_eq!(
        record.value().failure,
        Some(PublicationFailure::ValidationRejected)
    );
    assert!(!record.value().checked);
    assert!(record.value().validation_certificate.is_none());
    let link = record.validation().unwrap();
    assert!(link.local.is_none());
    assert_eq!(
        link.settlement.as_ref().unwrap().admission,
        ValidationAdmission::NotAdmitted
    );
    let node = link.create.node.clone();
    app.begin_drain(&node).unwrap();
    app.begin_close(&node).unwrap();
    app.reconcile_once().unwrap();
    let retired = authority.snapshot().unwrap();
    assert!(retired.center.sandboxes().is_empty());
    assert!(retired.is_valid());
    let mut tampered = serde_json::to_value(&retired).unwrap();
    tampered["metadata"]["registry"]["records"][0]["validation"]["settlement"]["admission"] =
        serde_json::json!("admitted");
    let tampered: sandproof_center_store::business::BusinessEnvelope<super::RegistryMetadata> =
        serde_json::from_value(tampered).unwrap();
    assert!(!tampered.is_valid());
}
