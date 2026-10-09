//! Synchronous guest transports with exact-child ownership.
use super::*;

impl LinuxWorker {
    pub(super) fn namespace_command(
        &self,
        name: &str,
    ) -> NodeResult<(sandbox_runtime::RuntimeHelperGuard, Process)> {
        let number = name
            .strip_prefix(&format!("vm-{}-", self.config.namespace))
            .ok_or("namespace helper identity")?
            .parse::<u64>()?;
        let vm = core::VmId(number);
        if self.vm_name(vm) != name {
            return Err("namespace helper identity not canonical".into());
        }
        let context = self.runtime_host_slot(self.physical_slot_for_vm(vm)?, vm)?;
        let guard = context.helper_gate.try_acquire_shared(Some(&context))?;
        let mut command = Process::new("/usr/bin/ip");
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        Ok((guard, command))
    }

    pub(super) fn spawn_namespace_command(
        &self,
        name: &str,
        guard: &sandbox_runtime::RuntimeHelperGuard,
        command: &mut Process,
    ) -> NodeResult<sandbox_runtime::OwnedChild> {
        let vm = core::VmId(
            name.strip_prefix(&format!("vm-{}-", self.config.namespace))
                .ok_or("namespace helper identity")?
                .parse::<u64>()?,
        );
        let context = self.runtime_host_slot(self.physical_slot_for_vm(vm)?, vm)?;
        let args = command
            .get_args()
            .map(|arg| arg.to_str().ok_or("non-UTF8 namespace helper argument"))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(sandbox_runtime::OwnedChild::new(guard.spawn_tracked(
            &context,
            "/usr/bin/ip",
            &args,
        )?))
    }

    pub(super) fn guest_status(
        &self,
        name: &str,
        method: &str,
        path: &str,
        input: Option<&[u8]>,
    ) -> NodeResult<String> {
        let (gate, mut command) = self.namespace_command(name)?;
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
        command.stdin(Stdio::piped()).stdout(Stdio::piped());
        let mut child = self.spawn_namespace_command(name, &gate, &mut command)?;
        if let Some(input) = input {
            child
                .stdin
                .take()
                .ok_or("curl stdin unavailable")?
                .write_all(input)?;
        }
        let output = child.wait_output()?;
        gate.finish_tracked()?;
        if !output.status.success() {
            return Ok(String::new());
        }
        Ok(String::from_utf8(output.stdout)?.trim().to_owned())
    }

    pub(super) fn guest_file_status(&self, name: &str, token: &str) -> NodeResult<String> {
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
            .stdout(Stdio::piped());
        let mut child = self.spawn_namespace_command(name, &gate, &mut command)?;
        let config = format!(
            "url = \"{ENVD_URL}/files?path=%2Fetc%2Fpasswd\"\nheader = \"X-Access-Token: {token}\"\n"
        );
        child
            .stdin
            .take()
            .ok_or("curl stdin unavailable")?
            .write_all(config.as_bytes())?;
        let output = child.wait_output()?;
        gate.finish_tracked()?;
        if !output.status.success() {
            return Err("envd file observation failed".into());
        }
        Ok(String::from_utf8(output.stdout)?.trim().to_owned())
    }
}
