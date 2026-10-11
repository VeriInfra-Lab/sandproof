//! Payload-free projection before an adapter loses the concrete host error.
use sandbox_runtime::RuntimeError;
use sandproof_controller_service::application::{
    ApplicationError,
    fault::{FaultContext, FaultKind, FaultRecord, FaultStage, json_kind},
};
use std::{error::Error, io};

pub(crate) fn project(
    stage: FaultStage,
    context: FaultContext,
    error: &(dyn Error + 'static),
) -> FaultRecord {
    let mut current = Some(error);
    // Foreign implementations may contain cyclic source chains. Never format
    // them, and never let diagnostic classification traverse indefinitely.
    for _ in 0..8 {
        let Some(error) = current else { break };
        let mut record = FaultRecord::host(stage, FaultKind::HostInvariant, context);
        if let Some(error) = error.downcast_ref::<RuntimeError>() {
            record.kind = match error {
                RuntimeError::Io(error) => {
                    record.code = error.raw_os_error();
                    record.io_kind = Some(error.kind());
                    FaultKind::Io
                }
                RuntimeError::KvmUnavailable(error) => {
                    record.code = error.raw_os_error();
                    record.io_kind = Some(error.kind());
                    FaultKind::KvmUnavailable
                }
                RuntimeError::Json(error) => json_kind(error),
                RuntimeError::InvalidConfiguration(_) => FaultKind::InvalidConfiguration,
                RuntimeError::InvalidId => FaultKind::InvalidId,
                RuntimeError::AlreadyClaimed => FaultKind::AlreadyClaimed,
                RuntimeError::MissingRecord => FaultKind::MissingRecord,
                RuntimeError::IdentityMismatch(_) => FaultKind::IdentityMismatch,
                RuntimeError::TimedOut(_) => FaultKind::TimedOut,
                RuntimeError::HostCommand { .. } => FaultKind::HostCommand,
                RuntimeError::Api { status, .. } => {
                    record.code = Some(i32::from(*status));
                    FaultKind::FirecrackerApi
                }
                RuntimeError::Protocol(_) => FaultKind::Protocol,
            };
            return record;
        }
        if let Some(error) = error.downcast_ref::<io::Error>() {
            record.kind = FaultKind::Io;
            record.code = error.raw_os_error();
            record.io_kind = Some(error.kind());
            return record;
        }
        if let Some(error) = error.downcast_ref::<serde_json::Error>() {
            record.kind = json_kind(error);
            return record;
        }
        if error
            .downcast_ref::<crate::RouteCapacityExceeded>()
            .is_some()
        {
            record.kind = FaultKind::RouteCapacity;
            return record;
        }
        current = error.source();
    }
    FaultRecord::host(stage, FaultKind::HostInvariant, context)
}

pub(crate) fn busy(
    error: &(dyn Error + 'static),
    stage: FaultStage,
    context: FaultContext,
) -> ApplicationError {
    project(stage, context, error).emit();
    ApplicationError::Busy
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt;

    #[derive(Debug)]
    struct Cyclic;
    impl fmt::Display for Cyclic {
        fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
            panic!("diagnostic must not format arbitrary errors")
        }
    }
    impl Error for Cyclic {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            Some(self)
        }
    }

    #[test]
    fn unknown_or_cyclic_errors_are_bounded_without_display() {
        let record = project(FaultStage::Observe, FaultContext::default(), &Cyclic);
        assert_eq!(record.kind, FaultKind::HostInvariant);
        assert!(record.to_string().len() < 320);
    }

    #[test]
    fn host_payloads_stay_out_of_diagnostics_but_types_and_codes_survive() {
        let secret = "token-secret /root/private guest-content\nforged-log";
        let context = FaultContext::instance(
            sandbox_controller_core::SandboxId(5),
            Some(sandbox_controller_core::OperationId(9)),
        );
        for (error, kind, code) in [
            (
                RuntimeError::HostCommand {
                    operation: secret,
                    stderr: secret.into(),
                },
                FaultKind::HostCommand,
                None,
            ),
            (
                RuntimeError::Api {
                    status: 409,
                    body: secret.into(),
                },
                FaultKind::FirecrackerApi,
                Some(409),
            ),
            (
                RuntimeError::IdentityMismatch(secret),
                FaultKind::IdentityMismatch,
                None,
            ),
            (
                RuntimeError::Io(io::Error::from_raw_os_error(13)),
                FaultKind::Io,
                Some(13),
            ),
            (
                RuntimeError::Json(serde_json::Error::io(io::Error::other(secret))),
                FaultKind::JsonIo,
                None,
            ),
        ] {
            let record = project(FaultStage::Execute, context, &error);
            assert_eq!(record.kind, kind);
            assert_eq!(record.code, code);
            assert_eq!(record.context, context);
            assert!(!record.to_string().contains("secret"));
            assert!(!record.to_string().contains('\n'));
        }
    }
}
