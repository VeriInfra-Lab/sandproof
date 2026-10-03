//! Opt-in root-only acceptance driver for real, previously persisted actions.
//! Stop the owning server first; a separate observer must verify the surviving
//! VMM identities and state/claim contents before and after this process.
//! This output proves rejection, not cleanup or absence of host mutations.

#[cfg(any(target_os = "linux", test))]
mod probe {
    use sandbox_controller_core as core;
    use sandproof_controller_service::HostSlotContext;
    use sandproof_protocol::{WorkAction, WorkOutcome};
    use serde::{Deserialize, Serialize};

    const MAX_INPUT_BYTES: u64 = 1024 * 1024;
    const MAX_ACTIONS: usize = 64;

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        version: u32,
        old_context: HostSlotContext,
        actions: Vec<WorkAction<core::EffectId>>,
    }

    #[derive(Default, Serialize)]
    pub(super) struct Report {
        version: u32,
        context_rejected: bool,
        release_rejected: bool,
        all_actions_rejected: bool,
        checked_actions: usize,
    }

    impl Input {
        fn validate(&self, namespace: &str) -> Result<(), &'static str> {
            if self.version != 1 || self.actions.is_empty() || self.actions.len() > MAX_ACTIONS {
                return Err("invalid input version or action count");
            }
            let binding = self.old_context.binding;
            let owner = format!("tenant-{}", binding.owner.0);
            let sandbox = format!("sb-{namespace}-{}", binding.sandbox.0);
            for action in &self.actions {
                let command = action.command.command();
                if action.identity.owner.as_str() != owner
                    || action.identity.sandbox.as_str() != sandbox
                    || action.identity.generation != binding.generation
                    || command.sandbox() != binding.sandbox
                {
                    return Err("action is outside the old binding");
                }
                // Restrict the driver to the old V1 host actions. The actual
                // protocol/epoch/resource checks remain LinuxWorker's own.
                let matches_binding = match command {
                    core::Command::InstallFence { .. } => true,
                    core::Command::PrepareVm {
                        generation,
                        vm,
                        slot,
                        source,
                        ..
                    } => {
                        *generation == binding.generation
                            && *vm == binding.vm
                            && *slot == binding.network
                            && source.is_none()
                    }
                    core::Command::ConfigureNetwork {
                        generation,
                        vm,
                        slot,
                        ..
                    }
                    | core::Command::AbortPrepare {
                        generation,
                        vm,
                        slot,
                        ..
                    } => {
                        *generation == binding.generation
                            && *vm == binding.vm
                            && *slot == binding.network
                    }
                    core::Command::ActivateVm { generation, vm, .. }
                    | core::Command::StopVm { generation, vm, .. }
                    | core::Command::CleanupVm { generation, vm, .. } => {
                        *generation == binding.generation && *vm == binding.vm
                    }
                    core::Command::ApplyFilePolicy { generation, .. } => {
                        *generation == binding.generation
                    }
                    core::Command::CleanupSlot {
                        generation, slot, ..
                    } => *generation == binding.generation && *slot == binding.network,
                    _ => false,
                };
                if !matches_binding {
                    return Err("action is outside the old V1 host resources");
                }
            }
            Ok(())
        }
    }

    fn rejected<E>(outcome: &WorkOutcome<E>) -> bool {
        matches!(
            outcome,
            WorkOutcome::Unknown { .. } | WorkOutcome::DefinitiveFailure { .. }
        )
    }

    fn bounded_json<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, &'static str> {
        if bytes.len() as u64 > MAX_INPUT_BYTES {
            return Err("input exceeds its byte bound");
        }
        serde_json::from_slice(bytes).map_err(|_| "invalid input JSON")
    }

    #[cfg(target_os = "linux")]
    fn private_json<T: serde::de::DeserializeOwned>(
        path: &std::path::Path,
    ) -> Result<T, &'static str> {
        use std::fs::{self, File};
        use std::io::Read;
        use std::os::unix::fs::MetadataExt;

        // Reuse the runtime's component-by-component root-owned, non-symlink,
        // non-writable-ancestor check; the input itself must also be private.
        sandbox_runtime::check_trusted_path(path).map_err(|_| "untrusted input path")?;
        let current = fs::symlink_metadata(path).map_err(|_| "input metadata unavailable")?;
        if !current.is_file()
            || current.uid() != 0
            || current.mode() & 0o077 != 0
            || current.nlink() != 1
            || current.len() > MAX_INPUT_BYTES
        {
            return Err("input is not a bounded private regular file");
        }
        let file = File::open(path).map_err(|_| "input unavailable")?;
        let metadata = file.metadata().map_err(|_| "input metadata unavailable")?;
        if !metadata.is_file()
            || metadata.uid() != 0
            || metadata.mode() & 0o077 != 0
            || metadata.nlink() != 1
            || metadata.len() > MAX_INPUT_BYTES
            || (metadata.dev(), metadata.ino()) != (current.dev(), current.ino())
        {
            return Err("input is not a bounded private regular file");
        }
        let mut bytes = Vec::new();
        file.take(MAX_INPUT_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "input read failed")?;
        bounded_json(&bytes)
    }

    #[cfg(target_os = "linux")]
    fn run(report: &mut Report) -> Result<(), &'static str> {
        use sandbox_controller_core::maintenance::slots::HostSlotPhase;
        use sandproof_node::{LinuxWorker, NodeConfig};
        use std::{env, path::Path};

        if rustix::process::geteuid().as_raw() != 0 {
            return Err("root is required");
        }
        let mut arguments = env::args_os().skip(1);
        let config_path = arguments.next().ok_or("two private input files required")?;
        let actions_path = arguments.next().ok_or("two private input files required")?;
        if arguments.next().is_some() {
            return Err("two private input files required");
        }
        let config: NodeConfig = private_json(Path::new(&config_path))?;
        let input: Input = private_json(Path::new(&actions_path))?;
        if config.bounded_host.is_none() {
            return Err("bounded host configuration is required");
        }
        input.validate(&config.namespace)?;
        // Initialization failure is failure, never evidence of stale rejection.
        let mut worker = LinuxWorker::new(config).map_err(|_| "worker initialization failed")?;
        let inventory = worker
            .host_slot_inventory()
            .map_err(|_| "host slot inventory unavailable")?;
        if !inventory.iter().any(|(context, phase)| {
            context.position == input.old_context.position
                && context.binding.vm > input.old_context.binding.vm
                && *phase == HostSlotPhase::Occupied
        }) {
            return Err("old position has no occupied successor");
        }
        report.context_rejected = worker.bind_host_slot(&input.old_context, false).is_err();
        if !report.context_rejected {
            return Err("old host context was accepted");
        }
        report.release_rejected = worker.release_host_slot(&input.old_context).is_err();
        if !report.release_rejected {
            return Err("old host release was accepted");
        }
        for action in &input.actions {
            if worker.observe_action(action).is_ok() {
                return Err("old host observation was accepted");
            }
            let receipt = worker
                .execute(action)
                .map_err(|_| "worker transport failed")?;
            if receipt.identity != action.identity || !rejected(&receipt.outcome) {
                return Err("old host action was accepted");
            }
            report.checked_actions += 1;
        }
        report.all_actions_rejected = report.checked_actions == input.actions.len();
        Ok(())
    }

    #[cfg(target_os = "linux")]
    pub(super) fn main() -> std::process::ExitCode {
        use std::io::Write;
        let mut report = Report {
            version: 1,
            ..Report::default()
        };
        let result = run(&mut report);
        let mut stdout = std::io::stdout().lock();
        if serde_json::to_writer(&mut stdout, &report).is_err() || writeln!(stdout).is_err() {
            eprintln!("report output failed");
            return std::process::ExitCode::FAILURE;
        }
        if let Err(reason) = result {
            // Only static error codes: never print actions, config or secrets.
            eprintln!("{reason}");
            std::process::ExitCode::FAILURE
        } else {
            std::process::ExitCode::SUCCESS
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use sandbox_controller_core::maintenance::slots::{HostBinding, HostSlotId};
        use sandproof_protocol::{ActionIdentity, ResourceId, ResourceKind, ResourceRef};

        fn input() -> Input {
            Input {
                version: 1,
                old_context: HostSlotContext {
                    position: HostSlotId(0),
                    expected_previous: None,
                    binding: HostBinding {
                        owner: core::TenantId(7),
                        sandbox: core::SandboxId(10),
                        generation: 1,
                        vm: core::VmId(10),
                        network: core::NetSlotId(10),
                    },
                },
                actions: vec![WorkAction {
                    identity: ActionIdentity {
                        protocol_version: 1,
                        owner: "tenant-7".try_into().unwrap(),
                        sandbox: "sb-0123456789abcdef-10".try_into().unwrap(),
                        operation: "op-1".try_into().unwrap(),
                        epoch: 1,
                        generation: 1,
                        logical_action: "effect-1".try_into().unwrap(),
                        attempt: 0,
                        resource: ResourceRef {
                            kind: ResourceKind::Vm,
                            id: ResourceId::try_from("vm-0123456789abcdef-10").unwrap(),
                        },
                    },
                    command: core::EffectId::initial(core::Command::StopVm {
                        sandbox: core::SandboxId(10),
                        epoch: 1,
                        generation: 1,
                        vm: core::VmId(10),
                    }),
                }],
            }
        }

        #[test]
        fn input_version_count_and_original_binding_are_required() {
            let original = input();
            assert!(original.validate("0123456789abcdef").is_ok());
            for change in 0..6 {
                let mut changed = input();
                match change {
                    0 => changed.version = 2,
                    1 => changed.actions.clear(),
                    2 => changed
                        .actions
                        .resize(MAX_ACTIONS + 1, original.actions[0].clone()),
                    3 => changed.actions[0].identity.owner = "tenant-8".try_into().unwrap(),
                    4 => changed.actions[0].identity.generation += 1,
                    _ => changed.old_context.binding.vm = core::VmId(11),
                }
                assert!(changed.validate("0123456789abcdef").is_err());
            }
            assert!(original.validate("fedcba9876543210").is_err());
        }

        #[test]
        fn observed_success_is_never_a_rejection() {
            assert!(!rejected(&WorkOutcome::ObservedSuccess(())));
            assert!(rejected(&WorkOutcome::<()>::Unknown {
                reason: sandproof_protocol::UnknownReason::ObservationUnavailable,
                last_observation: None,
            }));
            assert!(rejected(&WorkOutcome::<()>::DefinitiveFailure {
                code: sandproof_protocol::FailureCode::PolicyRejected,
                retryable: false,
                partial: None,
            }));
        }

        #[test]
        fn byte_limit_is_checked_before_json_decoding() {
            let mut bytes = vec![b' '; usize::try_from(MAX_INPUT_BYTES).unwrap()];
            bytes[..4].copy_from_slice(b"null");
            assert!(bounded_json::<serde_json::Value>(&bytes).is_ok());
            bytes.push(b' ');
            assert_eq!(
                bounded_json::<serde_json::Value>(&bytes).unwrap_err(),
                "input exceeds its byte bound"
            );
        }

        #[test]
        fn malformed_input_and_unchecked_reports_cannot_claim_success() {
            let original = input();
            let extra_field = serde_json::json!({
                "version": 1,
                "old_context": original.old_context,
                "actions": original.actions,
                "credential": "hidden",
            });
            assert!(serde_json::from_value::<Input>(extra_field).is_err());
            assert!(
                serde_json::from_slice::<Input>(br#"{"version":1,"credential":"hidden"}"#).is_err()
            );
            let report = Report {
                version: 1,
                ..Report::default()
            };
            let json = serde_json::to_value(report).unwrap();
            assert_eq!(json["all_actions_rejected"], false);
            assert_eq!(json["checked_actions"], 0);
        }
    }
}

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    probe::main()
}

#[cfg(not(target_os = "linux"))]
fn main() -> std::process::ExitCode {
    println!(
        "{{\"version\":1,\"context_rejected\":false,\"release_rejected\":false,\"all_actions_rejected\":false,\"checked_actions\":0}}"
    );
    eprintln!("Linux is required");
    std::process::ExitCode::FAILURE
}
