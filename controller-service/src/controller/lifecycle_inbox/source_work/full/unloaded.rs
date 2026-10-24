//! Read only actual persisted material custody plus original runtime release.
use super::super::{ControllerJournal, authenticated};
use super::{JournalError, codec, core, key};
use sandproof_protocol::lifecycle::{
    AcceptedLifecycleIntent, AuthenticatedCenterContext, LifecycleOperation, UnloadedFacts,
};

impl ControllerJournal {
    /// No observed helper exit, absent VM row or terminal input implies Unloaded.
    /// # Errors
    /// Foreign/stale requests and corrupt original storage refuse the query.
    pub fn lifecycle_unloaded(
        &self,
        intent: &AcceptedLifecycleIntent,
        context: &AuthenticatedCenterContext,
        now_ms: i64,
    ) -> Result<Option<UnloadedFacts>, JournalError> {
        let world = self.world()?;
        let tx = self.journal.connection.unchecked_transaction()?;
        self.committed_world.check_identity(&tx)?;
        let inbox = super::super::super::codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        authenticated(&inbox, intent, context)?;
        let (s, e) = inbox.find(intent)?.ok_or(JournalError::Conflict)?;
        let current = inbox.state(s, e);
        if !current.current || current.terminal {
            return Ok(None);
        }
        let LifecycleOperation::Unload {
            source_incarnation,
            capture,
        } = intent.request.operation
        else {
            return Err(JournalError::Conflict);
        };
        let sources = codec::load(&tx)?.ok_or(JournalError::UnsupportedStore)?;
        let Some(record) = sources.records.iter().find(|r| r.work.request() == intent) else {
            return Ok(None);
        };
        let Some(work) = &record.capture else {
            return Ok(None);
        };
        let (Some(manifest_digest), Some(material_bytes)) = (work.completed, work.material_bytes)
        else {
            return Ok(None);
        };
        let key = key(record).ok_or(JournalError::CorruptStore)?;
        let row = world
            .sandbox_record(key.sandbox)
            .ok_or(JournalError::CorruptStore)?;
        let plan = world
            .operation_plan(key)
            .ok_or(JournalError::CorruptStore)?;
        if row.terminal
            || row.phase != core::Phase::Paused
            || row.current_vm.is_some()
            || row
                .current_operation
                .is_none_or(|op| op.id != key.operation)
            || row.durable_snapshot != plan.target_snapshot
            || !plan
                .capture
                .is_some_and(core::capture::CaptureHandoff::may_stop_original)
        {
            return Ok(None);
        }
        let resources = world.resource_index(key.sandbox);
        if resources.vms.iter().any(|vm| {
            world
                .vm_record(*vm)
                .is_none_or(|v| v.status != core::VmStatus::Reclaimed)
        }) || resources.slots.iter().any(|slot| {
            world
                .slot_record(*slot)
                .is_none_or(|s| !s.reclaimed || s.active)
        }) {
            return Ok(None);
        }
        let Some(local) = current.local else {
            return Err(JournalError::CorruptStore);
        };
        let lifetime = crate::controller::admission::read_lifetime(
            &tx,
            &self.namespace,
            key.sandbox,
            local.owner,
        )
        .map_err(|_| JournalError::CorruptStore)?
        .ok_or(JournalError::CorruptStore)?;
        if Some(lifetime) != record.work.anchor.native.lifetime
            || now_ms < 0
            || now_ms >= lifetime.deadline_ms
        {
            return Ok(None);
        }
        // Original ConfirmRelease is durable per sandbox; reusing that physical
        // position cannot erase this operation's completed release.
        let released = crate::controller::checkpoint::release_witnesses(&tx, &world)?;
        if !released.iter().any(|w| {
            w.context.binding.sandbox == key.sandbox
                && Some(w.context.binding.vm) == record.work.anchor.native.current_vm
                && w.context.binding.owner == local.owner
                && w.context.binding.generation == row.generation
        }) {
            return Ok(None);
        }
        let convert = |n: i64| u64::try_from(n).map_err(|_| JournalError::CorruptStore);
        let result = UnloadedFacts {
            source_incarnation,
            capture,
            manifest_digest,
            material_bytes,
            started_at_ms: convert(lifetime.started_at_ms.ok_or(JournalError::CorruptStore)?)?,
            end_at_ms: convert(lifetime.deadline_ms)?,
            max_end_at_ms: convert(lifetime.max_deadline_ms)?,
            startup_deadline_ms: convert(lifetime.startup_deadline_ms)?,
        };
        self.committed_world.check_identity(&tx)?;
        tx.commit()?;
        Ok(Some(result))
    }
}
