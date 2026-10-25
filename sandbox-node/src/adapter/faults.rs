//! Existing deterministic fault boundaries, absent from ordinary deployment releases.
use crate::HostObservation;
use sandbox_controller_core as core;
use sandproof_controller_service::WorkerTransportError;
use sandproof_protocol::{WorkAction, WorkOutcome, WorkReceipt};
use std::{fs, os::unix::fs::MetadataExt, path::PathBuf};

mod restore_gate;
pub(crate) use restore_gate::RestoreActivationGate;

pub(super) fn before_execute(action: &WorkAction<core::EffectId>) {
    // Debug-only deterministic crash after the outbox dispatch claim,
    // before any host side effect. Recovery must not assume the
    // action was either executed or safely retryable.
    #[cfg(debug_assertions)]
    {
        let target = std::env::var("SANDPROOF_TEST_EXIT_BEFORE_HOST_ACTION").ok();
        if matches!(
            (target.as_deref(), action.command.command()),
            (
                Some("open_fence"),
                core::Command::InstallFence {
                    terminal: false,
                    ..
                }
            ) | (Some("stop_vm"), core::Command::StopVm { .. })
                | (Some("activate_vm"), core::Command::ActivateVm { .. })
        ) {
            std::process::exit(87);
        }
    }
    // Test-only, one-shot synchronization point after the durable
    // revocation/fence and before the physical stop. It lets a second
    // HTTP kill arrive while the first owns the lifecycle lock.
    #[cfg(debug_assertions)]
    if matches!(action.command.command(), core::Command::StopVm { .. })
        && let Some(path) = std::env::var_os("SANDPROOF_TEST_STOP_GATE")
    {
        use std::io::Read;
        use std::os::unix::net::UnixListener;
        let path = PathBuf::from(path);
        let parent = path.parent().expect("stop gate must have a parent");
        let metadata = fs::metadata(parent).expect("stop gate parent missing");
        assert_eq!(metadata.uid(), 0, "stop gate parent must be root owned");
        assert_eq!(
            metadata.mode() & 0o077,
            0,
            "stop gate parent must be private"
        );
        let listener = UnixListener::bind(&path).expect("bind stop gate");
        let (mut peer, _) = listener.accept().expect("accept stop gate");
        let mut release = [0_u8; 1];
        peer.read_exact(&mut release)
            .expect("read stop gate release");
        assert_eq!(release, [b'g']);
        drop(listener);
        fs::remove_file(path).expect("remove stop gate");
    }
}

pub(super) fn after_execute(
    action: &WorkAction<core::EffectId>,
    result: &Result<WorkReceipt<HostObservation>, WorkerTransportError>,
) {
    // The integration test exits the *whole service* after a real
    // host result but before the journal records its receipt. Ordinary release
    // binaries omit it; the fixed controlled CI build enables debug assertions.
    #[cfg(debug_assertions)]
    if result
        .as_ref()
        .is_ok_and(|receipt| matches!(receipt.outcome, WorkOutcome::ObservedSuccess(_)))
    {
        let target = std::env::var("SANDPROOF_TEST_EXIT_AFTER_HOST_ACTION").ok();
        let command = action.command.command();
        if matches!(
            (target.as_deref(), command),
            (Some("prepare_vm"), core::Command::PrepareVm { .. })
                | (
                    Some("configure_network"),
                    core::Command::ConfigureNetwork { .. }
                )
                | (Some("stop_vm"), core::Command::StopVm { .. })
                | (Some("activate_vm"), core::Command::ActivateVm { .. })
                | (Some("abort_prepare"), core::Command::AbortPrepare { .. })
                | (Some("cleanup_vm"), core::Command::CleanupVm { .. })
        ) {
            std::process::exit(86);
        }
    }
}

pub(super) fn refuse_reclaim(action: &WorkAction<core::EffectId>, variable: &str) -> bool {
    std::env::var(variable)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        == Some(action.command.command().sandbox().0)
}
