//! Actual Redis and original coordinator; external artifact/VM ports here are
//! controlled. Authenticated original host evidence has separate HTTPS tests.
use super::{Engine, authority, candidate, local_certificate, reply_loss, revision};
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
    lifecycle::{
        AcceptedLifecycleIntent, ClosedLifecycleWindow, ConfirmedLifecycleResult,
        CreateObservation, CreateRefusalReason, CurrentCreateReadiness, CurrentReadyFacts,
        LifecycleOperation, LifecycleWindowClose, RecordedLifecycleIntent,
        TerminalLifecycleObservation, TerminalStatus,
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
        let cleaned = record
            .operations()
            .last()
            .unwrap()
            .terminal()
            .unwrap()
            .clone();
        assert_eq!(cleaned, request.terminal);
        assert_eq!(cleaned.status, TerminalStatus::Cleaned);
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
    capacity_refusal: bool,
    clock: Arc<Time>,
}
impl HostPort for Host {
    fn record_lifecycle(
        &self,
        request: &AcceptedLifecycleIntent,
        _: Duration,
    ) -> Result<RecordedLifecycleIntent, PortError> {
        let state = self.authority.snapshot()?;
        let record = state
            .center
            .lifecycle_session(
                &request.request.binding.key.owner,
                &request.request.binding.session,
            )
            .unwrap();
        assert_eq!(record.operations().last().unwrap().intent(), request);
        let mut view = self.authority.clone();
        let public = sandproof_center_service::AuthorityPort::public_snapshot(&mut view).unwrap();
        let record = public
            .state
            .lifecycle_session(
                &request.request.binding.key.owner,
                &request.request.binding.session,
            )
            .unwrap();
        assert!(!public.visible(record));
        Ok(RecordedLifecycleIntent {
            intent: request.clone(),
            current_session_version: request.accepted_version,
            current: true,
            terminal: matches!(request.request.operation, LifecycleOperation::Destroy {}),
        })
    }
    fn observe_lifecycle_create(
        &self,
        original: &AcceptedLifecycleIntent,
        _: Duration,
    ) -> Result<CreateObservation, PortError> {
        let state = self.authority.snapshot()?;
        let record = state
            .center
            .lifecycle_session(
                &original.request.binding.key.owner,
                &original.request.binding.session,
            )
            .unwrap();
        let current = record.operations().last().unwrap().intent();
        assert_eq!(current, original, "publication does not issue Renew");
        let LifecycleOperation::Create { create, .. } = &original.request.operation else {
            unreachable!()
        };
        let readiness = if self.admitted {
            CurrentCreateReadiness::Observed {
                facts: CurrentReadyFacts {
                    started_at_ms: 1,
                    end_at_ms: 1 + create.timeout_ms,
                    max_end_at_ms: 1 + create.max_lifetime_ms,
                    startup_deadline_ms: create.startup_deadline_ms,
                    observed_at_ms: self.clock.0.load(Ordering::SeqCst),
                },
            }
        } else if self.capacity_refusal {
            CurrentCreateReadiness::Refused {
                reason: CreateRefusalReason::Capacity,
            }
        } else {
            CurrentCreateReadiness::Pending
        };
        Ok(CreateObservation {
            request: original.clone(),
            current_request: current.clone(),
            historical_create_succeeded: self.admitted,
            readiness,
        })
    }
    fn observe_lifecycle_terminal(
        &self,
        original: &AcceptedLifecycleIntent,
        _: Duration,
    ) -> Result<TerminalLifecycleObservation, PortError> {
        assert!(matches!(
            original.request.operation,
            LifecycleOperation::Destroy {}
        ));
        Ok(TerminalLifecycleObservation {
            request: original.clone(),
            status: TerminalStatus::Cleaned,
        })
    }
    fn confirm_lifecycle_terminal(
        &self,
        result: &TerminalLifecycleObservation,
        _: Duration,
    ) -> Result<ConfirmedLifecycleResult, PortError> {
        if self.capacity_refusal {
            return Err(PortError::OutcomeUnknown);
        }
        Ok(ConfirmedLifecycleResult {
            result: result.clone(),
        })
    }
    fn close_lifecycle_window(
        &self,
        request: &LifecycleWindowClose,
        _: Duration,
    ) -> Result<ClosedLifecycleWindow, PortError> {
        Ok(ClosedLifecycleWindow {
            retired_through: request.terminal.request.retention.window,
            terminal: request.terminal.clone(),
        })
    }
}

type Templates = BusinessTemplates<Candidates, Evidence>;
pub(super) fn template_provider(authority: &SharedBusinessAuthority) -> Templates {
    BusinessTemplates::new(
        authority.clone(),
        Candidates(authority.clone()),
        Evidence {
            authority: authority.clone(),
            admitted: false,
        },
    )
    .unwrap()
}
type Application = CenterApplication<SharedBusinessAuthority, Host, Templates>;
pub(super) fn application(
    authority: &SharedBusinessAuthority,
    admitted: bool,
) -> (Templates, Application, Arc<Time>) {
    application_with_refusal(authority, admitted, false)
}
fn application_with_refusal(
    authority: &SharedBusinessAuthority,
    admitted: bool,
    capacity_refusal: bool,
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
    let clock = Arc::new(Time(AtomicU64::new(1)));
    let app = CenterApplication::new(
        authority.clone(),
        Host {
            authority: authority.clone(),
            admitted,
            capacity_refusal,
            clock: Arc::clone(&clock),
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
    .with_clock(clock.clone())
    .with_lifecycle_observation_policy(sandproof_center_service::NodeObservationPolicy {
        health_window_ms: 1000,
        max_clock_skew_ms: 10,
    })
    .unwrap();
    (templates, app, clock)
}

#[test]
fn actual_publication_uses_original_driver_and_queries_aliases_exact_bindings() {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = authority(&fixture, &proxy);
    let (templates, app, clock) = application(&authority, true);
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
    assert!(validation.released());
    assert!(matches!(
        app.get(
            "8",
            validation.operations()[0]
                .intent()
                .request
                .binding
                .session
                .as_protocol_id()
                .as_str()
        ),
        Err(PortError::NotFound)
    ));
    assert_ready_instance_projection(&fixture, &authority, &app, &clock, &tenant, &immutable);
}

fn assert_ready_instance_projection(
    fixture: &Engine,
    authority: &SharedBusinessAuthority,
    app: &Application,
    clock: &Time,
    tenant: &TenantId,
    immutable: &str,
) {
    // A public instance using a newly Ready publication resolves its original
    // binding from the same view too, even after its mutable alias is deleted.
    let mut lifecycle = authority.clone();
    let request = sandproof_center_service::AuthorityPort::create_current(
        &mut lifecycle,
        tenant,
        &super::lifecycle_parameters(),
        super::validation_policy(),
        0,
    )
    .unwrap();
    let LifecycleOperation::Create { create, .. } = &request.request.operation else {
        unreachable!()
    };
    let ready = CreateObservation {
        request: request.clone(),
        current_request: request.clone(),
        historical_create_succeeded: true,
        readiness: CurrentCreateReadiness::Observed {
            facts: CurrentReadyFacts {
                started_at_ms: 1,
                end_at_ms: 1 + create.timeout_ms,
                max_end_at_ms: 1 + create.max_lifetime_ms,
                startup_deadline_ms: create.startup_deadline_ms,
                observed_at_ms: 1,
            },
        },
    };
    sandproof_center_service::AuthorityPort::accept_lifecycle_create_observation(
        &mut lifecycle,
        &ready,
    )
    .unwrap();
    clock.0.store(2, Ordering::SeqCst);
    let before: u64 = fixture.revision().parse().unwrap();
    let detail = app
        .get(
            "8",
            request.request.binding.session.as_protocol_id().as_str(),
        )
        .unwrap();
    assert_eq!(detail.info.template_id, immutable);
    assert_eq!(fixture.revision().parse::<u64>().unwrap() - before, 1);
    assert_eq!(
        app.get(
            "9",
            request.request.binding.session.as_protocol_id().as_str()
        ),
        Err(PortError::NotFound)
    );
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
    assert!(!pending.center.sandboxes()[0].released());
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
    let cleaned = link.settlement.as_ref().unwrap().cleaned.clone();
    clock.0.fetch_add(100, Ordering::SeqCst);
    app.begin_close(&cleaned).unwrap();
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

#[test]
fn actual_capacity_refusal_keeps_original_destroy_and_waits_for_exact_confirmation_before_settlement()
 {
    let fixture = Engine::new();
    let proxy = reply_loss::ReplyLoss::new(fixture.address);
    let authority = authority(&fixture, &proxy);
    let (templates, app, clock) = application_with_refusal(&authority, false, true);
    assert_eq!(
        templates
            .publish(&app, "8", &candidate().candidate_id)
            .unwrap()
            .state,
        PublicationState::Checking
    );
    let pending = authority.snapshot().unwrap();
    assert_eq!(pending.center.reserved(None, None), Some(super::resources(1)));
    assert!(pending.center.sandboxes()[0].operations().last().unwrap().terminal().is_none());
    assert_eq!(pending.center.identity_frontiers(), (1, 2));
    // First progress saved the original Destroy; this background pass drives
    // that same intent. Its lost confirmation must still prevent settlement.
    assert_eq!(templates.reconcile_once(&app), Ok(0));
    let settled = authority.snapshot().unwrap();
    let sandbox = &settled.center.sandboxes()[0];
    assert_eq!(sandbox.operations().len(), 2);
    assert!(matches!(
        sandbox.operations()[0].intent().request.operation,
        LifecycleOperation::Create { .. }
    ));
    assert_eq!(
        sandbox.operations()[0].create_refusal(),
        Some(CreateRefusalReason::Capacity)
    );
    let response = sandbox.operations()[1].terminal().unwrap();
    assert_eq!(response.status, TerminalStatus::Cleaned);
    assert!(sandbox.terminal_confirmation.is_none());
    assert_eq!(
        settled.center.reserved(None, None),
        Some(sandproof_protocol::internal::lifecycle::Resources::ZERO)
    );
    let record = &settled.metadata.registry().records()[0];
    assert!(!record.value().checked);
    assert!(record.value().validation_certificate.is_none());
    let link = record.validation().unwrap();
    assert!(link.local.is_none());
    assert!(link.settlement.is_none());
    assert!(record.value().failure.is_none());

    let mut wrong = response.clone();
    wrong.request.accepted_version += 1;
    assert_eq!(
        authority.confirm_terminal(&ConfirmedLifecycleResult { result: wrong }),
        Err(PortError::Conflict)
    );
    assert!(
        authority.snapshot().unwrap().center.sandboxes()[0]
            .terminal_confirmation
            .is_none()
    );
    // Deliver the late original host acknowledgement through the protected
    // authority entry, not by retrying publication or allocating a new create.
    authority
        .confirm_terminal(&ConfirmedLifecycleResult {
            result: response.clone(),
        })
        .unwrap();
    assert_eq!(templates.reconcile_once(&app).unwrap(), 1);
    let settled = authority.snapshot().unwrap();
    let sandbox = &settled.center.sandboxes()[0];
    assert_eq!(sandbox.operations().len(), 2);
    assert!(matches!(
        sandbox.operations()[0].intent().request.operation,
        LifecycleOperation::Create { .. }
    ));
    assert_eq!(
        sandbox.operations()[0].create_refusal(),
        Some(CreateRefusalReason::Capacity)
    );
    let response = sandbox.operations()[1].terminal().unwrap();
    assert_eq!(response.status, TerminalStatus::Cleaned);
    assert_eq!(sandbox.terminal_confirmation.as_ref(), Some(response));
    let record = &settled.metadata.registry().records()[0];
    assert_eq!(record.value().state, PublicationState::Failed);
    assert_eq!(
        record.value().failure,
        Some(PublicationFailure::ValidationRejected)
    );
    let link = record.validation().unwrap();
    let saved = link.settlement.as_ref().unwrap();
    assert_eq!(&link.create, sandbox.operations()[0].intent());
    assert_eq!(&saved.cleaned, response);
    assert_eq!(saved.admission, ValidationAdmission::NotAdmitted);

    let mut without_confirmation = serde_json::to_value(&settled).unwrap();
    without_confirmation["center"]["sandboxes"][0]["terminal_confirmation"] =
        serde_json::Value::Null;
    let invalid: sandproof_center_store::business::BusinessEnvelope<super::RegistryMetadata> =
        serde_json::from_value(without_confirmation).unwrap();
    assert!(!invalid.is_valid());
    clock.0.fetch_add(100, Ordering::SeqCst);
    app.retire_terminal_history_once().unwrap();
    let retired = authority.snapshot().unwrap();
    assert!(retired.center.sandboxes().is_empty());
    assert!(retired.is_valid());
    assert_eq!(
        retired.metadata.registry().records()[0].value().state,
        PublicationState::Failed
    );
    let mut forged = serde_json::to_value(retired).unwrap();
    forged["metadata"]["registry"]["records"][0]["validation"]["settlement"]["admission"] =
        serde_json::json!("admitted");
    let invalid: sandproof_center_store::business::BusinessEnvelope<super::RegistryMetadata> =
        serde_json::from_value(forged).unwrap();
    assert!(!invalid.is_valid());
}
