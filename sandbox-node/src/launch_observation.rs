//! Credential failures are separate from protected physical-resource authority.
use super::{
    CreateOptions, File, GuestSecret, MetadataExt, NodeResult, Path, RuntimeObservationConfig,
    Sha256, TemplateBinding, check_trusted_path, fs, hash_token, node_staging,
};
use sha2::Digest;
use std::io::Read;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialFault {
    Missing,
    Unreadable,
    Malformed,
    TokenHashMismatch,
    OriginalDigestMismatch,
}

/// This status never grants authority, changes lifecycle state or proves release.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchCredentialStatus {
    Intact,
    Unavailable(CredentialFault),
}

pub(super) fn bind_template(
    mut config: RuntimeObservationConfig,
    template: Option<&TemplateBinding>,
) -> NodeResult<RuntimeObservationConfig> {
    if let Some(template) = template {
        if !template.is_valid() {
            return Err("invalid original observation template".into());
        }
        config.kernel.clone_from(&template.kernel.path);
        config.rootfs.clone_from(&template.rootfs.path);
        config.vcpu_count = template.resources.cpu_count;
        config.memory_mib = template.resources.memory_mib;
        config
            .host_resources
            .as_mut()
            .ok_or("templates require host limits")?
            .memory_max_bytes = template.resources.host_memory_bytes;
    }
    Ok(config)
}

pub(super) fn credential_status(
    path: &Path,
    options: &CreateOptions,
    original_digest: &[u8; 32],
) -> NodeResult<LaunchCredentialStatus> {
    options.validate()?;
    let parent = path.parent().ok_or("credential has no parent")?;
    // Missing/untrusted parents are not a target-leaf fault. Do not turn loss
    // of shared path authority into a local credential exception.
    check_trusted_path(parent)?;
    let parent_metadata = fs::symlink_metadata(parent)?;
    if !parent_metadata.is_dir()
        || parent_metadata.uid() != 0
        || parent_metadata.mode() & 0o077 != 0
    {
        return Err("untrusted credential parent".into());
    }
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LaunchCredentialStatus::Unavailable(
                CredentialFault::Missing,
            ));
        }
        Err(error) => return Err(error.into()),
    };
    check_trusted_path(path)?;
    if !metadata.is_file() || metadata.uid() != 0 || metadata.mode() & 0o077 != 0 {
        return Err("untrusted credential leaf".into());
    }
    let input = match rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::CLOEXEC | rustix::fs::OFlags::NOFOLLOW,
        rustix::fs::Mode::empty(),
    ) {
        Ok(input) => File::from(input),
        Err(_) => {
            return Ok(LaunchCredentialStatus::Unavailable(
                CredentialFault::Unreadable,
            ));
        }
    };
    let opened = input.metadata()?;
    if (metadata.dev(), metadata.ino()) != (opened.dev(), opened.ino())
        || !opened.is_file()
        || opened.uid() != 0
        || opened.mode() & 0o077 != 0
    {
        return Err("credential identity changed during observation".into());
    }
    let maximum = node_staging::MAX_PAYLOAD as u64;
    if metadata.len() > maximum || opened.len() > maximum {
        return Ok(LaunchCredentialStatus::Unavailable(
            CredentialFault::Malformed,
        ));
    }
    // The extra byte detects growth after the metadata check without parsing
    // an unbounded leaf. This reuses the existing node-record payload budget.
    let mut payload = Vec::new();
    if input.take(maximum + 1).read_to_end(&mut payload).is_err() {
        return Ok(LaunchCredentialStatus::Unavailable(
            CredentialFault::Unreadable,
        ));
    }
    if payload.len() > node_staging::MAX_PAYLOAD {
        return Ok(LaunchCredentialStatus::Unavailable(
            CredentialFault::Malformed,
        ));
    }
    let stored = match serde_json::from_slice(&payload) {
        Ok(stored) => stored,
        Err(error) => {
            return Ok(LaunchCredentialStatus::Unavailable(if error.is_io() {
                CredentialFault::Unreadable
            } else {
                CredentialFault::Malformed
            }));
        }
    };
    File::open(parent)?.sync_all()?;
    classify(&stored, options, original_digest)
}

fn classify(
    stored: &GuestSecret,
    options: &CreateOptions,
    original_digest: &[u8; 32],
) -> NodeResult<LaunchCredentialStatus> {
    // A readable, changed launch descriptor is not merely a token fault.
    if stored.env_vars != options.env_vars || stored.template != options.template {
        return Err("stored template or initialization differs from admitted create".into());
    }
    if stored.token.len() < 32
        || stored.sha512.len() != 128
        || hash_token(&stored.token) != stored.sha512
    {
        return Ok(LaunchCredentialStatus::Unavailable(
            CredentialFault::TokenHashMismatch,
        ));
    }
    let digest: [u8; 32] = Sha256::digest(stored.token.as_bytes()).into();
    let mismatch = digest
        .iter()
        .zip(original_digest)
        .fold(0_u8, |difference, (actual, original)| {
            difference | (actual ^ original)
        });
    Ok(if mismatch == 0 {
        LaunchCredentialStatus::Intact
    } else {
        LaunchCredentialStatus::Unavailable(CredentialFault::OriginalDigestMismatch)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandbox_runtime::{HostResources, RuntimeHelperGate, RuntimeHostSlot};
    use sandproof_controller_service::templates::{ResourceShape, TemplateArtifact};

    fn secret(token: &str) -> GuestSecret {
        GuestSecret {
            token: token.into(),
            sha512: hash_token(token),
            env_vars: std::collections::BTreeMap::default(),
            template: None,
        }
    }

    #[test]
    fn only_credential_content_faults_can_be_local() {
        let token = "a".repeat(64);
        let digest = Sha256::digest(token.as_bytes()).into();
        let options = CreateOptions::default();
        assert_eq!(
            classify(&secret(&token), &options, &digest).unwrap(),
            LaunchCredentialStatus::Intact
        );
        let mut bad = secret(&token);
        bad.sha512 = "0".repeat(128);
        assert_eq!(
            classify(&bad, &options, &digest).unwrap(),
            LaunchCredentialStatus::Unavailable(CredentialFault::TokenHashMismatch)
        );
        assert_eq!(
            classify(&secret(&"b".repeat(64)), &options, &digest).unwrap(),
            LaunchCredentialStatus::Unavailable(CredentialFault::OriginalDigestMismatch)
        );
        let mut changed = secret(&token);
        changed.env_vars.insert("CHANGED".into(), "1".into());
        assert!(classify(&changed, &options, &digest).is_err());
    }

    #[test]
    fn physical_observation_uses_original_template_shape_without_credentials() {
        let base = RuntimeObservationConfig {
            guest_instance_id: "guest-instance".into(),
            console_policy: Some(sandbox_runtime::ConsolePolicy::Discard),
            host_slot: Some(RuntimeHostSlot {
                position: 1,
                uplink_index: 21,
                vm: 8,
                helper_gate: RuntimeHelperGate {
                    path: "/node/helper.lock".into(),
                    device: 2,
                    inode: 3,
                    max_data_routes_per_slot: 4,
                },
            }),
            jailer: "/bin/jailer".into(),
            firecracker: "/bin/firecracker".into(),
            chroot_base: "/jails".into(),
            kernel: "/default/kernel".into(),
            rootfs: "/default/rootfs".into(),
            vm_uid: 10_001,
            vm_gid: 10_001,
            vcpu_count: 1,
            memory_mib: 128,
            host_resources: Some(HostResources {
                cgroup_parent: "/sys/fs/cgroup/delegated".into(),
                memory_max_bytes: 192 * 1024 * 1024,
            }),
            private_netns: Some("/run/netns/vm-original-8".into()),
        };
        let template = TemplateBinding {
            guest: None,
            template_id: "original".into(),
            revision: "original-revision".into(),
            owner: Some(1),
            kernel: TemplateArtifact {
                path: "/original/kernel".into(),
                sha256: "a".repeat(64),
                size_bytes: 4096,
            },
            rootfs: TemplateArtifact {
                path: "/original/rootfs".into(),
                sha256: "b".repeat(64),
                size_bytes: 4096,
            },
            resources: ResourceShape {
                cpu_count: 2,
                memory_mib: 256,
                disk_bytes: 4096,
                host_memory_bytes: 300 * 1024 * 1024,
            },
            envd_version: "fixture".into(),
            startup: None,
        };
        assert_eq!(bind_template(base.clone(), None).unwrap(), base);
        let original = bind_template(base.clone(), Some(&template)).unwrap();
        assert_eq!(original.kernel, template.kernel.path);
        assert_eq!(original.rootfs, template.rootfs.path);
        assert_eq!(original.vcpu_count, 2);
        assert_eq!(original.memory_mib, 256);
        assert_eq!(
            original.host_resources.as_ref().unwrap().memory_max_bytes,
            300 * 1024 * 1024
        );
        assert_eq!(original.vm_uid, base.vm_uid);
        assert_eq!(original.vm_gid, base.vm_gid);
        assert_eq!(original.host_slot, base.host_slot);
        assert_eq!(original.private_netns, base.private_netns);
        assert_eq!(
            original.host_resources.unwrap().cgroup_parent,
            base.host_resources.as_ref().unwrap().cgroup_parent
        );
        let mut invalid = base.clone();
        invalid.host_resources = None;
        assert!(bind_template(invalid, Some(&template)).is_err());
        let mut invalid = template;
        invalid.resources.cpu_count = 0;
        assert!(bind_template(base, Some(&invalid)).is_err());
    }

    #[test]
    #[ignore = "requires root and SANDPROOF_ARTIFACT_TEST_PARENT in a trusted private directory; no KVM or VM"]
    fn protected_leaf_faults_are_local_but_parent_and_launch_authority_are_not() {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(rustix::process::geteuid().as_raw(), 0);
        let parent = std::env::var_os("SANDPROOF_ARTIFACT_TEST_PARENT").unwrap();
        check_trusted_path(Path::new(&parent)).unwrap();
        let directory = tempfile::tempdir_in(parent).unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let path = directory.path().join("credential");
        let options = CreateOptions::default();
        let original = secret(&"a".repeat(64));
        let digest = Sha256::digest(original.token.as_bytes()).into();
        assert_eq!(
            credential_status(&path, &options, &digest).unwrap(),
            LaunchCredentialStatus::Unavailable(CredentialFault::Missing)
        );
        fs::write(&path, b"{").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            credential_status(&path, &options, &digest).unwrap(),
            LaunchCredentialStatus::Unavailable(CredentialFault::Malformed)
        );
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(node_staging::MAX_PAYLOAD as u64 + 1)
            .unwrap();
        assert_eq!(
            credential_status(&path, &options, &digest).unwrap(),
            LaunchCredentialStatus::Unavailable(CredentialFault::Malformed)
        );
        fs::write(&path, serde_json::to_vec(&original).unwrap()).unwrap();
        assert_eq!(
            credential_status(&path, &options, &digest).unwrap(),
            LaunchCredentialStatus::Intact
        );
        let mut changed = original;
        changed.sha512 = "0".repeat(128);
        fs::write(&path, serde_json::to_vec(&changed).unwrap()).unwrap();
        assert_eq!(
            credential_status(&path, &options, &digest).unwrap(),
            LaunchCredentialStatus::Unavailable(CredentialFault::TokenHashMismatch)
        );
        changed.env_vars.insert("CHANGED".into(), "1".into());
        fs::write(&path, serde_json::to_vec(&changed).unwrap()).unwrap();
        assert!(credential_status(&path, &options, &digest).is_err());
        let link = directory.path().join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(credential_status(&link, &options, &digest).is_err());
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o770)).unwrap();
        assert!(credential_status(&directory.path().join("missing"), &options, &digest).is_err());
    }
}
