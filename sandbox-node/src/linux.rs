use rustix::process::geteuid;
use sandbox_controller_core as core;
use sandbox_runtime::{Runtime, RuntimeConfig, VmState, check_trusted_path};
use sandproof_controller_service::{HostEvidence, WorkerTransportError};
use sandproof_protocol::{
    ResourceEvidence, ResourceId, ResourceKind, ResourceRef, UnknownReason, WorkAction,
    WorkOutcome, WorkReceipt,
};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fs::{self, DirBuilder, File};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command as Process, Stdio};
use std::thread;
use std::time::{Duration, Instant};

type NodeResult<T> = Result<T, Box<dyn Error>>;
const ENVD_URL: &str = "http://172.16.0.2:49983";

/// Deployment inputs are fixed, root-owned host paths, not API request data.
#[derive(Clone, Debug)]
pub struct NodeConfig {
    pub namespace: String,
    pub state_dir: PathBuf,
    pub jailer: PathBuf,
    pub firecracker: PathBuf,
    pub chroot_base: PathBuf,
    pub kernel: PathBuf,
    pub rootfs: PathBuf,
    pub uid_base: u32,
    pub max_vm_id: u64,
    pub vcpu_count: u8,
    pub memory_mib: u32,
    /// V1 has one deployment-controlled managed-disk policy, not arbitrary
    /// user-supplied host paths or a directory ACL implementation.
    pub file_policy_digest: core::PolicyDigest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GuestSecret {
    token: String,
    sha512: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SlotBinding {
    sandbox: core::SandboxId,
    generation: core::Generation,
    vm: core::VmId,
    slot: core::NetSlotId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fence {
    epoch: core::Epoch,
    terminal: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum ObservedResult {
    Fence(Fence),
    Prepared { pid: u32, start_ticks: u64 },
    Network { namespace_inode: u64 },
    Activated { pid: u32, start_ticks: u64 },
    FilePolicy { disk_inode: u64 },
    Stopped { pid: u32, start_ticks: u64 },
    VmCleaned,
    SlotClosed,
}

/// Only the trusted worker constructs this from host checks. Serialization
/// stores the observation, but deserializing a guest reply is never authority.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostObservation {
    resource: ResourceRef,
    command: core::Command,
    result: ObservedResult,
}

impl ResourceEvidence for HostObservation {
    fn resource(&self) -> &ResourceRef {
        &self.resource
    }
}

impl HostEvidence for HostObservation {
    fn validates(&self, command: &core::Command) -> bool {
        if &self.command != command {
            return false;
        }
        match (command, &self.result) {
            (
                core::Command::InstallFence {
                    epoch, terminal, ..
                },
                ObservedResult::Fence(fence),
            ) => fence.epoch == *epoch && fence.terminal == *terminal,
            (core::Command::PrepareVm { .. }, ObservedResult::Prepared { pid, start_ticks })
            | (core::Command::ActivateVm { .. }, ObservedResult::Activated { pid, start_ticks })
            | (core::Command::StopVm { .. }, ObservedResult::Stopped { pid, start_ticks }) => {
                *pid != 0 && *start_ticks != 0
            }
            (
                core::Command::ConfigureNetwork { .. },
                ObservedResult::Network { namespace_inode },
            )
            | (
                core::Command::ApplyFilePolicy { .. },
                ObservedResult::FilePolicy {
                    disk_inode: namespace_inode,
                },
            ) => *namespace_inode != 0,
            (core::Command::CleanupVm { .. }, ObservedResult::VmCleaned)
            | (core::Command::CleanupSlot { .. }, ObservedResult::SlotClosed) => true,
            _ => false,
        }
    }
}

pub struct LinuxWorker {
    config: NodeConfig,
    last_error: Option<String>,
}

impl LinuxWorker {
    /// The state directory must already be private and root-owned. Secrets,
    /// slot bindings and fences are persisted below it with exact IDs.
    /// # Errors
    /// Rejects unsafe deployment paths and an unavailable host runtime.
    pub fn new(config: NodeConfig) -> NodeResult<Self> {
        if geteuid().as_raw() != 0 {
            return Err("node worker requires root on the isolated KVM host".into());
        }
        if config.namespace.len() != 16
            || !config
                .namespace
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err("invalid deployment namespace".into());
        }
        for path in [
            &config.state_dir,
            &config.jailer,
            &config.firecracker,
            &config.chroot_base,
            &config.kernel,
            &config.rootfs,
        ] {
            check_trusted_path(path)?;
        }
        let metadata = fs::metadata(&config.state_dir)?;
        if !metadata.is_dir() || metadata.uid() != 0 || metadata.mode() & 0o077 != 0 {
            return Err("node state directory must be root-only".into());
        }
        if config.uid_base == 0
            || config.max_vm_id == 0
            || config.vcpu_count == 0
            || config.memory_mib < 128
        {
            return Err("invalid node resource limits".into());
        }
        for subdir in ["secrets", "fences", "slots"] {
            let path = config.state_dir.join(subdir);
            if !path.exists() {
                DirBuilder::new().mode(0o700).create(&path)?;
            }
            check_trusted_path(&path)?;
            let metadata = fs::metadata(path)?;
            if !metadata.is_dir() || metadata.uid() != 0 || metadata.mode() & 0o077 != 0 {
                return Err("node state subdirectory must be root-only".into());
            }
        }
        for binary in ["/usr/bin/ip", "/usr/bin/curl", "/usr/bin/sha512sum"] {
            check_trusted_path(Path::new(binary))?;
        }
        Ok(Self {
            config,
            last_error: None,
        })
    }

    /// Create a one-time root-only credential before accepting a sandbox.
    /// The caller must use a CSPRNG and not log or pass the token in argv.
    /// # Errors
    /// Rejects reuse, malformed tokens, or a hash computation failure.
    pub fn put_secret(&self, sandbox: core::SandboxId, token: &str) -> NodeResult<()> {
        if token.len() < 32
            || token.len() > 128
            || !token
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        {
            return Err("invalid guest access token shape".into());
        }
        let sha512 = hash_token(token)?;
        let value = GuestSecret {
            token: token.to_owned(),
            sha512,
        };
        let path = self.secret_path(sandbox);
        write_new_json(&path, &value)
    }

    /// Read a credential only for the authenticated API response or worker.
    /// # Errors
    /// Rejects a missing or malformed stored secret.
    pub fn token_for(&self, sandbox: core::SandboxId) -> NodeResult<String> {
        Ok(self.secret(sandbox)?.token)
    }

    /// Execute one already claimed action. Any physical uncertainty becomes an
    /// Unknown receipt, never a fabricated success or an automatic retry.
    /// # Errors
    /// The current in-process worker transport is infallible; the result type
    /// remains aligned with the controller's fallible worker interface.
    pub fn execute(
        &mut self,
        action: &WorkAction<core::EffectId>,
    ) -> Result<WorkReceipt<HostObservation>, WorkerTransportError> {
        let result = self.perform(action);
        let outcome = match result {
            Ok(evidence) => {
                self.last_error = None;
                WorkOutcome::ObservedSuccess(evidence)
            }
            Err(error) => {
                self.last_error = Some(error.to_string());
                WorkOutcome::Unknown {
                    reason: UnknownReason::ObservationUnavailable,
                    last_observation: None,
                }
            }
        };
        Ok(WorkReceipt {
            identity: action.identity.clone(),
            outcome,
        })
    }

    #[must_use]
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    fn secret_path(&self, sandbox: core::SandboxId) -> PathBuf {
        self.config
            .state_dir
            .join("secrets")
            .join(format!("sb-{}-{}", self.config.namespace, sandbox.0))
    }

    fn secret(&self, sandbox: core::SandboxId) -> NodeResult<GuestSecret> {
        let path = self.secret_path(sandbox);
        check_trusted_path(&path)?;
        let value: GuestSecret = serde_json::from_reader(File::open(path)?)?;
        if value.token.len() < 32
            || value.sha512.len() != 128
            || hash_token(&value.token)? != value.sha512
        {
            return Err("stored guest secret hash mismatch".into());
        }
        Ok(value)
    }

    fn vm_name(&self, vm: core::VmId) -> String {
        format!("vm-{}-{}", self.config.namespace, vm.0)
    }

    fn slot_path(&self, slot: core::NetSlotId) -> PathBuf {
        self.config
            .state_dir
            .join("slots")
            .join(format!("slot-{}-{}.json", self.config.namespace, slot.0))
    }

    fn fence_path(&self, sandbox: core::SandboxId) -> PathBuf {
        self.config
            .state_dir
            .join("fences")
            .join(format!("sb-{}-{}.json", self.config.namespace, sandbox.0))
    }

    fn runtime(&self, sandbox: core::SandboxId, vm: core::VmId) -> NodeResult<Runtime> {
        let secret = self.secret(sandbox)?;
        let id = u32::try_from(vm.0)?;
        if vm.0 > self.config.max_vm_id {
            return Err("node VM ID exceeds allocated UID range".into());
        }
        let uid = self
            .config
            .uid_base
            .checked_add(id)
            .ok_or("VM uid overflow")?;
        let vm_name = self.vm_name(vm);
        Ok(Runtime::new(RuntimeConfig {
            jailer: self.config.jailer.clone(),
            firecracker: self.config.firecracker.clone(),
            chroot_base: self.config.chroot_base.clone(),
            kernel: self.config.kernel.clone(),
            rootfs: self.config.rootfs.clone(),
            vm_uid: uid,
            vm_gid: uid,
            vcpu_count: self.config.vcpu_count,
            memory_mib: self.config.memory_mib,
            daemonize: true,
            private_netns: Some(PathBuf::from(format!("/run/netns/{vm_name}"))),
            access_token_hash: Some(secret.sha512),
        })?)
    }

    fn slot(&self, slot: core::NetSlotId) -> NodeResult<SlotBinding> {
        let path = self.slot_path(slot);
        check_trusted_path(&path)?;
        let value: SlotBinding = serde_json::from_reader(File::open(path)?)?;
        if value.slot != slot {
            return Err("slot mapping identity mismatch".into());
        }
        Ok(value)
    }

    fn vm_for_workspace(
        &self,
        sandbox: core::SandboxId,
        generation: core::Generation,
    ) -> NodeResult<core::VmId> {
        let mut found = None;
        for entry in fs::read_dir(self.config.state_dir.join("slots"))? {
            let entry = entry?;
            let binding: SlotBinding = serde_json::from_reader(File::open(entry.path())?)?;
            if binding.sandbox == sandbox && binding.generation == generation {
                if found.is_some() {
                    return Err("ambiguous VM for workspace".into());
                }
                found = Some(binding.vm);
            }
        }
        found.ok_or_else(|| "no VM for workspace".into())
    }

    fn fence(&self, sandbox: core::SandboxId) -> NodeResult<Fence> {
        let path = self.fence_path(sandbox);
        check_trusted_path(&path)?;
        Ok(serde_json::from_reader(File::open(path)?)?)
    }

    fn require_fence(
        &self,
        sandbox: core::SandboxId,
        epoch: core::Epoch,
        terminal: bool,
    ) -> NodeResult<()> {
        let fence = self.fence(sandbox)?;
        if fence.epoch != epoch || fence.terminal != terminal {
            return Err("host fence does not match command".into());
        }
        Ok(())
    }

    #[expect(
        clippy::too_many_lines,
        reason = "closed V1 command dispatch keeps each physical precondition adjacent to its exact observation"
    )]
    fn perform(&self, action: &WorkAction<core::EffectId>) -> NodeResult<HostObservation> {
        action.identity.validate_version()?;
        let command = action.command.command();
        let generation = match command {
            core::Command::PrepareVm { generation, .. }
            | core::Command::ConfigureNetwork { generation, .. }
            | core::Command::ActivateVm { generation, .. }
            | core::Command::ApplyFilePolicy { generation, .. }
            | core::Command::StopVm { generation, .. }
            | core::Command::CleanupVm { generation, .. }
            | core::Command::CleanupSlot { generation, .. } => Some(*generation),
            _ => None,
        };
        if action.identity.attempt != action.command.attempt
            || action.identity.epoch != command.epoch().unwrap_or(action.identity.epoch)
            || generation.is_some_and(|value| value != action.identity.generation)
            || action.identity.sandbox.as_str()
                != format!("sb-{}-{}", self.config.namespace, command.sandbox().0)
            || action.identity.resource != self.resource(command)?
        {
            return Err("host action identity mismatch".into());
        }
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
                    if prior.epoch > *epoch
                        || (prior.epoch == *epoch && prior.terminal != *terminal)
                        || (prior.terminal && !terminal)
                    {
                        return Err("stale or reopened host fence".into());
                    }
                    if prior != next {
                        replace_json(&path, &next)?;
                    }
                } else {
                    write_new_json(&path, &next)?;
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
                self.require_fence(*sandbox, *epoch, false)?;
                let binding = SlotBinding {
                    sandbox: *sandbox,
                    generation: *generation,
                    vm: *vm,
                    slot: *slot,
                };
                write_new_json(&self.slot_path(*slot), &binding)?;
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
                }
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
                if *policy != core::NetworkPolicy::DenyAll {
                    return Err("only private no-egress network policy is implemented".into());
                }
                self.require_slot(*sandbox, *generation, *vm, *slot)?;
                self.running(*sandbox, *vm)?;
                let name = self.vm_name(*vm);
                let namespace = PathBuf::from(format!("/run/netns/{name}"));
                check_trusted_path(&namespace)?;
                let metadata = fs::symlink_metadata(&namespace)?;
                if !metadata.is_file() || metadata.uid() != 0 || metadata.ino() == 0 {
                    return Err("private network namespace identity mismatch".into());
                }
                let output = Process::new("/usr/bin/ip")
                    .args([
                        "netns",
                        "exec",
                        &name,
                        "/usr/bin/ip",
                        "-4",
                        "route",
                        "show",
                        "default",
                    ])
                    .output()?;
                if !output.status.success() || !output.stdout.is_empty() {
                    return Err("private namespace has external route or cannot be checked".into());
                }
                ObservedResult::Network {
                    namespace_inode: metadata.ino(),
                }
            }
            core::Command::ApplyFilePolicy {
                sandbox,
                epoch,
                generation,
                policy,
                ..
            } => {
                self.require_fence(*sandbox, *epoch, false)?;
                if policy.digest != self.config.file_policy_digest {
                    return Err("file policy digest is not the deployed V1 policy".into());
                }
                let vm = self.vm_for_workspace(*sandbox, *generation)?;
                self.running(*sandbox, vm)?;
                let disk = self
                    .config
                    .chroot_base
                    .join(
                        self.config
                            .firecracker
                            .file_name()
                            .ok_or("invalid firecracker name")?,
                    )
                    .join(self.vm_name(vm))
                    .join("root/rootfs.ext4");
                let metadata = fs::symlink_metadata(&disk)?;
                let uid = self
                    .config
                    .uid_base
                    .checked_add(u32::try_from(vm.0)?)
                    .ok_or("VM uid overflow")?;
                if !metadata.is_file()
                    || metadata.file_type().is_symlink()
                    || metadata.mode() & 0o777 != 0o600
                    || metadata.uid() != uid
                    || metadata.ino() == 0
                {
                    return Err("VM private disk policy not observed".into());
                }
                ObservedResult::FilePolicy {
                    disk_inode: metadata.ino(),
                }
            }
            core::Command::ActivateVm {
                sandbox,
                epoch,
                generation,
                vm,
            } => {
                self.require_fence(*sandbox, *epoch, false)?;
                if self.vm_for_workspace(*sandbox, *generation)? != *vm {
                    return Err("activation VM binding mismatch".into());
                }
                let observation = self.running(*sandbox, *vm)?;
                self.initialize_guest(*sandbox, *vm)?;
                let after = self.running(*sandbox, *vm)?;
                if after.pid != observation.pid
                    || after.start_time_ticks != observation.start_time_ticks
                {
                    return Err("VM changed while initializing guest service".into());
                }
                ObservedResult::Activated {
                    pid: after.pid,
                    start_ticks: after.start_time_ticks,
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
            core::Command::CleanupVm {
                sandbox,
                generation,
                vm,
            } => {
                if !self.fence(*sandbox)?.terminal
                    || self.vm_for_workspace(*sandbox, *generation)? != *vm
                {
                    return Err("cleanup requires terminal fence and exact VM".into());
                }
                let runtime = self.runtime(*sandbox, *vm)?;
                if runtime.observe(&self.vm_name(*vm))?.state != VmState::Stopped {
                    return Err("cannot clean live VM".into());
                }
                runtime.cleanup(&self.vm_name(*vm))?;
                if Path::new(&format!("/run/netns/{}", self.vm_name(*vm))).exists() {
                    return Err("network namespace remains after cleanup".into());
                }
                ObservedResult::VmCleaned
            }
            core::Command::CleanupSlot {
                sandbox,
                generation,
                slot,
            } => {
                if !self.fence(*sandbox)?.terminal {
                    return Err("slot cleanup requires terminal fence".into());
                }
                let binding = self.slot(*slot)?;
                if binding.sandbox != *sandbox || binding.generation != *generation {
                    return Err("slot cleanup binding mismatch".into());
                }
                if Path::new(&format!("/run/netns/{}", self.vm_name(binding.vm))).exists() {
                    return Err("slot namespace remains after cleanup".into());
                }
                if self
                    .runtime(*sandbox, binding.vm)?
                    .observe(&self.vm_name(binding.vm))?
                    .state
                    != VmState::Stopped
                {
                    return Err("slot VM remains alive".into());
                }
                ObservedResult::SlotClosed
            }
            _ => return Err("command is outside the V1 host executor".into()),
        };
        Ok(HostObservation {
            resource: action.identity.resource.clone(),
            command: command.clone(),
            result,
        })
    }

    fn resource(&self, command: &core::Command) -> NodeResult<ResourceRef> {
        let (kind, prefix, number) = match command {
            core::Command::InstallFence { sandbox, .. } => (ResourceKind::Sandbox, "sb", sandbox.0),
            core::Command::PrepareVm { vm, .. }
            | core::Command::ActivateVm { vm, .. }
            | core::Command::StopVm { vm, .. }
            | core::Command::CleanupVm { vm, .. } => (ResourceKind::Vm, "vm", vm.0),
            core::Command::ConfigureNetwork { slot, .. }
            | core::Command::CleanupSlot { slot, .. } => {
                (ResourceKind::NetworkSlot, "slot", slot.0)
            }
            core::Command::ApplyFilePolicy { workspace, .. } => {
                (ResourceKind::Workspace, "fs", workspace.0)
            }
            _ => return Err("unsupported host resource command".into()),
        };
        Ok(ResourceRef {
            kind,
            id: ResourceId::try_from(format!("{prefix}-{}-{number}", self.config.namespace))?,
        })
    }

    fn require_slot(
        &self,
        sandbox: core::SandboxId,
        generation: core::Generation,
        vm: core::VmId,
        slot: core::NetSlotId,
    ) -> NodeResult<()> {
        if self.slot(slot)?
            != (SlotBinding {
                sandbox,
                generation,
                vm,
                slot,
            })
        {
            return Err("slot binding mismatch".into());
        }
        Ok(())
    }

    fn running(
        &self,
        sandbox: core::SandboxId,
        vm: core::VmId,
    ) -> NodeResult<sandbox_runtime::Observation> {
        let observation = self.runtime(sandbox, vm)?.observe(&self.vm_name(vm))?;
        if observation.state != VmState::VmmRunning
            || observation.pid == 0
            || observation.start_time_ticks == 0
        {
            return Err("exact VM is not running".into());
        }
        Ok(observation)
    }

    fn initialize_guest(&self, sandbox: core::SandboxId, vm: core::VmId) -> NodeResult<()> {
        let name = self.vm_name(vm);
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if Self::guest_status(&name, "GET", "/health", None)? == "204" {
                break;
            }
            if Instant::now() >= deadline {
                return Err("envd health deadline exceeded".into());
            }
            thread::sleep(Duration::from_millis(100));
        }
        let token = self.secret(sandbox)?.token;
        let init = serde_json::json!({"accessToken": token, "defaultUser": "root", "defaultWorkdir": "/root"});
        let body = serde_json::to_vec(&init)?;
        if Self::guest_status(&name, "POST", "/init", Some(&body))? != "204" {
            return Err("envd rejected authenticated initialization".into());
        }
        if Self::guest_file_status(&name, &token)? != "200" {
            return Err("envd authenticated file access not ready".into());
        }
        Ok(())
    }

    fn guest_status(
        name: &str,
        method: &str,
        path: &str,
        input: Option<&[u8]>,
    ) -> NodeResult<String> {
        let mut command = Process::new("/usr/bin/ip");
        command.args([
            "netns",
            "exec",
            name,
            "/usr/bin/curl",
            "-q",
            "--noproxy",
            "*",
            "-s",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
            "--max-time",
            "5",
            "-X",
            method,
        ]);
        if input.is_some() {
            command.args([
                "-H",
                "Content-Type: application/json",
                "--data-binary",
                "@-",
            ]);
        }
        command.arg(format!("{ENVD_URL}{path}"));
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command.spawn()?;
        if let Some(input) = input {
            child
                .stdin
                .take()
                .ok_or("curl stdin unavailable")?
                .write_all(input)?;
        }
        let output = child.wait_with_output()?;
        if !output.status.success() {
            return Ok(String::new());
        }
        Ok(String::from_utf8(output.stdout)?.trim().to_owned())
    }

    fn guest_file_status(name: &str, token: &str) -> NodeResult<String> {
        let mut child = Process::new("/usr/bin/ip")
            .args([
                "netns",
                "exec",
                name,
                "/usr/bin/curl",
                "-q",
                "--noproxy",
                "*",
                "-s",
                "-o",
                "/dev/null",
                "-w",
                "%{http_code}",
                "--max-time",
                "5",
                "-K",
                "-",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let config = format!(
            "url = \"{ENVD_URL}/files?path=%2Fetc%2Fpasswd\"\nheader = \"X-Access-Token: {token}\"\n"
        );
        child
            .stdin
            .take()
            .ok_or("curl stdin unavailable")?
            .write_all(config.as_bytes())?;
        let output = child.wait_with_output()?;
        if !output.status.success() {
            return Err("envd file observation failed".into());
        }
        Ok(String::from_utf8(output.stdout)?.trim().to_owned())
    }
}

fn hash_token(token: &str) -> NodeResult<String> {
    let mut child = Process::new("/usr/bin/sha512sum")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or("sha512 stdin unavailable")?
        .write_all(token.as_bytes())?;
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err("sha512 computation failed".into());
    }
    let output = String::from_utf8(output.stdout)?;
    let digest = output
        .split_whitespace()
        .next()
        .ok_or("empty sha512 digest")?;
    if digest.len() != 128 || !digest.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("invalid sha512 digest".into());
    }
    Ok(digest.to_owned())
}

fn write_new_json<T: Serialize>(path: &Path, value: &T) -> NodeResult<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    serde_json::to_writer(&mut file, value)?;
    file.flush()?;
    file.sync_all()?;
    File::open(path.parent().ok_or("missing state parent")?)?.sync_all()?;
    Ok(())
}

fn replace_json<T: Serialize>(path: &Path, value: &T) -> NodeResult<()> {
    let parent = path.parent().ok_or("missing state parent")?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.as_file()
        .set_permissions(fs::Permissions::from_mode(0o600))?;
    serde_json::to_writer(&mut file, value)?;
    file.flush()?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}
