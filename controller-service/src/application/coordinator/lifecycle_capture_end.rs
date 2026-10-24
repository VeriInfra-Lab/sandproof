//! Observe original worker/terminal ends outside the manager lock. These
//! calls only save exact provider facts in the original source transaction.
use super::{ApplicationError,ControllerApplication,ControllerHost,NeverCrash,core,map_journal};
use crate::{JournalError,SourceWorkRecord};
use sandproof_protocol::lifecycle::AuthenticatedCenterContext;

impl<H:ControllerHost> ControllerApplication<H>{
    pub(super) fn maintain_lifecycle_materials(&self)->Result<(),ApplicationError>{
        let (sandboxes,host)={
            let inner=self.inner.lock().map_err(|_|ApplicationError::Busy)?;
            if inner.journal.lifecycle_inbox_configuration().map_err(map_journal)?.is_none(){return Ok(())}
            let values=inner.journal.world().map_err(map_journal)?.sandbox_records().iter().filter_map(|(sandbox,row)|{
                (row.terminal && row.phase==core::Phase::Destroyed).then_some(*sandbox)
            }).collect::<Vec<_>>();
            (values,std::sync::Arc::clone(&inner.host))
        };
        for sandbox in sandboxes{
            {
                let Ok(_active)=self.acquire_sandbox(sandbox) else {continue};
                match self.observe_capture_ends(sandbox,host.as_ref()){
                    Ok(())=>{},
                    Err(ApplicationError::Busy|ApplicationError::OutcomeUnknown|ApplicationError::Conflict)=>continue,
                    Err(error)=>return Err(error),
                }
            }
            // The C collector independently acquires the same sandbox/session
            // occupancy and rechecks every source, real pin and drain before I/O.
            match self.advance_terminal_lifecycle_material(sandbox){
                Ok(_) | Err(ApplicationError::Busy|ApplicationError::OutcomeUnknown|ApplicationError::Conflict)=>{},
                Err(error)=>return Err(error),
            }
        }
        Ok(())
    }
    fn pending_capture_ends(&self,sandbox:core::SandboxId)->Result<Vec<(SourceWorkRecord,AuthenticatedCenterContext)>,ApplicationError>{
        let inner=self.inner.lock().map_err(|_|ApplicationError::Busy)?;
        let Some(config)=inner.journal.lifecycle_inbox_configuration().map_err(map_journal)? else {return Ok(Vec::new())};
        let terminal=inner.journal.world().map_err(map_journal)?.sandbox_record(sandbox).is_some_and(|row|row.terminal);
        let mut pending=Vec::new();
        for record in inner.journal.lifecycle_source_work(&config.node).map_err(map_journal)?{
            if record.source_sandbox()!=sandbox || record.full_capture().is_none() || (record.capture_has_end() && !terminal){continue}
            let context=inner.journal.recorded_lifecycle_context(record.work.request(),&config.node).map_err(map_journal)?;
            pending.push((record,context));
        }
        Ok(pending)
    }

    // The caller owns ActiveSandbox. A provider response is revalidated and
    // persisted before cleanup or any material-retirement question is allowed.
    pub(super) fn observe_capture_ends(&self,sandbox:core::SandboxId,host:&H)->Result<(),ApplicationError>{
        for (record,context) in self.pending_capture_ends(sandbox)?{
            if !record.capture_has_end() && record.capture_worker().is_none(){
                let result=self.inner.lock().map_err(|_|ApplicationError::Busy)?.journal.record_lifecycle_capture_never_released(&record.work,&context,&mut NeverCrash);
                match result {
                    Ok(_) | Err(JournalError::Conflict|JournalError::UnresolvedAction)=>{},
                    Err(error)=>return Err(map_journal(error)),
                }
            }
            if !record.capture_has_end()
                && let (Some(action),Some(full),Some(worker))=(record.capture_action(),record.full_capture(),record.capture_worker())
                && let Some(observation)=host.observe_lifecycle_capture_request_end(action,full,record.work.requirement(),worker)
            {
                self.inner.lock().map_err(|_|ApplicationError::Busy)?.journal.record_lifecycle_capture_request_end(&record.work,&context,observation,&mut NeverCrash).map_err(map_journal)?;
                continue;
            }
            let question={
                let inner=self.inner.lock().map_err(|_|ApplicationError::Busy)?;
                match inner.journal.lifecycle_terminated_capture_work(&record.work){
                    Ok(work)=>Some(work),
                    Err(JournalError::Conflict|JournalError::UnresolvedAction|JournalError::UnsupportedInput)=>None,
                    Err(error)=>return Err(map_journal(error)),
                }
            };
            let Some(work)=question else {continue};
            let Some(observation)=host.observe_lifecycle_terminated_capture(&work) else {continue};
            self.inner.lock().map_err(|_|ApplicationError::Busy)?.journal.retire_lifecycle_terminated_capture(&record.work,observation,&mut NeverCrash).map_err(map_journal)?;
        }
        Ok(())
    }
}
