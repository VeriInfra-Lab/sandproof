//! Exact source-bound termination; durable permission belongs to the caller.
use super::{
    Observation, ProcessRecord, Runtime, RuntimeError, RuntimeProcessIdentity, STOP_TIMEOUT,
    Signal, VmState, errno_to_io, pidfd_send_signal, validate_id, verified_process,
    wait_pidfd_exit,
};

fn require_source(
    record: &ProcessRecord,
    expected: &RuntimeProcessIdentity,
) -> Result<(), RuntimeError> {
    expected.validate_fields()?;
    if record.source_identity()? != *expected {
        return Err(RuntimeError::IdentityMismatch(
            "stop original process binding",
        ));
    }
    if super::helper_gate::boot_id()? != expected.boot_id {
        return Err(RuntimeError::IdentityMismatch(
            "stop original host boot changed",
        ));
    }
    Ok(())
}

pub(super) fn stop_record(record: &ProcessRecord) -> Result<Observation, RuntimeError> {
    if let Some(pidfd) = verified_process(record)? {
        pidfd_send_signal(&pidfd, Signal::TERM).map_err(errno_to_io)?;
        if !wait_pidfd_exit(&pidfd, STOP_TIMEOUT)? {
            pidfd_send_signal(&pidfd, Signal::KILL).map_err(errno_to_io)?;
            if !wait_pidfd_exit(&pidfd, STOP_TIMEOUT)? {
                return Err(RuntimeError::TimedOut("exact VM process exit"));
            }
        }
    }
    Ok(exited(record))
}

fn exited(record: &ProcessRecord) -> Observation {
    Observation {
        resource_id: record.resource_id.clone(),
        pid: record.pid,
        start_time_ticks: record.start_time_ticks,
        state: VmState::Stopped,
    }
}

impl Runtime {
    /// Stop only the source bound to the caller's original persistent work.
    /// Matching fields are physical identity checks, not a durable Stop permit.
    /// The caller must first confirm V5's original bundle registration/Stop work
    /// and exclude late capture/other workers under the original admission gate.
    /// No bundle, jail, claim, network or capacity is deleted/released here.
    /// # Errors
    /// Missing/invalid/conflicting claims, changed host boot and timeouts remain
    /// unresolved. An error may follow a signal; observe only this same source.
    pub fn stop_original_vm(
        &self,
        id: &str,
        expected: &RuntimeProcessIdentity,
    ) -> Result<Observation, RuntimeError> {
        validate_id(id)?;
        let record = self.read_record(id)?;
        require_source(&record, expected)?;
        // Use this checked record throughout. Calling stop(id) here would read
        // a second record that was not compared with the supplied source.
        stop_record(&record)
    }

    /// Read-only execution-exit observation for the identical original source.
    /// Stopped includes an exited unreaped task, as in the existing stop API;
    /// this is not evidence of helper drain, PID-cell release or physical GC.
    /// # Errors
    /// A missing/changed claim, live original process or unavailable observation
    /// is unresolved, never inferred to be absent from a name or API timeout.
    pub fn observe_original_stopped(
        &self,
        id: &str,
        expected: &RuntimeProcessIdentity,
    ) -> Result<Observation, RuntimeError> {
        validate_id(id)?;
        let record = self.read_record(id)?;
        require_source(&record, expected)?;
        if verified_process(&record)?.is_some() {
            return Err(RuntimeError::IdentityMismatch("original VM is still live"));
        }
        Ok(exited(&record))
    }
}

#[cfg(test)]
#[path = "original_stop_tests.rs"]
mod tests;
