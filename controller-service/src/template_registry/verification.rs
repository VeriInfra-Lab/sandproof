//! Ephemeral evidence from the registry's own reads, never from a guest or journal.

use super::CandidateStore;
use crate::JournalError;
use crate::templates::{TemplateArtifact, TemplateBinding};
use sha2::{Digest, Sha256};
use std::{fs, io::Read};

/// A completed registry hash check. Private fields and no deserializer prevent
/// manifests or persisted publication state from supplying this evidence.
/// All writable handles must be closed before candidate handoff; the trusted
/// operator must keep staged and sealed artifacts immutable thereafter. The
/// observed file identity, including ctime, is not proof of immutability.
pub struct CandidateVerification {
    artifacts: [VerifiedArtifact; 2],
}

struct VerifiedArtifact {
    descriptor: TemplateArtifact,
    identity: ArtifactIdentity,
}

#[derive(PartialEq, Eq)]
struct ArtifactIdentity {
    size: u64,
    modified: Option<std::time::SystemTime>,
    #[cfg(unix)]
    unix: (u64, u64, i64, i64, u32, u32, u32, u64),
}

impl ArtifactIdentity {
    fn read(metadata: &fs::Metadata) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Self {
            size: metadata.len(),
            modified: metadata.modified().ok(),
            #[cfg(unix)]
            unix: (
                metadata.dev(),
                metadata.ino(),
                metadata.ctime(),
                metadata.ctime_nsec(),
                metadata.uid(),
                metadata.gid(),
                metadata.mode(),
                metadata.nlink(),
            ),
        }
    }
}

impl VerifiedArtifact {
    fn check(store: &CandidateStore, artifact: &TemplateArtifact) -> Result<Self, JournalError> {
        Self::check_with(store, artifact, hash_file)
    }

    fn check_with(
        store: &CandidateStore,
        artifact: &TemplateArtifact,
        hash: impl FnOnce(&mut fs::File) -> Result<String, JournalError>,
    ) -> Result<Self, JournalError> {
        let mut file = store.readonly_file(&artifact.path, artifact.size_bytes)?;
        let before = ArtifactIdentity::read(&file.metadata()?);
        if before.size != artifact.size_bytes {
            return Err(JournalError::Conflict);
        }
        // Check both the open file and the path again. Observable identity
        // changes invalidate this check; metadata cannot detect every write or
        // replace the trusted immutable-artifact deployment precondition.
        if hash(&mut file)? != artifact.sha256
            || ArtifactIdentity::read(&file.metadata()?) != before
            || ArtifactIdentity::read(
                &store
                    .readonly_file(&artifact.path, artifact.size_bytes)?
                    .metadata()?,
            ) != before
        {
            return Err(JournalError::Conflict);
        }
        Ok(Self {
            descriptor: artifact.clone(),
            identity: before,
        })
    }

    fn matches(&self, artifact: &TemplateArtifact) -> bool {
        // Non-Unix platforms cannot establish the inode/change-time identity.
        cfg!(unix)
            && self.descriptor == *artifact
            && fs::symlink_metadata(&artifact.path).is_ok_and(|metadata| {
                metadata.is_file() && ArtifactIdentity::read(&metadata) == self.identity
            })
    }
}

fn hash_file(file: &mut fs::File) -> Result<String, JournalError> {
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => digest.update(&buffer[..count]),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(format!("{:x}", digest.finalize()))
}

impl CandidateVerification {
    pub(super) fn check(
        store: &CandidateStore,
        binding: &TemplateBinding,
    ) -> Result<Self, JournalError> {
        Ok(Self {
            artifacts: [
                VerifiedArtifact::check(store, &binding.kernel)?,
                VerifiedArtifact::check(store, &binding.rootfs)?,
            ],
        })
    }

    pub(super) fn relocate(&mut self, binding: &TemplateBinding) {
        self.artifacts[0]
            .descriptor
            .path
            .clone_from(&binding.kernel.path);
        self.artifacts[1]
            .descriptor
            .path
            .clone_from(&binding.rootfs.path);
    }

    /// Match the exact sealed descriptor and current file identity. This only
    /// permits hash reuse; callers still enforce trusted paths and admission.
    #[must_use]
    pub fn matches(&self, artifact: &TemplateArtifact) -> bool {
        self.artifacts.iter().any(|value| value.matches(artifact))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::super::RegistryConfig;
    use super::*;
    use std::{
        io::{Seek, SeekFrom, Write},
        os::unix::fs::{MetadataExt, PermissionsExt},
    };

    #[test]
    fn observable_identity_change_after_hash_cannot_issue_verification() {
        for replace in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("artifact");
            fs::write(&path, b"original").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
            let artifact = TemplateArtifact {
                path: path.clone(),
                size_bytes: 8,
                sha256: format!("{:x}", Sha256::digest(b"original")),
            };
            let store = CandidateStore {
                config: RegistryConfig {
                    root: directory.path().into(),
                    max_templates_global: 1,
                    max_templates_per_tenant: 1,
                    max_artifact_bytes_global: 1024,
                    max_artifact_bytes_per_tenant: 1024,
                    validation_timeout_secs: 60,
                },
                trusted_uid: fs::metadata(&path).unwrap().uid(),
            };
            let result = VerifiedArtifact::check_with(&store, &artifact, |file| {
                let digest = hash_file(file)?;
                if replace {
                    let replacement = directory.path().join("replacement");
                    fs::copy(&path, &replacement).unwrap();
                    fs::rename(replacement, &path).unwrap();
                } else {
                    let before = file.metadata().unwrap();
                    // Unix ctime may tick more coarsely than writes. Explicitly
                    // cross that observable boundary before restoring mtime.
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
                    loop {
                        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
                        let current = fs::metadata(&path).unwrap();
                        if (current.ctime(), current.ctime_nsec())
                            != (before.ctime(), before.ctime_nsec())
                        {
                            break;
                        }
                        assert!(
                            std::time::Instant::now() < deadline,
                            "ctime did not advance"
                        );
                        std::thread::yield_now();
                    }
                    let mut writer = fs::OpenOptions::new().write(true).open(&path).unwrap();
                    writer.seek(SeekFrom::Start(0)).unwrap();
                    writer.write_all(b"modified").unwrap();
                    writer.set_modified(before.modified().unwrap()).unwrap();
                    fs::set_permissions(&path, before.permissions()).unwrap();
                }
                Ok(digest)
            });
            assert!(
                matches!(result, Err(JournalError::Conflict)),
                "replace={replace}, error={:?}",
                result.as_ref().err()
            );
        }
    }
}
