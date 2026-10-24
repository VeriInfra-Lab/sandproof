use super::super::{
    model::Inbox,
    readiness::snapshot::{base_snapshot, startup_records},
};
use super::{
    DeserializeOwned, HostEvidence, JournalError, SourceAnchor, SourceObservationConfig,
    SourceObservationRequirement, SourceWork, SourceWorkRecord, digest,
};
use crate::capture::HostEpochObservation;
use crate::controller::{admission, capture, checkpoint, data_credentials};
use rusqlite::{Connection, Transaction};
use sandbox_controller_core::{self as core, source_observation::SourceObservationGuard};
use sandproof_protocol::lifecycle::{AcceptedLifecycleIntent, LifecycleOperation};

#[derive(Clone, Copy)]
pub(super) struct CurrentSource<'a> {
    pub world: &'a core::World,
    pub inbox: &'a Inbox,
    pub limits: &'a SourceObservationConfig,
    pub active: Option<&'a HostEpochObservation>,
}

fn live_native(
    native: &crate::NativeCreateSnapshot,
    local: crate::InternalLocalBinding,
    accepted: u64,
    current_version: u64,
    now_ms: i64,
) -> bool {
    let Some(lifetime) = native.lifetime else {
        return false;
    };
    let Some(vm) = native.current_vm else {
        return false;
    };
    SourceObservationGuard {
        work_version: accepted,
        current_version,
        work_owner: local.owner,
        current_owner: native.owner,
        work_vm: vm,
        current_vm: native.current_vm,
        work_generation: native.generation,
        current_generation: native.generation,
        work_epoch: native.epoch,
        current_epoch: native.epoch,
        phase: native.phase,
        terminal: native.terminal,
        now_ms,
        deadline_ms: lifetime.deadline_ms,
    }
    .allows()
        && lifetime
            .started_at_ms
            .is_some_and(|started| started <= now_ms && started <= lifetime.startup_deadline_ms)
}

pub(super) fn current<E: HostEvidence + DeserializeOwned>(
    tx: &Transaction<'_>,
    namespace: &str,
    intent: &AcceptedLifecycleIntent,
    now_ms: i64,
    current: CurrentSource<'_>,
) -> Result<SourceWork, JournalError> {
    let CurrentSource {
        world,
        inbox,
        limits,
        active,
    } = current;
    let (s, e) = inbox.find(intent)?.ok_or(JournalError::Conflict)?;
    let session = &inbox.sessions[s];
    let state = inbox.state(s, e);
    if !state.current
        || state.terminal
        || !matches!(intent.request.operation, LifecycleOperation::Unload { .. })
    {
        return Err(JournalError::Conflict);
    }
    let bootstrap = inbox
        .host_epoch_observation
        .as_ref()
        .ok_or(JournalError::UnresolvedAction)?;
    if active != Some(bootstrap) {
        return Err(JournalError::UnresolvedAction);
    }
    let local = state.local.ok_or(JournalError::UnresolvedAction)?;
    let create = session
        .entries
        .iter()
        .find(|entry| {
            matches!(
                entry.intent.request.operation,
                LifecycleOperation::Create { .. }
            )
        })
        .ok_or(JournalError::CorruptStore)?
        .intent
        .clone();
    let view = base_snapshot(tx, namespace, world, &create, &inbox.config.context())?.view;
    let native = view.native.ok_or(JournalError::CorruptStore)?;
    if !native.has_completed_original_create(local)
        || view.historical_result != Some(core::OperationResult::Running(native.generation))
    {
        return Err(JournalError::UnresolvedAction);
    }
    if !live_native(
        &native,
        local,
        intent.accepted_version,
        session.current_version,
        now_ms,
    ) {
        return Err(JournalError::UnresolvedAction);
    }
    let source = capture::original_source::<E>(tx, namespace, world, local.sandbox)?;
    if source.boot_id != bootstrap.registration.boot_id
        || source.pid == 0
        || source.start_time_ticks == 0
    {
        return Err(JournalError::InvalidHostEvidence);
    }
    let (actions, receipts) = startup_records::<E>(tx, namespace, world, local)?;
    let options = admission::read_create_options(tx, namespace, local.sandbox)
        .map_err(|_| JournalError::CorruptStore)?;
    let credential = data_credentials::snapshot(tx, world, namespace, local.sandbox, true)?
        .ok_or(JournalError::UnresolvedAction)?;
    if credential.0 != native.generation {
        return Err(JournalError::CorruptStore);
    }
    let anchor = SourceAnchor {
        request: intent.clone(),
        create,
        local,
        native,
        node: inbox.config.node.clone(),
        host_bootstrap: bootstrap.registration.clone(),
        source: source.clone(),
        options_sha256: digest(&serde_json::to_vec(&options)?),
        credential,
        original_actions: actions.iter().map(|a| a.identity.clone()).collect(),
        original_receipts: receipts.iter().map(|r| digest(r)).collect(),
    };
    let requirement = SourceObservationRequirement {
        work_binding: digest(&serde_json::to_vec(&anchor)?),
        node: inbox.config.node.clone(),
        host_bootstrap: bootstrap.registration.clone(),
        resource: actions[1].identity.resource.clone(),
        source,
        limits: limits.clone(),
    };
    Ok(SourceWork {
        anchor,
        requirement,
    })
}

pub(super) fn validate_record(
    connection: &Connection,
    world: &core::World,
    inbox: &Inbox,
    config: &SourceObservationConfig,
    record: &SourceWorkRecord,
) -> Result<(), JournalError> {
    let work = &record.work;
    let anchor = &work.anchor;
    let req = &work.requirement;
    let (s, _) = inbox
        .find(&anchor.request)
        .map_err(|_| JournalError::CorruptStore)?
        .ok_or(JournalError::CorruptStore)?;
    let session = &inbox.sessions[s];
    if !matches!(
        anchor.request.request.operation,
        LifecycleOperation::Unload { .. }
    ) || session.local != Some(anchor.local)
        || inbox
            .find(&anchor.create)
            .map_err(|_| JournalError::CorruptStore)?
            .is_none()
        || anchor.create.request.binding.session != session.id
        || !matches!(
            anchor.create.request.operation,
            LifecycleOperation::Create { .. }
        )
        || !anchor.native.has_completed_original_create(anchor.local)
        || req.limits != *config
        || req.node != inbox.config.node
        || anchor.node != req.node
        || Some(&req.host_bootstrap) != inbox.config.host_epoch.as_ref()
        || anchor.host_bootstrap != req.host_bootstrap
        || req.source != anchor.source
        || !req.host_bootstrap.valid()
        || req.source.boot_id != req.host_bootstrap.boot_id
        || req.source.pid == 0
        || req.source.start_time_ticks == 0
        || req.work_binding != digest(&serde_json::to_vec(anchor)?)
        || anchor.original_actions.len() != 5
        || anchor.original_receipts.len() != 5
        || req.resource != anchor.original_actions[1].resource
        || serde_json::to_vec(work)?.len() as u64 > u64::from(config.max_work_bytes)
        || record.observation.as_ref().is_some_and(|o| !o.matches(req))
    {
        return Err(JournalError::CorruptStore);
    }
    if let Some(current) = world.sandbox_record(anchor.local.sandbox) {
        if current.owner != anchor.local.owner
            || current.generation != anchor.native.generation
            || world
                .operation_record(core::OperationKey {
                    sandbox: anchor.local.sandbox,
                    operation: anchor.local.operation,
                })
                .is_none()
        {
            return Err(JournalError::CorruptStore);
        }
    } else if !session.terminal {
        return Err(JournalError::CorruptStore);
    }
    let namespace = checkpoint::namespace(connection)?;
    let vm = anchor.native.current_vm.ok_or(JournalError::CorruptStore)?;
    let expected_resource = crate::controller::identity::resource(
        &namespace,
        &core::Command::ActivateVm {
            sandbox: anchor.local.sandbox,
            epoch: anchor.native.epoch,
            generation: anchor.native.generation,
            vm,
        },
    )?;
    if req.resource != expected_resource {
        return Err(JournalError::CorruptStore);
    }
    for action in &anchor.original_actions {
        if action.owner != crate::controller::identity::owner_id(anchor.local.owner)?
            || action.sandbox
                != crate::controller::identity::sandbox_id(&namespace, anchor.local.sandbox)?
            || action.operation
                != crate::controller::identity::operation_id(
                    &namespace,
                    core::OperationKey {
                        sandbox: anchor.local.sandbox,
                        operation: anchor.local.operation,
                    },
                )?
            || action.epoch != anchor.native.epoch
            || action.generation != anchor.native.generation
        {
            return Err(JournalError::CorruptStore);
        }
    }
    Ok(())
}
