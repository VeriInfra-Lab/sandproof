//! Mutations and their adjacent original physical preconditions.
use super::{
    Duration, ENVD_URL, Fence, File, HostObservation, Instant, LinuxWorker, NodeResult,
    Observation, ObservedResult, RuntimeError, SlotBinding, Stdio, UnknownReason, VmState,
    WorkAction, WorkOutcome, WorkReceipt, WorkerTransportError, check_trusted_path, core,
    decisions, fence_can_advance, fs, node_staging, thread,
};

/// One execution's receipt and payload-free diagnostic. No shared error slot is
/// overwritten when independent sandboxes execute concurrently.
#[derive(Debug)]
pub struct ExecutionReport {
    pub receipt: WorkReceipt<HostObservation>,
    pub diagnostic: Option<sandproof_controller_service::application::fault::FaultRecord>,
}

impl LinuxWorker {
    /// Execute one already claimed action. Any physical uncertainty becomes an
    /// Unknown receipt, never a fabricated success or an automatic retry.
    /// The caller coordinates actions for the same sandbox; sharing this worker
    /// does not grant permission to execute a second action for that sandbox.
    /// # Errors
    /// The current in-process worker transport is infallible; the result type
    /// remains aligned with the controller's fallible worker interface.
    pub fn execute(
        &self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<WorkReceipt<HostObservation>, WorkerTransportError> {
        Ok(self.execute_diagnosed(action).receipt)
    }

    /// Execute through the same physical path and return this call's diagnostic.
    /// A diagnostic reports uncertainty, never permission to repeat an effect.
    #[must_use]
    pub fn execute_diagnosed(&self, action: &WorkAction<core::EffectId>) -> ExecutionReport {
        let (outcome, diagnostic) = match self.perform(action) {
            Ok(evidence) => (WorkOutcome::ObservedSuccess(evidence), None),
            Err(error) => {
                let fault = crate::fault::project(
                    sandproof_controller_service::application::fault::FaultStage::Execute,
                    sandproof_controller_service::application::fault::FaultContext::action(action),
                    error.as_ref(),
                );
                fault.emit();
                (
                    WorkOutcome::Unknown {
                        reason: UnknownReason::ObservationUnavailable,
                        last_observation: None,
                    },
                    Some(fault),
                )
            }
        };
        ExecutionReport {
            receipt: WorkReceipt {
                identity: action.identity.clone(),
                outcome,
            },
            diagnostic,
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "closed V1 command dispatch keeps each physical precondition adjacent to its exact observation"
    )]
    pub(super) fn perform(
        &self,
        action: &WorkAction<core::EffectId>,
    ) -> NodeResult<HostObservation> {
        self.validate_action(action)?;
        let command = action.command.command();
        let result = match command {
            core::Command::InstallFence {
                sandbox,
                epoch,
                terminal,
            } => {
                let path = self.fence_path(*sandbox);
                let next = Fence {
                    epoch: *epoch,
                    terminal: *terminal,
                };
                if path.exists() {
                    let prior = self.fence(*sandbox)?;
                    if !fence_can_advance(prior, next) {
                        return Err("stale or reopened host fence".into());
                    }
                    if prior == next {
                        self.sync_node_record(&path)?;
                    } else {
                        self.replace_node_json(&path, &next)?;
                    }
                } else {
                    self.write_node_json(&path, &next)?;
                }
                ObservedResult::Fence(self.fence(*sandbox)?)
            }
            core::Command::PrepareVm {
                sandbox,
                epoch,
                generation,
                vm,
                slot,
                source,
            } => {
                if source.is_some() {
                    return Err("snapshot restore is outside V1".into());
                }
                if let Some(template) = &self.secret(*sandbox, *generation)?.template {
                    self.validate_template(template)?;
                }
                self.vm_uid(*vm)?;
                self.require_fence(*sandbox, *epoch, false)?;
                let binding = SlotBinding {
                    owner: action.identity.owner.clone(),
                    sandbox: *sandbox,
                    generation: *generation,
                    vm: *vm,
                    slot: *slot,
                };
                self.write_node_json(&self.slot_path(*slot), &binding)?;
                let observation = self.runtime(*sandbox, *vm)?.launch(&self.vm_name(*vm))?;
                if observation.state != VmState::VmmRunning
                    || observation.pid == 0
                    || observation.start_time_ticks == 0
                {
                    return Err("VM has not entered running state".into());
                }
                ObservedResult::Prepared {
                    pid: observation.pid,
                    start_ticks: observation.start_time_ticks,
                    boot_id: self.source_boot_id(*sandbox, *vm, &observation)?,
                }
            }
            core::Command::AbortPrepare {
                sandbox,
                epoch,
                generation,
                vm,
                slot,
            } => {
                self.require_fence(*sandbox, *epoch, true)?;
                self.ensure_prepare_slot(
                    &action.identity.owner,
                    *sandbox,
                    *generation,
                    *vm,
                    *slot,
                )?;
                let name = self.vm_name(*vm);
                let runtime = self.runtime(*sandbox, *vm)?;
                match runtime.observe(&name) {
                    Err(RuntimeError::MissingRecord) => {
                        self.reclaim_aborted_prepare(*sandbox, *generation, *vm, *slot)?;
                    }
                    Ok(Observation {
                        state: VmState::Stopped,
                        ..
                    }) => {
                        self.reclaim_stopped_prepare(*sandbox, *generation, *vm, *slot)?;
                    }
                    Ok(_) => {
                        // Runtime::stop uses a pidfd and the durable process
                        // start identity; no name-only or PID-only signal.
                        if runtime.stop(&name)?.state != VmState::Stopped {
                            return Err("exact VM exit not observed".into());
                        }
                        self.reclaim_stopped_prepare(*sandbox, *generation, *vm, *slot)?;
                    }
                    Err(error) => return Err(error.into()),
                }
                self.observe_aborted_prepare(*sandbox, *epoch, *generation, *vm, *slot)?
            }
            core::Command::ConfigureNetwork {
                sandbox,
                epoch,
                generation,
                vm,
                slot,
                policy,
            } => {
                self.require_fence(*sandbox, *epoch, false)?;
                self.require_slot(*sandbox, *generation, *vm, *slot)?;
                self.running(*sandbox, *vm)?;
                if !matches!(policy, core::NetworkPolicy::DenyAll) {
                    self.runtime(*sandbox, *vm)?
                        .configure_ipv4_uplink(&self.vm_name(*vm), policy)?;
                }
                self.observe_network(*sandbox, *epoch, *generation, *vm, *slot, policy)?
            }
            core::Command::ApplyFilePolicy {
                sandbox,
                epoch,
                generation,
                policy,
                ..
            } => self.observe_file_policy(*sandbox, *epoch, *generation, *policy)?,
            core::Command::ActivateVm {
                sandbox,
                epoch,
                generation,
                vm,
            } => {
                let after = self.complete_guest_activation(*sandbox, *epoch, *generation, *vm)?;
                ObservedResult::Activated {
                    pid: after.pid,
                    start_ticks: after.start_time_ticks,
                    boot_id: self.source_boot_id(*sandbox, *vm, &after)?,
                }
            }
            core::Command::StopVm {
                sandbox,
                epoch,
                generation,
                vm,
            } => {
                self.require_fence(*sandbox, *epoch, true)?;
                if self.vm_for_workspace(*sandbox, *generation)? != *vm {
                    return Err("stop VM binding mismatch".into());
                }
                let observation = self.runtime(*sandbox, *vm)?.stop(&self.vm_name(*vm))?;
                if observation.state != VmState::Stopped {
                    return Err("VM stop not observed".into());
                }
                ObservedResult::Stopped {
                    pid: observation.pid,
                    start_ticks: observation.start_time_ticks,
                }
            }
            core::Command::CleanupVm { .. } => {
                self.reclaim_incomplete_cleanup(action)?;
                ObservedResult::VmCleaned
            }
            core::Command::CleanupSlot {
                sandbox,
                generation,
                slot,
            } => self.observe_closed_slot(*sandbox, *generation, *slot)?,
            _ => return Err("command is outside the V1 host executor".into()),
        };
        Ok(HostObservation {
            resource: action.identity.resource.clone(),
            command: command.clone(),
            result,
        })
    }

    pub(super) fn complete_guest_activation(
        &self,
        sandbox: core::SandboxId,
        epoch: core::Epoch,
        generation: core::Generation,
        vm: core::VmId,
    ) -> NodeResult<Observation> {
        self.require_fence(sandbox, epoch, false)?;
        if self.vm_for_workspace(sandbox, generation)? != vm {
            return Err("activation VM binding mismatch".into());
        }
        let before = self.running(sandbox, vm)?;
        self.initialize_guest(sandbox, vm)?;
        self.start_template_once(sandbox, vm, &before)?;
        let after = self.running(sandbox, vm)?;
        if !decisions::process_identity_matches(&before, &after) {
            return Err("VM changed while initializing guest service".into());
        }
        Ok(after)
    }

    pub(super) fn initialize_guest(
        &self,
        sandbox: core::SandboxId,
        vm: core::VmId,
    ) -> NodeResult<()> {
        let name = self.vm_name(vm);
        let secret = self.secret_for_vm(sandbox, vm)?;
        self.initialize_guest_authorization(&name, &secret)
    }

    // Both callers obtain this exact immutable generation record from the
    // original VM binding. Restore keeps the same value for its final check.
    pub(super) fn initialize_guest_authorization(
        &self,
        name: &str,
        secret: &super::GuestSecret,
    ) -> NodeResult<()> {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if self.guest_status(name, "GET", "/health", None)? == "204" {
                break;
            }
            if Instant::now() >= deadline {
                return Err("envd health deadline exceeded".into());
            }
            thread::sleep(Duration::from_millis(100));
        }
        let startup = secret
            .template
            .as_ref()
            .and_then(|value| value.startup.as_ref());
        let init = crate::startup::init_payload(&secret.token, &secret.env_vars, startup);
        let body = serde_json::to_vec(&init)?;
        if self.guest_status(name, "POST", "/init", Some(&body))? != "204" {
            return Err("envd rejected authenticated initialization".into());
        }
        // A debug-only crash window after envd accepted the request but
        // before the host can record authenticated readiness.
        #[cfg(debug_assertions)]
        if std::env::var("SANDPROOF_TEST_EXIT_AFTER_GUEST_INIT")
            .ok()
            .as_deref()
            == Some("activate_vm")
        {
            std::process::exit(88);
        }
        if self.guest_file_status(name, &secret.token)? != "200" {
            return Err("envd authenticated file access not ready".into());
        }
        Ok(())
    }

    /// Intent is durable before dispatch. Recovery reuses confirmed dispatch,
    /// never repeats an unknown one. Start confirmation is not app readiness.
    pub(super) fn start_template_once(
        &self,
        sandbox: core::SandboxId,
        vm: core::VmId,
        observation: &Observation,
    ) -> NodeResult<()> {
        let secret = self.secret_for_vm(sandbox, vm)?;
        let Some(template) = secret.template else {
            return Ok(());
        };
        let Some(startup) = &template.startup else {
            return Ok(());
        };
        if startup.argv.is_empty() {
            return Ok(());
        }
        let path = self
            .config
            .state_dir
            .join("startup")
            .join(format!("{}.json", self.vm_name(vm)));
        let mut expected = crate::startup::StartupRecord {
            sandbox: sandbox.0,
            vm: vm.0,
            vmm_pid: observation.pid,
            start_ticks: observation.start_time_ticks,
            template: template.clone(),
            env_vars: secret.env_vars,
            guest_pid: None,
        };
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                check_trusted_path(&path)?;
                self.sync_node_record(&path)?;
                let stored: crate::startup::StartupRecord =
                    serde_json::from_reader(File::open(&path)?)?;
                return stored.confirmed_for(&expected);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        self.write_node_json(&path, &expected)?;
        let pid = self.dispatch_startup(&self.vm_name(vm), &secret.token, startup)?;
        #[cfg(debug_assertions)]
        if std::env::var("SANDPROOF_TEST_EXIT_AFTER_TEMPLATE_START")
            .ok()
            .as_deref()
            == Some("activate_vm")
        {
            std::process::exit(88);
        }
        expected.guest_pid = Some(pid);
        self.replace_node_json(&path, &expected)
    }

    pub(super) fn dispatch_startup(
        &self,
        name: &str,
        token: &str,
        startup: &sandproof_controller_service::templates::TemplateStartup,
    ) -> NodeResult<u32> {
        // Neither the credential nor tenant argv enters host process argv.
        // Bodies are retired by the exact per-incarnation manifest.
        let payload = crate::startup::start_request(startup)?;
        let durable_path = self
            .config
            .state_dir
            .join("startup")
            .join(format!("{name}.request"));
        node_staging::write(&self.config, &durable_path, &payload, false)?;
        let (gate, mut command) = self.namespace_command(name)?;
        command
            .args([
                "netns",
                "exec",
                name,
                "/usr/bin/curl",
                "-q",
                "--noproxy",
                "*",
                "-fsN",
                "--max-time",
                "5",
                "-K",
                "-",
                "-H",
                "Content-Type: application/connect+json",
                "-H",
                "Connect-Protocol-Version: 1",
                "--data-binary",
            ])
            .arg(format!("@{}", durable_path.display()))
            .arg(format!("{ENVD_URL}/process.Process/Start"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped());
        let mut child = self.spawn_namespace_command(name, &gate, &mut command)?;
        let output = child.exchange_until(
            format!("header = \"X-Access-Token: {token}\"\n").as_bytes(),
            sandbox_runtime::HelperLimits {
                stdout_bytes: 64 * 1024,
                ..sandbox_runtime::HelperLimits::CONTROL
            },
            crate::startup::first_frame_complete,
        )?;
        // envd deliberately detaches child lifetime from the HTTP stream.
        // Close the observation stream; never send a guest termination here.
        // Tracked launch execs curl in the recorded SAME PID; stopping this
        // exact child cannot orphan a supervisor's separate descendant.
        gate.finish_tracked()?;
        crate::startup::read_started(std::io::Cursor::new(output.stdout))
    }
}
