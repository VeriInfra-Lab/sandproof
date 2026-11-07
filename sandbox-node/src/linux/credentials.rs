//! Bound launch credentials and immutable template validation.
use super::{
    ArtifactStamp, BTreeMap, CandidateVerification, CreateOptions, File, GuestSecret,
    LaunchCredentialStatus, LinuxWorker, NodeResult, PathBuf, TemplateArtifact, TemplateBinding,
    artifact_file_digest, check_trusted_path, core, fs, hash_token, launch_observation,
};

pub(super) fn secret_file_name(
    namespace: &str,
    sandbox: core::SandboxId,
    generation: core::Generation,
) -> String {
    format!("sb-{namespace}-{}-{generation}", sandbox.0)
}

impl LinuxWorker {
    /// Create the root-only credential for one already bound native generation.
    /// The caller must use a CSPRNG and not log or pass the token in argv.
    /// # Errors
    /// Rejects reuse, malformed tokens, or a hash computation failure.
    pub fn put_secret(
        &self,
        sandbox: core::SandboxId,
        generation: core::Generation,
        token: &str,
    ) -> NodeResult<()> {
        self.put_launch_secret(sandbox, generation, token, &BTreeMap::new())
    }

    /// Store the exact creation environment in the same root-only record as
    /// the token. The original position is bound before this record; the VM
    /// cannot be booted until the same generation's credential is registered.
    /// # Errors
    /// Rejects reuse, invalid variables or an unsafe credential location.
    pub fn put_launch_secret(
        &self,
        sandbox: core::SandboxId,
        generation: core::Generation,
        token: &str,
        env_vars: &BTreeMap<String, String>,
    ) -> NodeResult<()> {
        self.put_bound_launch_secret(
            sandbox,
            generation,
            token,
            &CreateOptions {
                env_vars: env_vars.clone(),
                ..CreateOptions::default()
            },
        )
    }

    /// Persist the exact admitted prebuilt template with the launch credential.
    /// # Errors
    /// Rejects invalid artifacts, credentials, configuration or record reuse.
    pub fn put_bound_launch_secret(
        &self,
        sandbox: core::SandboxId,
        generation: core::Generation,
        token: &str,
        options: &CreateOptions,
    ) -> NodeResult<()> {
        self.credential_context(sandbox, generation)?;
        options.validate()?;
        if let Some(template) = &options.template {
            self.validate_template(template)?;
        }
        let env_vars = &options.env_vars;
        if token.len() < 32
            || token.len() > 128
            || !token
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        {
            return Err("invalid guest access token shape".into());
        }
        if env_vars.iter().any(|(key, value)| {
            key.is_empty() || key.contains('=') || key.contains('\0') || value.contains('\0')
        }) {
            return Err("invalid guest initialization variable".into());
        }
        let sha512 = hash_token(token);
        let value = GuestSecret {
            generation,
            token: token.to_owned(),
            sha512,
            env_vars: env_vars.clone(),
            template: options.template.clone(),
        };
        let path = self.secret_path(sandbox, generation);
        self.write_node_json(&path, &value)
    }

    /// Read a credential only for the authenticated API response or worker.
    /// # Errors
    /// Rejects a missing or malformed stored secret.
    pub fn token_for(
        &self,
        sandbox: core::SandboxId,
        generation: core::Generation,
    ) -> NodeResult<String> {
        self.credential_context(sandbox, generation)?;
        Ok(self.secret(sandbox, generation)?.token)
    }

    /// A missing secret is distinguishable from a malformed or untrusted one.
    /// # Errors
    /// Refuses to treat a corrupt credential as safe to replace.
    pub fn has_secret(
        &self,
        sandbox: core::SandboxId,
        generation: core::Generation,
    ) -> NodeResult<bool> {
        self.credential_context(sandbox, generation)?;
        self.has_secret_record(sandbox, generation)
    }

    pub(super) fn has_secret_record(
        &self,
        sandbox: core::SandboxId,
        generation: core::Generation,
    ) -> NodeResult<bool> {
        let path = self.secret_path(sandbox, generation);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
            Ok(_) => {
                self.sync_node_record(&path)?;
                self.secret(sandbox, generation)?;
                Ok(true)
            }
        }
    }

    /// A present but mismatched initialization record cannot be reused after
    /// a controller restart; that would start the VM with the wrong inputs.
    /// # Errors
    /// Rejects unreadable, malformed or mismatched stored data.
    pub fn has_launch_secret(
        &self,
        sandbox: core::SandboxId,
        generation: core::Generation,
        env_vars: &BTreeMap<String, String>,
    ) -> NodeResult<bool> {
        if !self.has_secret(sandbox, generation)? {
            return Ok(false);
        }
        if self.secret(sandbox, generation)?.env_vars != *env_vars {
            return Err("stored guest initialization differs from admitted create".into());
        }
        Ok(true)
    }

    /// # Errors
    /// A mismatched descriptor is not permission to replace or relaunch a VM.
    pub fn has_bound_launch_secret(
        &self,
        sandbox: core::SandboxId,
        generation: core::Generation,
        options: &CreateOptions,
    ) -> NodeResult<bool> {
        if !self.has_secret(sandbox, generation)? {
            return Ok(false);
        }
        let stored = self.secret(sandbox, generation)?;
        if stored.env_vars != options.env_vars || stored.template != options.template {
            return Err("stored template or initialization differs from admitted create".into());
        }
        Ok(true)
    }

    /// Classify only a target credential fault after its original index binding
    /// and physical resources have been checked by the caller. This is not
    /// authentication, permission to replace a secret, or cleanup evidence.
    /// # Errors
    /// Untrusted paths and changed launch configuration remain global errors.
    pub fn audit_launch_credential(
        &self,
        sandbox: core::SandboxId,
        generation: core::Generation,
        options: &CreateOptions,
        original_digest: &[u8; 32],
    ) -> NodeResult<LaunchCredentialStatus> {
        launch_observation::credential_status(
            &self.secret_path(sandbox, generation),
            generation,
            options,
            original_digest,
        )
    }

    /// Check protected inputs and cgroup limits before controller admission.
    /// # Errors
    /// Rejects missing, modified or untrusted artifacts and unavailable host limits.
    pub fn validate_template(&self, template: &TemplateBinding) -> NodeResult<()> {
        if !template.is_valid() {
            return Err("invalid template descriptor".into());
        }
        self.validate_artifact(&template.kernel)?;
        self.validate_artifact(&template.rootfs)?;
        let mut resources = self
            .config
            .host_resources
            .clone()
            .ok_or("templates require explicit host resources")?;
        resources.memory_max_bytes = template.resources.host_memory_bytes;
        resources.validate(template.resources.memory_mib)?;
        Ok(())
    }

    /// Reuse completed registry hashes only for unchanged, trusted sealed files.
    /// Missing/stale evidence leaves the normal node cache/hash checks in place.
    pub fn cache_candidate_verification(
        &self,
        template: &TemplateBinding,
        verification: &CandidateVerification,
    ) {
        for artifact in [&template.kernel, &template.rootfs] {
            let Ok(stamp) = ArtifactStamp::read(artifact) else {
                continue;
            };
            if verification.matches(artifact)
                && ArtifactStamp::read(artifact).is_ok_and(|current| current == stamp)
            {
                let Ok(mut cache) = self.validated_artifacts.lock() else {
                    return;
                };
                cache.insert(artifact.path.clone(), stamp);
            }
        }
    }

    pub(super) fn validate_artifact(&self, artifact: &TemplateArtifact) -> NodeResult<()> {
        self.validate_artifact_with(artifact, artifact_file_digest)
    }

    pub(super) fn validate_artifact_with(
        &self,
        artifact: &TemplateArtifact,
        hash: impl FnOnce(File) -> std::io::Result<String>,
    ) -> NodeResult<()> {
        let stamp = ArtifactStamp::read(artifact)?;
        let cached = self
            .validated_artifacts
            .lock()
            .map_err(|_| "artifact cache lock failed")?
            .get(&artifact.path)
            == Some(&stamp);
        // Neither a full hash nor filesystem I/O runs under the shared cache
        // lock. A concurrent miss may hash twice, within the caller's bounded
        // work allowance; it cannot make another sandbox wait for this hash.
        if !cached && hash(File::open(&artifact.path)?)? != artifact.sha256 {
            return Err("template artifact digest mismatch or changed during verification".into());
        }
        if ArtifactStamp::read(artifact)? != stamp {
            return Err("template artifact digest mismatch or changed during verification".into());
        }
        if !cached {
            self.validated_artifacts
                .lock()
                .map_err(|_| "artifact cache lock failed")?
                .insert(artifact.path.clone(), stamp);
        }
        Ok(())
    }

    pub(super) fn secret_for_vm(
        &self,
        sandbox: core::SandboxId,
        vm: core::VmId,
    ) -> NodeResult<GuestSecret> {
        let context = self.original_source_context(&sandproof_protocol::ResourceRef {
            kind: sandproof_protocol::ResourceKind::Vm,
            id: sandproof_protocol::ResourceId::try_from(self.vm_name(vm))?,
        })?;
        if context.binding.sandbox != sandbox || context.binding.vm != vm {
            return Err("credential VM differs from original sandbox binding".into());
        }
        self.secret(sandbox, context.binding.generation)
    }

    pub(super) fn secret_path(
        &self,
        sandbox: core::SandboxId,
        generation: core::Generation,
    ) -> PathBuf {
        self.config.state_dir.join("secrets").join(secret_file_name(
            &self.config.namespace,
            sandbox,
            generation,
        ))
    }

    pub(super) fn secret(
        &self,
        sandbox: core::SandboxId,
        generation: core::Generation,
    ) -> NodeResult<GuestSecret> {
        let path = self.secret_path(sandbox, generation);
        check_trusted_path(&path)?;
        let value: GuestSecret = serde_json::from_reader(File::open(&path)?)?;
        File::open(path.parent().ok_or("secret parent")?)?.sync_all()?;
        if generation == 0
            || value.generation != generation
            || value.token.len() < 32
            || value.sha512.len() != 128
            || hash_token(&value.token) != value.sha512
            || value
                .template
                .as_ref()
                .is_some_and(|binding| !binding.is_valid())
        {
            return Err("stored guest secret hash mismatch".into());
        }
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_generation_names_are_distinct_and_fit_the_original_entry_bound() {
        assert_eq!(secret_file_name("ns", core::SandboxId(1), 1), "sb-ns-1-1");
        assert_ne!(
            secret_file_name("ns", core::SandboxId(1), 1),
            secret_file_name("ns", core::SandboxId(1), 2)
        );
        let maximum = secret_file_name("0123456789abcdef", core::SandboxId(u64::MAX), u64::MAX);
        assert!(maximum.len() <= 64);
        assert!(sandproof_protocol::ResourceId::try_from(maximum).is_ok());
    }
}
