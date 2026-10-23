use super::{
    AcceptedLifecycleIntent, CreateReadiness, CreateReadinessPending, InternalLocalBinding,
    LifecycleCreateObservation, LifecycleReadyBasis, LifecycleReadyObservation,
    NativeCreateSnapshot, SourceProcessIdentity, core,
};
use crate::controller::lifecycle_inbox::{authenticated, codec};
use crate::controller::{
    admission, allocators, capture, data_credentials, identity, internal_inbox, storage,
};
use crate::{ControllerJournal, CreateOptions, HostEvidence, JournalError};
use rusqlite::Transaction;
use sandproof_protocol::{
    WorkAction, WorkOutcome, WorkReceipt,
    lifecycle::{AuthenticatedCenterContext, LifecycleOperation},
};
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReadyCandidate {
    pub view: LifecycleCreateObservation,
    pub actions: Vec<WorkAction<core::EffectId>>,
    receipts: Vec<Vec<u8>>,
    source: Option<SourceProcessIdentity>,
    credential: Option<(core::Generation, [u8; 32])>,
    options: Option<CreateOptions>,
    ready_fact: Option<sandproof_protocol::internal::lifecycle::LifecycleFact>,
}
impl ReadyCandidate {
    pub(crate) fn can_probe(&self) -> bool {
        self.actions.len() == 5
            && self.source.is_some()
            && self.credential.is_some()
            && self.view.readiness
                == CreateReadiness::Pending(CreateReadinessPending::HostUnobserved)
    }
}

fn expected_commands(
    world: &core::World,
    local: InternalLocalBinding,
) -> Result<Vec<core::Command>, JournalError> {
    let record = world
        .sandbox_record(local.sandbox)
        .ok_or(JournalError::CorruptStore)?;
    let key = core::OperationKey {
        sandbox: local.sandbox,
        operation: local.operation,
    };
    let plan = world
        .operation_plan(key)
        .ok_or(JournalError::CorruptStore)?;
    let vm = record.current_vm.ok_or(JournalError::CorruptStore)?;
    let slot = record.net_slot.ok_or(JournalError::CorruptStore)?;
    if !plan.sealed
        || plan.vm != Some(vm)
        || plan.slot != Some(slot)
        || plan.epoch != record.epoch
        || plan.generation != record.generation
        || plan.restore_from.is_some()
    {
        return Err(JournalError::CorruptStore);
    }
    Ok(vec![
        core::Command::InstallFence {
            sandbox: local.sandbox,
            epoch: plan.epoch,
            terminal: false,
        },
        core::Command::PrepareVm {
            sandbox: local.sandbox,
            epoch: plan.epoch,
            generation: plan.generation,
            vm,
            slot,
            source: None,
        },
        core::Command::ConfigureNetwork {
            sandbox: local.sandbox,
            epoch: plan.epoch,
            generation: plan.generation,
            vm,
            slot,
            policy: record.policy.clone(),
        },
        core::Command::ApplyFilePolicy {
            sandbox: local.sandbox,
            epoch: plan.epoch,
            generation: plan.generation,
            workspace: record.workspace,
            policy: record.file_policy,
        },
        core::Command::ActivateVm {
            sandbox: local.sandbox,
            epoch: plan.epoch,
            generation: plan.generation,
            vm,
        },
    ])
}

pub(in crate::controller) type StartupEvidence = (Vec<WorkAction<core::EffectId>>, Vec<Vec<u8>>);

pub(in crate::controller) fn startup_records<E: HostEvidence + DeserializeOwned>(
    tx: &Transaction<'_>,
    namespace: &str,
    world: &core::World,
    local: InternalLocalBinding,
) -> Result<StartupEvidence, JournalError> {
    let commands = expected_commands(world, local)?;
    let key = core::OperationKey {
        sandbox: local.sandbox,
        operation: local.operation,
    };
    let plan = world
        .operation_plan(key)
        .ok_or(JournalError::CorruptStore)?;
    let mut found: Vec<Option<(WorkAction<core::EffectId>, Vec<u8>)>> = vec![None; 5];
    let mut query = tx.prepare("SELECT action_json,receipt_json FROM outbox WHERE sandbox_id=?1 AND dispatch_state='receipted' ORDER BY id")?;
    for row in query.query_map(
        [identity::sandbox_id(namespace, local.sandbox)?.as_str()],
        |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Vec<u8>>(1)?)),
    )? {
        let (action_bytes, receipt_bytes) = row?;
        let action: WorkAction<core::EffectId> = serde_json::from_slice(&action_bytes)?;
        let Some(index) = commands
            .iter()
            .position(|command| command == action.command.command())
        else {
            continue;
        };
        let logical = world
            .logical_effect(&action.command.logical)
            .ok_or(JournalError::CorruptStore)?;
        if logical.current_attempt != action.command.attempt {
            continue;
        }
        if logical.origin != core::EffectOrigin::Operation(key)
            || !core::effect::effect_succeeded_check(world, action.command.command())
            || action.identity.owner != identity::owner_id(local.owner)?
            || action.identity.sandbox != identity::sandbox_id(namespace, local.sandbox)?
            || action.identity.operation != identity::operation_id(namespace, key)?
            || action.identity.epoch != plan.epoch
            || action.identity.generation != plan.generation
            || action.identity.attempt != action.command.attempt
            || action.identity.resource != identity::resource(namespace, action.command.command())?
        {
            return Err(JournalError::CorruptStore);
        }
        let receipt: WorkReceipt<E> = serde_json::from_slice(&receipt_bytes)?;
        receipt.validate_for(&action)?;
        if !matches!(receipt.outcome, WorkOutcome::ObservedSuccess(ref evidence) if evidence.validates(action.command.command()))
            || found[index].is_some()
        {
            return Err(JournalError::CorruptStore);
        }
        found[index] = Some((action, receipt_bytes));
    }
    let Some(found) = found.into_iter().collect::<Option<Vec<_>>>() else {
        return Err(JournalError::UnresolvedAction);
    };
    Ok(found.into_iter().unzip())
}

fn native_snapshot(
    tx: &Transaction<'_>,
    namespace: &str,
    world: &core::World,
    local: InternalLocalBinding,
    record: &core::SandboxRecord,
) -> Result<NativeCreateSnapshot, JournalError> {
    if record.owner != local.owner {
        return Err(JournalError::CorruptStore);
    }
    Ok(NativeCreateSnapshot {
        owner: record.owner,
        phase: record.phase,
        terminal: record.terminal,
        epoch: record.epoch,
        generation: record.generation,
        current_vm: record.current_vm,
        current_operation: record.current_operation,
        lifetime: admission::read_lifetime(tx, namespace, local.sandbox, local.owner)
            .map_err(|_| JournalError::CorruptStore)?,
        uncertain_actions: world
            .effect_ids()
            .iter()
            .filter(|effect| {
                effect.command().sandbox() == local.sandbox
                    && matches!(
                        world.effect_status(effect),
                        Some(core::EffectStatus::Dispatched | core::EffectStatus::OutcomeUnknown)
                    )
            })
            .cloned()
            .collect(),
    })
}

pub(in crate::controller) fn base_snapshot(
    tx: &Transaction<'_>,
    namespace: &str,
    world: &core::World,
    intent: &AcceptedLifecycleIntent,
    context: &AuthenticatedCenterContext,
) -> Result<ReadyCandidate, JournalError> {
    let inbox = codec::load(tx)?.ok_or(JournalError::UnsupportedStore)?;
    authenticated(&inbox, intent, context)?;
    let (s, _) = inbox.find(intent)?.ok_or(JournalError::Conflict)?;
    if !matches!(intent.request.operation, LifecycleOperation::Create { .. }) {
        return Err(JournalError::UnsupportedInput);
    }
    let session = &inbox.sessions[s];
    let current_entry = session
        .entries
        .iter()
        .find(|entry| entry.intent.accepted_version == session.current_version)
        .ok_or(JournalError::CorruptStore)?;
    let current_request = current_entry.intent.clone();
    let local = session.local;
    let native = local
        .and_then(|local| {
            world
                .sandbox_record(local.sandbox)
                .map(|record| (local, record))
        })
        .map(|(local, record)| native_snapshot(tx, namespace, world, local, record))
        .transpose()?;
    let historical_result = local
        .and_then(|local| {
            world.operation_record(core::OperationKey {
                sandbox: local.sandbox,
                operation: local.operation,
            })
        })
        .and_then(|record| record.result);
    let applied_renewal = match (
        &intent.request.operation,
        &current_request.request.operation,
    ) {
        (
            LifecycleOperation::Create {
                target_incarnation, ..
            },
            LifecycleOperation::Renew { incarnation, .. },
        ) => {
            target_incarnation == incarnation
                && current_entry.applied_deadline.is_some()
                && current_entry.applied_deadline
                    == native
                        .as_ref()
                        .and_then(|n| n.lifetime.map(|lifetime| lifetime.deadline_ms))
        }
        _ => false,
    };
    let readiness = if let Some(reason) = session.refusal {
        CreateReadiness::Refused { reason }
    } else if current_request == *intent || applied_renewal {
        CreateReadiness::Pending(CreateReadinessPending::NoLocalMapping)
    } else {
        CreateReadiness::NotCurrent
    };
    Ok(ReadyCandidate {
        view: LifecycleCreateObservation {
            request: intent.clone(),
            current_request,
            local,
            historical_result,
            native_input_high_water: allocators::input_high_water(tx)?,
            native,
            readiness,
        },
        actions: vec![],
        receipts: vec![],
        source: None,
        credential: None,
        options: None,
        ready_fact: None,
    })
}

fn fill_candidate<E: HostEvidence + DeserializeOwned>(
    candidate: &mut ReadyCandidate,
    tx: &Transaction<'_>,
    namespace: &str,
    world: &core::World,
    now_ms: i64,
) -> Result<(), JournalError> {
    if candidate.view.readiness == CreateReadiness::NotCurrent {
        return Ok(());
    }
    let Some(local) = candidate.view.local else {
        return Ok(());
    };
    let Some(native) = candidate.view.native.as_ref() else {
        candidate.view.readiness = CreateReadiness::NotRunning;
        return Ok(());
    };
    if native.terminal
        || matches!(
            native.phase,
            core::Phase::Pausing
                | core::Phase::Paused
                | core::Phase::Revoking
                | core::Phase::Destroyed
        )
    {
        candidate.view.readiness = CreateReadiness::NotRunning;
        return Ok(());
    }
    if storage::unresolved(world, local.sandbox) {
        candidate.view.readiness = CreateReadiness::Pending(CreateReadinessPending::WorkUnknown);
        return Ok(());
    }
    if !native.has_completed_original_create(local)
        || candidate.view.historical_result
            != Some(core::OperationResult::Running(native.generation))
    {
        candidate.view.readiness =
            CreateReadiness::Pending(CreateReadinessPending::StartupIncomplete);
        return Ok(());
    }
    let lifetime = native.lifetime.ok_or(JournalError::CorruptStore)?;
    if lifetime.deadline_ms <= now_ms {
        candidate.view.readiness = CreateReadiness::Expired;
        return Ok(());
    }
    if lifetime
        .started_at_ms
        .is_none_or(|started| started > now_ms || started > lifetime.startup_deadline_ms)
    {
        candidate.view.readiness =
            CreateReadiness::Pending(CreateReadinessPending::OriginalEvidenceMissing);
        return Ok(());
    }
    candidate.ready_fact = internal_inbox::native_ready_fact(tx, namespace, world, local)?;
    let observed =
        capture::original_source::<E>(tx, namespace, world, local.sandbox).and_then(|source| {
            startup_records::<E>(tx, namespace, world, local)
                .map(|(actions, receipts)| (source, actions, receipts))
        });
    let (source, actions, receipts) = match observed {
        Ok(found) => found,
        Err(JournalError::UnresolvedAction) => {
            candidate.view.readiness =
                CreateReadiness::Pending(CreateReadinessPending::OriginalEvidenceMissing);
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    candidate.source = Some(source);
    candidate.actions = actions;
    candidate.receipts = receipts;
    candidate.options = Some(
        admission::read_create_options(tx, namespace, local.sandbox)
            .map_err(|_| JournalError::CorruptStore)?,
    );
    candidate.credential =
        match data_credentials::snapshot(tx, world, namespace, local.sandbox, true) {
            Ok(value) => value,
            Err(JournalError::CredentialIndexPoisoned) => None,
            Err(error) => return Err(error),
        };
    candidate.view.readiness = CreateReadiness::Pending(
        if candidate
            .credential
            .is_some_and(|(generation, _)| generation == native.generation)
        {
            CreateReadinessPending::HostUnobserved
        } else {
            CreateReadinessPending::CredentialUnobserved
        },
    );
    Ok(())
}

impl ControllerJournal {
    pub(crate) fn lifecycle_ready_candidate<E: HostEvidence + DeserializeOwned>(
        &self,
        intent: &AcceptedLifecycleIntent,
        context: &AuthenticatedCenterContext,
        now_ms: i64,
    ) -> Result<ReadyCandidate, JournalError> {
        if now_ms < 0 {
            return Err(JournalError::UnsupportedInput);
        }
        let world = self.world()?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        codec::validate(&tx, &world)?;
        let mut candidate = base_snapshot(&tx, &self.namespace, &world, intent, context)?;
        fill_candidate::<E>(&mut candidate, &tx, &self.namespace, &world, now_ms)?;
        self.committed_world.check_identity(&tx)?;
        tx.commit()?;
        Ok(candidate)
    }

    pub(crate) fn finish_lifecycle_ready<E: HostEvidence + DeserializeOwned>(
        &self,
        before: &ReadyCandidate,
        context: &AuthenticatedCenterContext,
        probes: &[Option<WorkReceipt<E>>],
        credential_matches: bool,
        now_ms: i64,
    ) -> Result<LifecycleCreateObservation, JournalError> {
        let mut after =
            self.lifecycle_ready_candidate::<E>(&before.view.request, context, now_ms)?;
        if !after.can_probe() {
            return Ok(after.view);
        }
        if after != *before {
            after.view.readiness =
                CreateReadiness::Pending(CreateReadinessPending::SnapshotChanged);
            return Ok(after.view);
        }
        if !credential_matches {
            after.view.readiness =
                CreateReadiness::Pending(CreateReadinessPending::CredentialUnobserved);
            return Ok(after.view);
        }
        if probes.len() != after.actions.len() || !probes_match(&after, probes)? {
            return Ok(after.view);
        }
        let Some(sandproof_protocol::internal::lifecycle::LifecycleFact::Ready {
            started_at_ms,
            end_at_ms,
            max_end_at_ms,
        }) = after.ready_fact
        else {
            return Err(JournalError::CorruptStore);
        };
        let lifetime = after
            .view
            .native
            .as_ref()
            .and_then(|native| native.lifetime)
            .ok_or(JournalError::CorruptStore)?;
        after.view.readiness = CreateReadiness::Observed(LifecycleReadyObservation {
            started_at_ms,
            end_at_ms,
            max_end_at_ms,
            startup_deadline_ms: u64::try_from(lifetime.startup_deadline_ms)
                .map_err(|_| JournalError::CorruptStore)?,
            observed_at_ms: u64::try_from(now_ms).map_err(|_| JournalError::UnsupportedInput)?,
            source: after.source.ok_or(JournalError::CorruptStore)?,
            basis: LifecycleReadyBasis {
                native_input_high_water: after.view.native_input_high_water,
                original_startup_actions: after
                    .actions
                    .into_iter()
                    .map(|action| action.identity)
                    .collect(),
                original_startup_receipts: after
                    .receipts
                    .iter()
                    .map(|bytes| {
                        sandproof_protocol::lifecycle::ContentDigest::from_sha256(
                            Sha256::digest(bytes).into(),
                        )
                    })
                    .collect(),
            },
        });
        Ok(after.view)
    }
}

fn probes_match<E: HostEvidence>(
    candidate: &ReadyCandidate,
    probes: &[Option<WorkReceipt<E>>],
) -> Result<bool, JournalError> {
    for (action, receipt) in candidate.actions.iter().zip(probes) {
        let Some(receipt) = receipt else {
            return Ok(false);
        };
        receipt.validate_for(action)?;
        let WorkOutcome::ObservedSuccess(evidence) = &receipt.outcome else {
            return Ok(false);
        };
        if !evidence.validates(action.command.command()) {
            return Err(JournalError::InvalidHostEvidence);
        }
        if matches!(
            action.command.command(),
            core::Command::PrepareVm { .. } | core::Command::ActivateVm { .. }
        ) && evidence
            .source_process_identity(action.command.command(), &action.identity.resource)
            != candidate.source
        {
            return Ok(false);
        }
    }
    Ok(true)
}
