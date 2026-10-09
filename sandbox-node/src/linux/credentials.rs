//! Bound launch credentials and immutable template validation.
use super::{
    ArtifactStamp, BTreeMap, CandidateVerification, CreateOptions, File, GuestSecret,
    LaunchCredentialStatus, LinuxWorker, NodeResult, PathBuf, TemplateArtifact, TemplateBinding,
    artifact_file_digest, check_trusted_path, core, fs, hash_token, launch_observation,
};

impl LinuxWorker {
    /// Create a one-time root-only credential before accepting a sandbox.
    /// The caller must use a CSPRNG and not log or pass the token in argv.
    /// # Errors
    /// Rejects reuse, malformed tokens, or a hash computation failure.
    pub fn put_secret(&self, sandbox: core::SandboxId, token: &str) -> NodeResult<()> {
        self.put_launch_secret(sandbox, token, &BTreeMap::new())
    }

    /// Store the exact creation environment in the same root-only record as
    /// the token. This record exists before the VM can be reserved or booted.
    /// # Errors
    /// Rejects reuse, invalid variables or an unsafe credential location.
    pub fn put_launch_secret(
        &self,
        sandbox: core::SandboxId,
        token: &str,
        env_vars: &BTreeMap<String, String>,
    ) -> NodeResult<()> {
        self.put_bound_launch_secret(
            sandbox,
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
        token: &str,
        options: &CreateOptions,
    ) -> NodeResult<()> {
        self.require_occupied_sandbox(sandbox)?;
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
        let sha512 = hash_token(token)?;
        let value = GuestSecret {
            token: token.to_owned(),
            sha512,
            env_vars: env_vars.clone(),
            template: options.template.clone(),
        };
        let path = self.secret_path(sandbox);
        self.write_node_json(&path, &value)
    }

    /// Read a credential only for the authenticated API response or worker.
    /// # Errors
    /// Rejects a missing or malformed stored secret.
    pub fn token_for(&self, sandbox: core::SandboxId) -> NodeResult<String> {
        Ok(self.secret(sandbox)?.token)
    }

    /// A missing secret is distinguishable from a malformed or untrusted one.
    /// # Errors
    /// Refuses to treat a corrupt credential as safe to replace.
    pub fn has_secret(&self, sandbox: core::SandboxId) -> NodeResult<bool> {
        let path = self.secret_path(sandbox);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
            Ok(_) => {
                self.sync_node_record(&path)?;
                self.secret(sandbox)?;
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
        env_vars: &BTreeMap<String, String>,
    ) -> NodeResult<bool> {
        if !self.has_secret(sandbox)? {
            return Ok(false);
        }
        if self.secret(sandbox)?.env_vars != *env_vars {
            return Err("stored guest initialization differs from admitted create".into());
        }
        Ok(true)
    }

    /// # Errors
    /// A mismatched descriptor is not permission to replace or relaunch a VM.
    pub fn has_bound_launch_secret(
        &self,
        sandbox: core::SandboxId,
        options: &CreateOptions,
    ) -> NodeResult<bool> {
        if !self.has_secret(sandbox)? {
            return Ok(false);
        }
        let stored = self.secret(sandbox)?;
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
        options: &CreateOptions,
        original_digest: &[u8; 32],
    ) -> NodeResult<LaunchCredentialStatus> {
        launch_observation::credential_status(&self.secret_path(sandbox), options, original_digest)
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
        let Ok(mut cache) = self.validated_artifacts.lock() else {
            return;
        };
        for artifact in [&template.kernel, &template.rootfs] {
            let Ok(stamp) = ArtifactStamp::read(artifact) else {
                continue;
            };
            if verification.matches(artifact)
                && ArtifactStamp::read(artifact).is_ok_and(|current| current == stamp)
            {
                cache.insert(artifact.path.clone(), stamp);
            }
        }
    }

    pub(super) fn validate_artifact(&self, artifact: &TemplateArtifact) -> NodeResult<()> {
        let stamp = ArtifactStamp::read(artifact)?;
        let mut cache = self
            .validated_artifacts
            .lock()
            .map_err(|_| "artifact cache lock failed")?;
        if cache.get(&artifact.path) == Some(&stamp) {
            return Ok(());
        }
        if artifact_file_digest(File::open(&artifact.path)?)? != artifact.sha256
            || ArtifactStamp::read(artifact)? != stamp
        {
            return Err("template artifact digest mismatch or changed during verification".into());
        }
        cache.insert(artifact.path.clone(), stamp);
        Ok(())
    }

    pub(super) fn secret_path(&self, sandbox: core::SandboxId) -> PathBuf {
        self.config
            .state_dir
            .join("secrets")
            .join(format!("sb-{}-{}", self.config.namespace, sandbox.0))
    }

    pub(super) fn secret(&self, sandbox: core::SandboxId) -> NodeResult<GuestSecret> {
        let path = self.secret_path(sandbox);
        check_trusted_path(&path)?;
        let value: GuestSecret = serde_json::from_reader(File::open(&path)?)?;
        File::open(path.parent().ok_or("secret parent")?)?.sync_all()?;
        if value.token.len() < 32
            || value.sha512.len() != 128
            || hash_token(&value.token)? != value.sha512
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
