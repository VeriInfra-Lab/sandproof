//! Public lifecycle decisions remain in the original complete business CAS.
use super::{
    DiagnosticFailure, DiagnosticStage, Envelope, SharedBusinessAuthority, diagnostics,
    to_authority,
};
use crate::business::LifecycleQuery;
use crate::{LifecyclePolicy, application::authority_error};
use sandproof_api::PortError;
use sandproof_center_core::{CenterState, DecisionError, LifecycleSubmission, SessionPhase};
use sandproof_center_store::authority::AuthorityState;
use sandproof_controller_service::{
    CreateOptions,
    application::{CreateCommand, selected_policy},
};
use sandproof_protocol::{
    TenantId,
    lifecycle::{
        AcceptedLifecycleIntent, CreateObservation, CurrentCreateReadiness, LifecycleOperation,
        OperationKey, PublicCreateRequest, PublicDestroyRequest, PublicResumeRequest,
        PublicSessionPhase, PublicSessionView, PublicUnloadRequest, SessionId,
    },
};

impl SharedBusinessAuthority {
    /// One confirmed envelope supplies lookup, expiry and the Unloaded view.
    /// The actual shared authority mutex stays held throughout; a pure no-op
    /// needs no second confirmation. A changed expiry is committed before its
    /// view escapes. Other phases retain the existing detached observation path.
    pub(crate) fn begin_lifecycle_query(
        &self,
        owner: &TenantId,
        session: &SessionId,
        now: impl FnOnce() -> Result<u64, PortError>,
    ) -> Result<LifecycleQuery, PortError> {
        let mut authority = self.0.lock().map_err(|_| {
            diagnostics::emit(
                DiagnosticStage::AuthoritySnapshotLock,
                DiagnosticFailure::Port(PortError::OutcomeUnknown),
            );
            PortError::OutcomeUnknown
        })?;
        let current = authority
            .snapshot_envelope()
            .inspect_err(|error| {
                diagnostics::emit(
                    DiagnosticStage::AuthoritySnapshot,
                    DiagnosticFailure::Authority(*error),
                );
            })
            .map_err(authority_error)?;
        let record = public_record(&current, owner, session)?;
        if record.phase() != SessionPhase::Unloaded {
            return Ok(LifecycleQuery::Current {
                intent: record
                    .operations()
                    .last()
                    .ok_or(PortError::OutcomeUnknown)?
                    .intent()
                    .clone(),
                phase: record.phase(),
            });
        }
        let _expiry = sandproof_center_store::DiagnosticScope::step(
            sandproof_center_store::DiagnosticStep::Expiry,
        );
        let view = (|| {
            let next = current
                .center
                .terminate_expired(now()?)
                .map_err(|error| authority_error(error.into()))?;
            if next == current.center {
                return public_view(record);
            }
            authority
                .decide(|confirmed| {
                    // The same inner mutex has remained held since confirmation.
                    let updated = confirmed.with_center(next);
                    let record = public_record(&updated, owner, session).map_err(to_authority)?;
                    let view = public_view(record).map_err(to_authority)?;
                    Ok((updated, view))
                })
                .map_err(authority_error)
        })()
        .inspect_err(|error| {
            diagnostics::emit(
                DiagnosticStage::LifecycleQueryExpiry,
                DiagnosticFailure::Port(*error),
            );
        })?;
        Ok(LifecycleQuery::Ready(view))
    }

    pub(crate) fn public_lifecycle_intent(
        &self,
        owner: &TenantId,
        session: &SessionId,
    ) -> Result<(AcceptedLifecycleIntent, SessionPhase), PortError> {
        let record = self.public_lifecycle_record(owner, session)?;
        let intent = record
            .operations()
            .last()
            .ok_or(PortError::OutcomeUnknown)?
            .intent()
            .clone();
        Ok((intent, record.phase()))
    }

    pub(crate) fn public_lifecycle_record(
        &self,
        owner: &TenantId,
        session: &SessionId,
    ) -> Result<sandproof_center_core::Sandbox, PortError> {
        let current = self.snapshot()?;
        Ok(public_record(&current, owner, session)?.clone())
    }
    pub(super) fn accept_bound_destroy(
        &self,
        original: &AcceptedLifecycleIntent,
    ) -> Result<AcceptedLifecycleIntent, sandproof_center_store::authority::AuthorityError> {
        let mut authority = self
            .0
            .lock()
            .map_err(|_| sandproof_center_store::authority::AuthorityError::RecoveryRequired)?;
        authority.snapshot_envelope()?;
        authority.decide(|current| {
            let binding = &original.request.binding;
            let record = current
                .center
                .lifecycle_session(&binding.key.owner, &binding.session)
                .ok_or(DecisionError::NotFound)?;
            if record
                .operations()
                .first()
                .is_none_or(|first| first.intent() != original)
            {
                return Err(DecisionError::Conflict.into());
            }
            let prior = record
                .operations()
                .last()
                .ok_or(DecisionError::Invalid)?
                .intent();
            if record.terminating() {
                return Ok((current.clone(), prior.clone()));
            }
            let submission = LifecycleSubmission::Destroy {
                key: current.center.next_sdk_operation_key(&binding.key.owner)?,
                session: binding.session.clone(),
                expected_version: prior.accepted_version,
            };
            let (center, intent) =
                current
                    .center
                    .accept_lifecycle(&binding.key.owner, &submission, 0)?;
            Ok((current.with_center(center), intent))
        })
    }
    /// # Errors
    /// Public owner/visibility, current version and key allocation share one CAS.
    pub(super) fn accept_sdk_followup(
        &self,
        owner: &TenantId,
        sandbox: &sandproof_protocol::SandboxId,
        renewal: Option<(u64, bool)>,
        now_ms: u64,
    ) -> Result<AcceptedLifecycleIntent, sandproof_center_store::authority::AuthorityError> {
        let mut authority = self
            .0
            .lock()
            .map_err(|_| sandproof_center_store::authority::AuthorityError::RecoveryRequired)?;
        authority.snapshot_envelope()?;
        authority.decide(|current| {
            if current.metadata.private_sandboxes().contains(sandbox) {
                return Err(DecisionError::NotFound.into());
            }
            let session = SessionId::from_protocol_id(sandbox.clone());
            let record = current
                .center
                .lifecycle_session(owner, &session)
                .ok_or(DecisionError::NotFound)?;
            let prior = record
                .operations()
                .last()
                .ok_or(DecisionError::Invalid)?
                .intent();
            if renewal.is_none() && record.terminating() {
                return Ok((current.clone(), prior.clone()));
            }
            let key = current.center.next_sdk_operation_key(owner)?;
            let submission = match renewal {
                Some((timeout_secs, extend_only)) => LifecycleSubmission::Renew {
                    key,
                    session,
                    expected_version: prior.accepted_version,
                    timeout_secs,
                    extend_only,
                },
                None => LifecycleSubmission::Destroy {
                    key,
                    session,
                    expected_version: prior.accepted_version,
                },
            };
            let (center, intent) = current
                .center
                .accept_lifecycle(owner, &submission, now_ms)?;
            Ok((current.with_center(center), intent))
        })
    }

    /// Persist termination without waiting for any host operation. Exact
    /// replay returns the same intent; unknown CAS never means no effect.
    /// # Errors
    /// Owner, full input, expected version, window and authority must match.
    pub fn accept_public_destroy(
        &self,
        owner: &TenantId,
        session: SessionId,
        request: &PublicDestroyRequest,
    ) -> Result<AcceptedLifecycleIntent, PortError> {
        check_public_window(request.window)?;
        let mut authority = self.0.lock().map_err(|_| PortError::OutcomeUnknown)?;
        authority.snapshot_envelope().map_err(authority_error)?;
        authority
            .decide(|current| {
                if current
                    .metadata
                    .private_sandboxes()
                    .contains(session.as_protocol_id())
                {
                    return Err(DecisionError::NotFound.into());
                }
                let submission = LifecycleSubmission::Destroy {
                    key: key(&current.center, owner, request.window, request.sequence),
                    session,
                    expected_version: request.expected_version,
                };
                let (next, intent) = current.center.accept_lifecycle(owner, &submission, 0)?;
                Ok((current.with_center(next), intent))
            })
            .map_err(authority_error)
    }

    /// # Errors
    /// Replays confirm the original full input before the current policy/clock.
    /// New requests check the protected descriptor in the SAME envelope as CAS.
    pub fn accept_public_create(
        &self,
        owner: &TenantId,
        request: &PublicCreateRequest,
        policy: LifecyclePolicy,
        now: impl FnOnce() -> Result<u64, PortError>,
    ) -> Result<AcceptedLifecycleIntent, PortError> {
        check_public_window(request.window)?;
        self.accept_create_from(
            owner,
            &request.parameters,
            policy,
            now,
            CreateKeySource::Explicit(request.window, request.sequence),
        )
    }

    /// SDK inputs have no caller replay key. Allocate only in this same
    /// authoritative CAS; subsequent internal work keeps the accepted key.
    /// # Errors
    /// The same descriptor/policy/capacity checks as explicit creation apply.
    pub fn accept_sdk_create(
        &self,
        owner: &TenantId,
        parameters: &sandproof_protocol::lifecycle::CreateParameters,
        policy: LifecyclePolicy,
        now: impl FnOnce() -> Result<u64, PortError>,
    ) -> Result<AcceptedLifecycleIntent, PortError> {
        self.accept_create_from(
            owner,
            parameters,
            policy,
            now,
            CreateKeySource::AllocatedSdk,
        )
    }

    fn accept_create_from(
        &self,
        owner: &TenantId,
        parameters: &sandproof_protocol::lifecycle::CreateParameters,
        policy: LifecyclePolicy,
        now: impl FnOnce() -> Result<u64, PortError>,
        source: CreateKeySource,
    ) -> Result<AcceptedLifecycleIntent, PortError> {
        let mut authority = self.0.lock().map_err(|_| PortError::OutcomeUnknown)?;
        authority.snapshot_envelope().map_err(authority_error)?;
        authority
            .decide(|current| {
                let submission = LifecycleSubmission::Create {
                    key: match source {
                        CreateKeySource::Explicit(window, sequence) => {
                            key(&current.center, owner, window, sequence)
                        }
                        CreateKeySource::AllocatedSdk => {
                            current.center.next_sdk_operation_key(owner)?
                        }
                    },
                    parameters: parameters.clone(),
                };
                if let Some(intent) = current.center.replay_lifecycle(owner, &submission)? {
                    return Ok((current.clone(), intent));
                }
                let template = current
                    .metadata
                    .bound(&parameters.template_revision, &parameters.template_digest)
                    .map_err(to_authority)?;
                let descriptor = current
                    .metadata
                    .artifact_binding(&parameters.template_revision, &parameters.template_digest)
                    .map_err(to_authority)?;
                let owner_number = owner
                    .as_str()
                    .parse::<u64>()
                    .map_err(|_| DecisionError::Invalid)?;
                if descriptor
                    .owner
                    .is_some_and(|registered| registered != owner_number)
                    || current.metadata.builder_revision() == Some(&parameters.template_revision)
                {
                    return Err(DecisionError::NotFound.into());
                }
                let shape = template.resources();
                let millis = |seconds: u64| {
                    sandproof_protocol::numeric::seconds_to_milliseconds(seconds)
                        .ok_or(DecisionError::Invalid)
                };
                if !policy.is_valid()
                    || parameters.startup_timeout_ms > millis(policy.startup_timeout_secs)?
                    || parameters.timeout_ms > millis(policy.max_timeout_secs)?
                    || parameters.max_lifetime_ms > millis(policy.max_lifetime_secs)?
                    || parameters.resources.instances != 1
                    || parameters.resources.cpu != u64::from(shape.cpu_count)
                    || parameters.resources.memory_bytes != shape.host_memory_bytes
                    || parameters.resources.disk_bytes != shape.disk_bytes
                {
                    return Err(DecisionError::Invalid.into());
                }
                let options: CreateOptions = serde_json::from_slice(&parameters.options)
                    .map_err(|_| DecisionError::Invalid)?;
                if options.template.is_some()
                    || options.validate().is_err()
                    || options
                        .metadata
                        .keys()
                        .any(|key| key.starts_with("_sandproof_"))
                {
                    return Err(DecisionError::Invalid.into());
                }
                let networking = options.networking.as_ref();
                let command = CreateCommand {
                    template_id: template.template_id().to_owned(),
                    timeout: None,
                    allow_internet_access: networking.and_then(|n| n.allow_internet_access),
                    network: networking.and_then(|n| n.network.clone()),
                    metadata: None,
                    env_vars: None,
                };
                if selected_policy(&command).map_err(to_authority)?
                    != sandbox_controller_core::NetworkPolicy::DenyAll
                    && !policy.public_ipv4_enabled
                {
                    return Err(DecisionError::Invalid.into());
                }
                let (center, intent) = current.center.accept_lifecycle(
                    owner,
                    &submission,
                    now().map_err(to_authority)?,
                )?;
                Ok((current.with_center(center), intent))
            })
            .map_err(authority_error)
    }

    /// # Errors
    /// Return a confirmed full replay before any new host observation.
    pub fn replay_public_unload(
        &self,
        owner: &TenantId,
        session: &SessionId,
        request: &PublicUnloadRequest,
    ) -> Result<Option<AcceptedLifecycleIntent>, PortError> {
        check_public_window(request.window)?;
        let current = self.snapshot()?;
        if current
            .metadata
            .private_sandboxes()
            .contains(session.as_protocol_id())
        {
            return Err(PortError::NotFound);
        }
        current
            .center
            .replay_lifecycle(
                owner,
                &LifecycleSubmission::Unload {
                    key: key(&current.center, owner, request.window, request.sequence),
                    session: session.clone(),
                    expected_version: request.expected_version,
                },
            )
            .map_err(|error| authority_error(error.into()))
    }

    /// # Errors
    /// A new unload requires the same call's current observation. Its original
    /// Create and the new intent are rechecked and saved by one business CAS.
    pub fn accept_public_unload(
        &self,
        owner: &TenantId,
        session: &SessionId,
        request: &PublicUnloadRequest,
        observation: &CreateObservation,
        now_ms: u64,
    ) -> Result<AcceptedLifecycleIntent, PortError> {
        check_public_window(request.window)?;
        let mut authority = self.0.lock().map_err(|_| PortError::OutcomeUnknown)?;
        authority.snapshot_envelope().map_err(authority_error)?;
        authority
            .decide(|current| {
                if current
                    .metadata
                    .private_sandboxes()
                    .contains(session.as_protocol_id())
                {
                    return Err(DecisionError::NotFound.into());
                }
                let submission = LifecycleSubmission::Unload {
                    key: key(&current.center, owner, request.window, request.sequence),
                    session: session.clone(),
                    expected_version: request.expected_version,
                };
                if let Some(intent) = current.center.replay_lifecycle(owner, &submission)? {
                    return Ok((current.clone(), intent));
                }
                let CurrentCreateReadiness::Observed { facts } = &observation.readiness else {
                    return Err(DecisionError::Unavailable.into());
                };
                if observation.request.request.binding.key.owner != *owner
                    || &observation.request.request.binding.session != session
                    || now_ms >= facts.end_at_ms
                {
                    return Err(DecisionError::Stale.into());
                }
                let observed = current
                    .center
                    .accept_lifecycle_create_observation(observation)?;
                let (next, intent) = observed.accept_lifecycle(owner, &submission, now_ms)?;
                Ok((current.with_center(next), intent))
            })
            .map_err(authority_error)
    }

    /// # Errors
    /// Material, source and target identities come only from the original
    /// Unloaded record in this same CAS, never from public input.
    pub fn accept_public_resume(
        &self,
        owner: &TenantId,
        session: &SessionId,
        request: &PublicResumeRequest,
        now: impl FnOnce() -> Result<u64, PortError>,
    ) -> Result<AcceptedLifecycleIntent, PortError> {
        check_public_window(request.window)?;
        let mut authority = self.0.lock().map_err(|_| PortError::OutcomeUnknown)?;
        authority.snapshot_envelope().map_err(authority_error)?;
        authority
            .decide(|current| {
                if current
                    .metadata
                    .private_sandboxes()
                    .contains(session.as_protocol_id())
                {
                    return Err(DecisionError::NotFound.into());
                }
                let submission = LifecycleSubmission::Resume {
                    key: key(&current.center, owner, request.window, request.sequence),
                    session: session.clone(),
                    expected_version: request.expected_version,
                };
                if let Some(intent) = current.center.replay_lifecycle(owner, &submission)? {
                    return Ok((current.clone(), intent));
                }
                let (next, intent) = current.center.accept_lifecycle(
                    owner,
                    &submission,
                    now().map_err(to_authority)?,
                )?;
                Ok((current.with_center(next), intent))
            })
            .map_err(authority_error)
    }

    /// # Errors
    /// A second Unload is bound to the current resumed execution and lifetime,
    /// not the historical first Create. The original replay still comes first.
    pub fn accept_public_resumed_unload(
        &self,
        owner: &TenantId,
        session: &SessionId,
        request: &PublicUnloadRequest,
        observation: &sandproof_protocol::lifecycle::ResumeObservation,
        now_ms: u64,
    ) -> Result<AcceptedLifecycleIntent, PortError> {
        check_public_window(request.window)?;
        let mut authority = self.0.lock().map_err(|_| PortError::OutcomeUnknown)?;
        authority.snapshot_envelope().map_err(authority_error)?;
        authority
            .decide(|current| {
                if current
                    .metadata
                    .private_sandboxes()
                    .contains(session.as_protocol_id())
                {
                    return Err(DecisionError::NotFound.into());
                }
                let submission = LifecycleSubmission::Unload {
                    key: key(&current.center, owner, request.window, request.sequence),
                    session: session.clone(),
                    expected_version: request.expected_version,
                };
                if let Some(intent) = current.center.replay_lifecycle(owner, &submission)? {
                    return Ok((current.clone(), intent));
                }
                let sandproof_protocol::lifecycle::CurrentResumeReadiness::Observed { facts } =
                    &observation.readiness
                else {
                    return Err(DecisionError::Unavailable.into());
                };
                if observation.request.request.binding.key.owner != *owner
                    || &observation.request.request.binding.session != session
                    || now_ms >= facts.end_at_ms
                {
                    return Err(DecisionError::Stale.into());
                }
                let observed = current
                    .center
                    .accept_lifecycle_resume_observation(observation)?;
                let (next, intent) = observed.accept_lifecycle(owner, &submission, now_ms)?;
                Ok((current.with_center(next), intent))
            })
            .map_err(authority_error)
    }

    /// # Errors
    /// Fresh original authority, full owner and full session are mandatory.
    pub fn query_public_lifecycle(
        &self,
        owner: &TenantId,
        session: &SessionId,
    ) -> Result<PublicSessionView, PortError> {
        let record = self.public_lifecycle_record(owner, session)?;
        public_view(&record)
    }
}

fn public_record<'a>(
    current: &'a Envelope,
    owner: &TenantId,
    session: &SessionId,
) -> Result<&'a sandproof_center_core::Sandbox, PortError> {
    if current
        .metadata
        .private_sandboxes()
        .contains(session.as_protocol_id())
    {
        return Err(PortError::NotFound);
    }
    current
        .center
        .lifecycle_session(owner, session)
        .ok_or(PortError::NotFound)
}

/// Pure projection; callers must supply their same-operation confirmed state.
fn public_view(record: &sandproof_center_core::Sandbox) -> Result<PublicSessionView, PortError> {
    let current = record
        .operations()
        .last()
        .ok_or(PortError::OutcomeUnknown)?
        .intent();
    let phase = match record.phase() {
        SessionPhase::Creating => PublicSessionPhase::Creating,
        // The ledger retains a historical Ready. A current publishable
        // Running needs the original host observation provider, still
        // absent here. A new query timestamp cannot supply that evidence.
        SessionPhase::Running => return Err(PortError::Busy),
        SessionPhase::Renewing => PublicSessionPhase::Renewing,
        SessionPhase::Unloading => PublicSessionPhase::Unloading,
        SessionPhase::Unloaded => PublicSessionPhase::Unloaded,
        SessionPhase::Resuming => PublicSessionPhase::Resuming,
        SessionPhase::StateLost => PublicSessionPhase::StateLost,
        SessionPhase::Terminating => PublicSessionPhase::Terminating,
        SessionPhase::Cleaned => PublicSessionPhase::Cleaned,
    };
    Ok(PublicSessionView {
        session: current.request.binding.session.clone(),
        operation: current.request.binding.key.clone(),
        current_version: current.accepted_version,
        phase,
        unload_outcome: record
            .operations()
            .last()
            .and_then(|operation| operation.unloaded())
            .map(|observation| observation.outcome.clone())
            .or_else(|| {
                matches!(current.request.operation, LifecycleOperation::Unload { .. })
                    .then_some(sandproof_protocol::lifecycle::UnloadOutcome::Pending)
            }),
        create_refusal: record
            .operations()
            .first()
            .and_then(sandproof_center_core::LifecycleOperationRecord::create_refusal),
    })
}

#[derive(Clone, Copy)]
enum CreateKeySource {
    Explicit(
        sandproof_protocol::lifecycle::RequestWindowId,
        sandproof_protocol::lifecycle::RequestSequence,
    ),
    AllocatedSdk,
}

fn check_public_window(
    window: sandproof_protocol::lifecycle::RequestWindowId,
) -> Result<(), PortError> {
    if window.get().is_multiple_of(2) {
        Err(PortError::Invalid)
    } else {
        Ok(())
    }
}

fn key(
    center: &CenterState,
    owner: &TenantId,
    window: sandproof_protocol::lifecycle::RequestWindowId,
    sequence: sandproof_protocol::lifecycle::RequestSequence,
) -> OperationKey {
    OperationKey {
        deployment: center.deployment().clone(),
        owner: owner.clone(),
        window,
        sequence,
    }
}
