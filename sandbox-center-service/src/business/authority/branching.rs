use super::{Envelope, SharedBusinessAuthority, to_authority};
use crate::{application::authority_error, branch_batch::BatchError, projection::live};
use sandproof_api::PortError;
use sandproof_center_core::DecisionError;
use sandproof_controller_service::application::branching::{
    BranchBatchIdentity, BranchControlAction, BranchControlReply, BranchControlRequest,
    BranchControlStatus, BranchStartFailure, BranchTarget, PublicBranchBatch,
    PublicBranchBatchRequest,
};
use sandproof_protocol::{
    ResourceId, SandboxId, TenantId,
    lifecycle::{
        ContentDigest, IncarnationId, LifecycleOperation, OperationKey, RequestSequence,
        RequestWindowId, SessionId,
    },
};

impl SharedBusinessAuthority {
    /// Reserve the complete host plan in the same authority that admits Unload.
    /// The caller selects the plan from protected template and host configuration.
    /// # Errors
    /// Unknown storage, a foreign parent, a stale window/VM or insufficient
    /// capacity cannot authorize a new batch.
    pub fn reserve_branch_batch(
        &self,
        owner: &TenantId,
        parent: SessionId,
        request: PublicBranchBatchRequest,
        batch: ResourceId,
        now_ms: u64,
    ) -> Result<PublicBranchBatch, PortError> {
        let mut authority = self.0.lock().map_err(|_| PortError::OutcomeUnknown)?;
        authority.snapshot_envelope().map_err(authority_error)?;
        authority
            .decide(|current| {
                let operation = OperationKey {
                    deployment: current.center.deployment().clone(),
                    owner: owner.clone(),
                    window: request.window,
                    sequence: request.sequence,
                };
                if current.center.has_lifecycle_key(&operation) {
                    return Err(DecisionError::Conflict.into());
                }
                let mut next = current.clone();
                let ledger = next
                    .metadata
                    .branch_batches
                    .as_mut()
                    .ok_or(DecisionError::Unavailable)?;
                if let Some(original) = ledger.find(&operation) {
                    let result = original
                        .replay(&operation, &parent, &request)
                        .map_err(map_error)?
                        .ok_or(DecisionError::Conflict)?;
                    return Ok((next, result));
                }
                if current.center.public_request_window(owner)? != Some(request.window) {
                    return Err(DecisionError::Stale.into());
                }
                let incarnation = live_incarnation(
                    current,
                    owner,
                    &parent,
                    now_ms,
                    Some(request.expected_version),
                )
                .map_err(to_authority)?;
                let parent_record = current
                    .center
                    .lifecycle_session(owner, &parent)
                    .ok_or(DecisionError::NotFound)?;
                if ledger.parent_exited(owner, &parent, incarnation) {
                    return Err(DecisionError::Conflict.into());
                }
                let original = parent_record
                    .operations()
                    .first()
                    .ok_or(DecisionError::NotFound)?
                    .intent();
                let LifecycleOperation::Create { create, .. } = &original.request.operation else {
                    return Err(DecisionError::NotFound.into());
                };
                if !ledger.templates().contains(&create.template_revision) {
                    return Err(DecisionError::Invalid.into());
                }
                let identity = BranchBatchIdentity {
                    parent,
                    incarnation,
                    checkpoint: request.checkpoint.clone(),
                    batch,
                };
                ledger.retire_closed(|original| {
                    match current.center.public_request_window(&original.owner) {
                        Ok(Some(window)) => window.get() > original.window.get(),
                        Ok(None) => true,
                        Err(_) => false,
                    }
                });
                let ports = ledger
                    .available_ports(&identity.parent, usize::from(request.count.get()))
                    .map_err(map_error)?;
                let plan = ports
                    .into_iter()
                    .enumerate()
                    .map(|(ordinal, port)| {
                        let sandbox = format!("branch-{}-{ordinal}", identity.batch.as_str())
                            .try_into()
                            .map_err(|_| DecisionError::Invalid)?;
                        let port = port.try_into().map_err(|_| DecisionError::Invalid)?;
                        Ok(BranchTarget { sandbox, port })
                    })
                    .collect::<Result<Vec<_>, DecisionError>>()?;
                let result = ledger
                    .start(operation, identity, request, plan)
                    .map_err(map_error)?;
                Ok((next, result))
            })
            .map_err(authority_error)
    }

    /// # Errors
    /// Foreign or unknown handles are not visible, even when still occupied.
    pub fn query_branch_batch(
        &self,
        owner: &TenantId,
        parent: &SessionId,
        window: RequestWindowId,
        sequence: RequestSequence,
    ) -> Result<PublicBranchBatch, PortError> {
        let mut authority = self.0.lock().map_err(|_| PortError::OutcomeUnknown)?;
        let current = authority.snapshot_envelope().map_err(authority_error)?;
        let operation = OperationKey {
            deployment: current.center.deployment().clone(),
            owner: owner.clone(),
            window,
            sequence,
        };
        let record = current
            .metadata
            .branch_batches
            .as_ref()
            .and_then(|ledger| ledger.find(&operation))
            .ok_or(PortError::NotFound)?;
        let result = record.public_result();
        if &result.parent != parent {
            return Err(PortError::NotFound);
        }
        Ok(result)
    }

    /// # Errors
    /// Only a complete matching observation for the current live VM can publish.
    pub fn publish_branch_batch(
        &self,
        operation: &OperationKey,
        identity: &BranchBatchIdentity,
        targets: &[BranchTarget],
        now_ms: u64,
    ) -> Result<(), PortError> {
        self.change_branch_batch(operation, |current, record| {
            require_parent(current, &operation.owner, identity, now_ms, None)?;
            record.publish(identity, targets).map_err(port_error)
        })
    }

    /// # Errors
    /// Unknown authority or a different batch cannot record cleanup intent.
    pub fn begin_branch_batch_cleanup(
        &self,
        operation: &OperationKey,
        identity: &BranchBatchIdentity,
        reason: BranchStartFailure,
    ) -> Result<(), PortError> {
        self.change_branch_batch(operation, |_, record| {
            record.require_identity(identity).map_err(port_error)?;
            record.begin_cleanup(reason);
            Ok(())
        })
    }

    /// # Errors
    /// Cleanup must name the original batch and follow recorded cleanup intent.
    pub fn confirm_branch_batch_cleanup(
        &self,
        operation: &OperationKey,
        identity: &BranchBatchIdentity,
    ) -> Result<(), PortError> {
        self.change_branch_batch(operation, |_, record| {
            record.confirm_cleanup(identity).map_err(port_error)
        })
    }

    /// # Errors
    /// The original authorization and the individual branch must still exist.
    pub fn begin_branch_release(
        &self,
        operation: &OperationKey,
        identity: &BranchBatchIdentity,
        sandbox: &SandboxId,
    ) -> Result<(), PortError> {
        self.change_branch_batch(operation, |_, record| {
            record.require_identity(identity).map_err(port_error)?;
            record.begin_release(sandbox).map_err(port_error)
        })
    }

    /// # Errors
    /// Requires the previously revoked original branch and exact batch identity.
    pub fn confirm_branch_release(
        &self,
        operation: &OperationKey,
        identity: &BranchBatchIdentity,
        sandbox: &SandboxId,
    ) -> Result<(), PortError> {
        self.change_branch_batch(operation, |_, record| {
            record
                .confirm_release(identity, sandbox)
                .map_err(port_error)
        })
    }

    /// Persist branch revocation before emitting a physical release request.
    /// # Errors
    /// Missing, foreign or not-yet-published branches cannot be revoked here.
    pub fn revoke_branch(&self, owner: &TenantId, sandbox: &SandboxId) -> Result<(), PortError> {
        let mut authority = self.0.lock().map_err(|_| PortError::OutcomeUnknown)?;
        authority.snapshot_envelope().map_err(authority_error)?;
        authority
            .decide(|current| {
                let mut next = current.clone();
                let record = next
                    .metadata
                    .branch_batches
                    .as_mut()
                    .and_then(|ledger| ledger.find_branch_mut(owner, sandbox))
                    .ok_or(DecisionError::NotFound)?;
                record.begin_release(sandbox).map_err(map_error)?;
                Ok((next, ()))
            })
            .map_err(authority_error)
    }

    /// Take a finite work projection from the same confirmed lifecycle envelope.
    /// # Errors
    /// Unknown authority cannot supply a work list.
    pub fn pending_branch_work(&self, now_ms: u64) -> Result<Vec<BranchControlRequest>, PortError> {
        let mut authority = self.0.lock().map_err(|_| PortError::OutcomeUnknown)?;
        let current = authority.snapshot_envelope().map_err(authority_error)?;
        let Some(ledger) = &current.metadata.branch_batches else {
            return Ok(Vec::new());
        };
        Ok(ledger
            .batches()
            .iter()
            .filter_map(|record| {
                let action = record.next_action()?;
                let (operation, batch, targets) = record.recorded_plan();
                let parent = current
                    .center
                    .lifecycle_session(&operation.owner, &batch.parent)?;
                let route =
                    crate::projection::route_for_record(&current.center, parent, now_ms).ok()?;
                (route.identity.incarnation == batch.incarnation
                    && current
                        .center
                        .node_is_reachable(&route.identity.node, now_ms))
                .then(|| BranchControlRequest {
                    parent: route.identity,
                    operation: operation.clone(),
                    batch: batch.clone(),
                    targets: targets.to_vec(),
                    action,
                })
            })
            .collect())
    }

    /// Accept a matching authenticated host result against the latest envelope.
    /// # Errors
    /// Unknown transactions or a changed original plan never acknowledge an effect.
    pub fn observe_branch_work(
        &self,
        request: &BranchControlRequest,
        reply: &BranchControlReply,
        now_ms: u64,
    ) -> Result<(), PortError> {
        if !reply.matches(request) {
            return Err(PortError::Conflict);
        }
        match (&request.action, reply.status) {
            (_, BranchControlStatus::ParentExited { source }) => {
                self.fence_exited_branch_parent(request, source)
            }
            (_, BranchControlStatus::Pending {} | BranchControlStatus::CleaningUp {}) => Ok(()),
            (BranchControlAction::Start {}, BranchControlStatus::Ready {}) => self
                .publish_branch_batch(&request.operation, &request.batch, &request.targets, now_ms),
            (
                action @ (BranchControlAction::Start {} | BranchControlAction::Observe {}),
                BranchControlStatus::Failed { reason },
            ) => self.change_branch_batch(&request.operation, |_, record| {
                record
                    .require_identity(&request.batch)
                    .map_err(port_error)?;
                let allowed = if matches!(action, BranchControlAction::Start {}) {
                    record.next_action().as_ref() == Some(action)
                } else {
                    record.reserved() != 0 && reason == BranchStartFailure::GuestSetupFailed
                };
                if !allowed {
                    return Err(PortError::Conflict);
                }
                record.begin_cleanup(reason);
                record.confirm_cleanup(&request.batch).map_err(port_error)
            }),
            (
                BranchControlAction::Release {
                    branch: Some(branch),
                },
                BranchControlStatus::Released {},
            ) => self.confirm_branch_release(&request.operation, &request.batch, branch),
            (BranchControlAction::Release { branch: None }, BranchControlStatus::Released {}) => {
                self.confirm_branch_batch_cleanup(&request.operation, &request.batch)
            }
            _ => Err(PortError::Conflict),
        }
    }

    fn fence_exited_branch_parent(
        &self,
        request: &BranchControlRequest,
        source: ContentDigest,
    ) -> Result<(), PortError> {
        let mut authority = self.0.lock().map_err(|_| PortError::OutcomeUnknown)?;
        authority.snapshot_envelope().map_err(authority_error)?;
        authority
            .decide(|current| {
                let parent = current
                    .center
                    .lifecycle_session(&request.operation.owner, &request.batch.parent)
                    .ok_or(DecisionError::NotFound)?;
                require_exit_execution(parent, request)?;
                let original = parent.operations().first().ok_or(DecisionError::NotFound)?;
                let LifecycleOperation::Create { create, .. } =
                    &original.intent().request.operation
                else {
                    return Err(DecisionError::NotFound.into());
                };
                current
                    .metadata
                    .artifact_binding(&create.template_revision, &create.template_digest)
                    .map_err(to_authority)?;
                let mut next = current.clone();
                let ledger = next
                    .metadata
                    .branch_batches
                    .as_mut()
                    .ok_or(DecisionError::NotFound)?;
                if !ledger.templates().contains(&create.template_revision) {
                    return Err(DecisionError::Conflict.into());
                }
                ledger
                    .fence_parent_exit(&request.operation, &request.batch, &request.targets, source)
                    .map_err(map_error)?;
                Ok((next, ()))
            })
            .map_err(authority_error)
    }

    /// # Errors
    /// Returns only an active branch under an exactly matched live parent.
    pub fn authorized_branch_target(
        &self,
        operation: &OperationKey,
        identity: &BranchBatchIdentity,
        sandbox: &SandboxId,
        now_ms: u64,
    ) -> Result<BranchTarget, PortError> {
        let mut authority = self.0.lock().map_err(|_| PortError::OutcomeUnknown)?;
        let current = authority.snapshot_envelope().map_err(authority_error)?;
        require_parent(&current, &operation.owner, identity, now_ms, None)?;
        let record = current
            .metadata
            .branch_batches
            .as_ref()
            .and_then(|ledger| ledger.find(operation))
            .ok_or(PortError::NotFound)?;
        record.require_identity(identity).map_err(port_error)?;
        record.target(sandbox).cloned().ok_or(PortError::NotFound)
    }

    fn change_branch_batch(
        &self,
        operation: &OperationKey,
        change: impl FnOnce(
            &Envelope,
            &mut crate::branch_batch::BranchBatchRecord,
        ) -> Result<(), PortError>,
    ) -> Result<(), PortError> {
        let mut authority = self.0.lock().map_err(|_| PortError::OutcomeUnknown)?;
        authority.snapshot_envelope().map_err(authority_error)?;
        authority
            .decide(|current| {
                let mut next = current.clone();
                let record = next
                    .metadata
                    .branch_batches
                    .as_mut()
                    .and_then(|ledger| ledger.find_mut(operation))
                    .ok_or(DecisionError::NotFound)?;
                change(current, record).map_err(to_authority)?;
                Ok((next, ()))
            })
            .map_err(authority_error)
    }
}

fn require_exit_execution(
    parent: &sandproof_center_core::Sandbox,
    request: &BranchControlRequest,
) -> Result<(), DecisionError> {
    let origin = parent
        .current_execution_origin()
        .ok_or(DecisionError::Conflict)?;
    let (LifecycleOperation::Create {
        target_incarnation: incarnation,
        ..
    }
    | LifecycleOperation::Resume {
        target_incarnation: incarnation,
        ..
    }) = origin.request.operation
    else {
        return Err(DecisionError::Conflict);
    };
    let route = &request.parent;
    let binding = &origin.request.binding;
    if parent.released()
        || !request.matches_node(&binding.node)
        || route.incarnation != incarnation
        || route.center != origin.center
        || route.session != binding.session
        || route.operation.owner != binding.key.owner
        || route.accepted_version < origin.accepted_version
    {
        return Err(DecisionError::Conflict);
    }
    // Renew can advance the route without replacing the execution. Match the
    // exact in-flight route against its persisted operation, then fence only
    // the original execution, even if a newer lease now describes it.
    if parent.operations().iter().any(|operation| {
        let accepted = operation.intent();
        let original = &accepted.request.binding;
        let accepted_incarnation = match accepted.request.operation {
            LifecycleOperation::Create {
                target_incarnation, ..
            }
            | LifecycleOperation::Resume {
                target_incarnation, ..
            } => target_incarnation,
            LifecycleOperation::Renew { incarnation, .. } => incarnation,
            LifecycleOperation::Unload {
                source_incarnation, ..
            } if operation
                .unloaded()
                .and_then(|observation| observation.outcome.continued())
                .is_some() =>
            {
                source_incarnation
            }
            _ => return false,
        };
        accepted.accepted_version == route.accepted_version
            && accepted.center == route.center
            && original.node == route.node
            && original.session == route.session
            && original.key == route.operation
            && accepted_incarnation == route.incarnation
    }) {
        Ok(())
    } else {
        Err(DecisionError::Conflict)
    }
}

fn require_parent(
    current: &Envelope,
    owner: &TenantId,
    identity: &BranchBatchIdentity,
    now_ms: u64,
    expected_version: Option<u64>,
) -> Result<(), PortError> {
    if live_incarnation(current, owner, &identity.parent, now_ms, expected_version)?
        == identity.incarnation
    {
        Ok(())
    } else {
        Err(PortError::Conflict)
    }
}

fn live_incarnation(
    current: &Envelope,
    owner: &TenantId,
    session: &SessionId,
    now_ms: u64,
    expected_version: Option<u64>,
) -> Result<IncarnationId, PortError> {
    if current
        .metadata
        .private_sandboxes()
        .contains(session.as_protocol_id())
    {
        return Err(PortError::NotFound);
    }
    let parent = current
        .center
        .lifecycle_session(owner, session)
        .ok_or(PortError::NotFound)?;
    let origin = parent
        .current_execution_origin()
        .ok_or(PortError::Conflict)?;
    if !live(parent, now_ms)
        || expected_version.is_some_and(|version| parent.current_version() != Some(version))
        || !current
            .center
            .node_is_reachable(&origin.request.binding.node, now_ms)
    {
        return Err(PortError::Conflict);
    }
    match origin.request.operation {
        LifecycleOperation::Create {
            target_incarnation, ..
        }
        | LifecycleOperation::Resume {
            target_incarnation, ..
        } => Ok(target_incarnation),
        _ => Err(PortError::Conflict),
    }
}

fn map_error(error: BatchError) -> DecisionError {
    match error {
        BatchError::Capacity => DecisionError::Capacity,
        BatchError::InvalidTargets => DecisionError::Invalid,
        BatchError::Identity | BatchError::Conflict | BatchError::WrongPhase => {
            DecisionError::Conflict
        }
    }
}

fn port_error(error: BatchError) -> PortError {
    authority_error(map_error(error).into())
}
