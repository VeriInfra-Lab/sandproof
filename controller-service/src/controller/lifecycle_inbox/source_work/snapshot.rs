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
        || session.entries[e].unload_resolution.is_some()
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
    let execution_origin = session
        .current_execution_origin()
        .ok_or(JournalError::CorruptStore)?
        .intent
        .clone();
    let view = base_snapshot(
        tx,
        namespace,
        world,
        &execution_origin,
        &inbox.config.context(),
    )?
    .view;
    let native = view.native.ok_or(JournalError::CorruptStore)?;
    let continued = if native.has_completed_original_execution(local) {
        None
    } else {
        super::full::continued_ready_records::<E>(tx, world, namespace, local, &native)?
    };
    if (!native.has_completed_original_execution(local) && continued.is_none())
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
    let source = match &continued {
        Some((work, ..)) => work.source().source.clone(),
        None => capture::original_source::<E>(tx, namespace, world, local.sandbox)?,
    };
    if source.boot_id != bootstrap.registration.boot_id
        || source.pid == 0
        || source.start_time_ticks == 0
    {
        return Err(JournalError::InvalidHostEvidence);
    }
    let (mut actions, mut receipts) = startup_records::<E>(tx, namespace, world, local)?;
    let continued_from = continued.map(|(work, action, receipt)| {
        actions.push(action);
        receipts.push(receipt);
        work.identity()
    });
    let options = admission::read_create_options(tx, namespace, local.sandbox)
        .map_err(|_| JournalError::CorruptStore)?;
    let credential = data_credentials::snapshot(tx, world, namespace, local.sandbox, true)?
        .ok_or(JournalError::UnresolvedAction)?;
    if credential.0 != native.generation {
        return Err(JournalError::CorruptStore);
    }
    let anchor = SourceAnchor {
        request: intent.clone(),
        execution_origin,
        local,
        native,
        continued_from,
        node: inbox.config.node.clone(),
        host_bootstrap: bootstrap.registration.clone(),
        source: source.clone(),
        options_sha256: digest(&serde_json::to_vec(&options)?),
        credential,
        original_actions: actions.iter().map(|a| a.identity.clone()).collect(),
        original_receipts: receipts.iter().map(|r| digest(r)).collect(),
    };
    question(anchor, limits)
}

fn question(anchor: SourceAnchor, limits: &SourceObservationConfig) -> Result<SourceWork, JournalError> {
    let requirement = SourceObservationRequirement {
        work_binding: digest(&serde_json::to_vec(&anchor)?),
        node: anchor.node.clone(),
        host_bootstrap: anchor.host_bootstrap.clone(),
        resource: anchor.original_actions[1].resource.clone(),
        source: anchor.source.clone(),
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
    validate_execution_origin(world, inbox, anchor, s)?;
    if !matches!(
        anchor.request.request.operation,
        LifecycleOperation::Unload { .. }
    ) || session.local.is_none_or(|local| {
        local.sandbox != anchor.local.sandbox || local.owner != anchor.local.owner
    }) || inbox
        .find(&anchor.execution_origin)
        .map_err(|_| JournalError::CorruptStore)?
        .is_none()
        || anchor.execution_origin.request.binding.session != session.id
        || !matches!(
            anchor.execution_origin.request.operation,
            LifecycleOperation::Create { .. } | LifecycleOperation::Resume { .. }
        )
        || if let Some(identity) = anchor.continued_from {
            super::full::continued_identity(world, anchor.local, &anchor.native) != Some(identity)
        } else {
            !anchor.native.has_completed_original_execution(anchor.local)
        }
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
        || anchor.original_actions.len()
            != if anchor.continued_from.is_some() {
                6
            } else {
                5
            }
        || anchor.original_receipts.len() != anchor.original_actions.len()
        || req.resource != anchor.original_actions[1].resource
        || serde_json::to_vec(work)?.len() as u64 > u64::from(config.max_work_bytes)
        || record.observation.as_ref().is_some_and(|o| !o.matches(req))
    {
        return Err(JournalError::CorruptStore);
    }
    if let Some(current) = world.sandbox_record(anchor.local.sandbox) {
        if current.owner != anchor.local.owner
            || current.generation < anchor.native.generation
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
    validate_original_actions(world, &namespace, anchor)
}

fn validate_original_actions(world:&core::World,namespace:&str,anchor:&SourceAnchor)->Result<(),JournalError>{
    let origin_key = core::OperationKey {
        sandbox: anchor.local.sandbox,
        operation: anchor.local.operation,
    };
    let origin_plan = world
        .operation_plan(origin_key)
        .ok_or(JournalError::CorruptStore)?;
    for (index, action) in anchor.original_actions.iter().enumerate() {
        let (key, epoch) = if index == 5 {
            let continued = anchor.continued_from.ok_or(JournalError::CorruptStore)?;
            (continued.key, continued.epoch)
        } else {
            (origin_key, origin_plan.epoch)
        };
        if action.owner != crate::controller::identity::owner_id(anchor.local.owner)?
            || action.sandbox
                != crate::controller::identity::sandbox_id(namespace, anchor.local.sandbox)?
            || action.operation != crate::controller::identity::operation_id(namespace, key)?
            || action.epoch != epoch
            || action.generation != anchor.native.generation
        {
            return Err(JournalError::CorruptStore);
        }
    }
    Ok(())
}

fn validate_execution_origin(
    world: &core::World,
    inbox: &Inbox,
    anchor: &SourceAnchor,
    session: usize,
) -> Result<(), JournalError> {
    let (origin_session, origin_entry) = inbox
        .find(&anchor.execution_origin)
        .map_err(|_| JournalError::CorruptStore)?
        .ok_or(JournalError::CorruptStore)?;
    if origin_session != session
        || inbox.sessions[session].entries[origin_entry].local_operation
            != Some(anchor.local.operation)
    {
        return Err(JournalError::CorruptStore);
    }
    let origin_key = core::OperationKey {
        sandbox: anchor.local.sandbox,
        operation: anchor.local.operation,
    };
    let origin_plan = world
        .operation_plan(origin_key)
        .ok_or(JournalError::CorruptStore)?;
    let origin_row = world
        .operation_record(origin_key)
        .ok_or(JournalError::CorruptStore)?;
    let origin_kind = match anchor.execution_origin.request.operation {
        LifecycleOperation::Create { .. } => core::OperationKind::Create,
        LifecycleOperation::Resume { .. } => core::OperationKind::Resume,
        _ => return Err(JournalError::CorruptStore),
    };
    let epoch_matches = match anchor.continued_from {
        Some(identity) => {
            super::full::continued_identity(world, anchor.local, &anchor.native) == Some(identity)
        }
        None => origin_plan.epoch == anchor.native.epoch,
    };
    if origin_row.operation.kind != origin_kind
        || origin_plan.generation != anchor.native.generation
        || !epoch_matches
        || origin_plan.vm != anchor.native.current_vm
        || (origin_kind == core::OperationKind::Create) != origin_plan.restore_from.is_none()
    {
        return Err(JournalError::CorruptStore);
    }
    Ok(())
}
