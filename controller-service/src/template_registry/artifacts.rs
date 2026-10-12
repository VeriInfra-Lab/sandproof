//! Original protected host artifact staging and sealing; no registry authority.
use super::{
    CandidateManifest, CandidateVerification, Publication, PublicationState, RegistryConfig,
    valid_candidate_id, valid_digest,
};
use crate::JournalError;
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::Path,
};

/// Artifact trust is rooted at a private directory checked by the deployment.
/// The UID is a trusted constructor parameter, never a request parameter.
pub struct CandidateStore {
    pub(super) config: RegistryConfig,
    pub(super) trusted_uid: u32,
}

impl CandidateStore {
    /// # Errors
    /// Rejects non-private/symlinked roots, or missing private inbox/artifact directories.
    pub fn new(config: RegistryConfig, trusted_uid: u32) -> Result<Self, JournalError> {
        if !config.is_valid() {
            return Err(JournalError::Conflict);
        }
        let store = Self {
            config,
            trusted_uid,
        };
        for path in [
            &store.config.root,
            &store.config.root.join("inbox"),
            &store.config.root.join("artifacts"),
        ] {
            store.private_dir(path)?;
        }
        Ok(store)
    }

    #[must_use]
    pub fn config(&self) -> &RegistryConfig {
        &self.config
    }

    /// Verify an operator-selected immutable local descriptor using the same
    /// artifact identity, digest and ext4 checks as candidate publication.
    /// Paths must first be derived from a protected local cache or catalog;
    /// this method does not authorize an arbitrary remote path. All writers
    /// must be closed and the deployment must preserve artifact immutability.
    /// The returned free space is observation, not a guest success assertion.
    /// # Errors
    /// Rejects invalid descriptors, writable or changed artifacts, digest
    /// mismatches and inconsistent ext4 geometry.
    pub fn verify_binding(
        &self,
        binding: &crate::templates::TemplateBinding,
    ) -> Result<(CandidateVerification, u64), JournalError> {
        if !binding.is_valid() {
            return Err(JournalError::Conflict);
        }
        let evidence = CandidateVerification::check(self, binding)?;
        let mut rootfs = self.readonly_file(&binding.rootfs.path, binding.rootfs.size_bytes)?;
        let free = ext4_free_bytes(&mut rootfs, binding.resources.disk_bytes)?;
        if !evidence.matches(&binding.kernel) || !evidence.matches(&binding.rootfs) {
            return Err(JournalError::Conflict);
        }
        Ok((evidence, free))
    }

    fn private_dir(&self, path: &Path) -> Result<(), JournalError> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_dir() {
            return Err(JournalError::Conflict);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.uid() != self.trusted_uid || metadata.mode() & 0o077 != 0 {
                return Err(JournalError::Conflict);
            }
        }
        Ok(())
    }

    pub(super) fn readonly_file(&self, path: &Path, maximum: u64) -> Result<File, JournalError> {
        let before = fs::symlink_metadata(path)?;
        if !before.is_file() || before.len() > maximum {
            return Err(JournalError::Conflict);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if before.uid() != self.trusted_uid || before.mode() & 0o222 != 0 || before.nlink() != 1
            {
                return Err(JournalError::Conflict);
            }
        }
        let file = File::open(path)?;
        let after = file.metadata()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if (before.dev(), before.ino()) != (after.dev(), after.ino()) {
                return Err(JournalError::Conflict);
            }
        }
        Ok(file)
    }

    /// Read protected staging metadata, not an arbitrary client path or guest assertion.
    /// # Errors
    /// Rejects mismatched ownership, paths, hashes, ext4 accounting, or platform metadata.
    pub fn candidate(&self, owner: u64, id: &str) -> Result<Publication, JournalError> {
        self.verified_candidate(owner, id).map(|(value, _)| value)
    }

    /// Fully check a candidate and retain ephemeral evidence for its sealed paths.
    /// # Errors
    /// Rejects the same invalid candidates as `candidate`, including observable
    /// file identity changes during the digest check. The operator must close
    /// all writers before handoff and keep the artifacts immutable; file
    /// identity is not proof of that precondition. Evidence is not persisted.
    pub fn verified_candidate(
        &self,
        owner: u64,
        id: &str,
    ) -> Result<(Publication, CandidateVerification), JournalError> {
        self.verified_at(owner, id, false)
    }

    fn verified_at(
        &self,
        owner: u64,
        id: &str,
        sealed: bool,
    ) -> Result<(Publication, CandidateVerification), JournalError> {
        if !valid_candidate_id(id) || owner == 0 {
            return Err(JournalError::Conflict);
        }
        let staged = self.config.root.join("inbox").join(id);
        let destination = self.config.root.join("artifacts").join(id);
        let source = if sealed { &destination } else { &staged };
        self.private_dir(source)?;
        let file = self.readonly_file(&source.join("manifest.json"), 64 * 1024)?;
        let manifest: CandidateManifest = serde_json::from_reader(file.take(64 * 1024 + 1))?;
        let mut binding = manifest.binding;
        if binding.owner != Some(owner)
            || binding.template_id != format!("tpl-{id}")
            || binding.revision != "r1"
            || !binding.is_valid()
            || binding.envd_version != "0.9.0"
            || binding.kernel.path != staged.join("kernel")
            || binding.rootfs.path != staged.join("rootfs.ext4")
            || !valid_digest(&manifest.image_digest)
            || manifest.min_free_bytes > binding.resources.disk_bytes
        {
            return Err(JournalError::Conflict);
        }
        if sealed {
            binding.kernel.path = destination.join("kernel");
            binding.rootfs.path = destination.join("rootfs.ext4");
        }
        let (mut verification, free_bytes) = self.verify_binding(&binding)?;
        if free_bytes < manifest.min_free_bytes {
            return Err(JournalError::Conflict);
        }
        if !sealed && destination.try_exists()? {
            return Err(JournalError::Conflict);
        }
        binding.kernel.path = destination.join("kernel");
        binding.rootfs.path = destination.join("rootfs.ext4");
        verification.relocate(&binding);
        Ok((
            Publication {
                candidate_id: id.into(),
                binding,
                image_digest: manifest.image_digest,
                free_bytes,
                min_free_bytes: manifest.min_free_bytes,
                state: PublicationState::Checking,
                checked: false,
                validation_sandbox: None,
                validation_certificate: None,
                failure: None,
            },
            verification,
        ))
    }

    /// Resume only the SAME durably accepted candidate through its original
    /// staging-to-sealed transition. A sealed path is reverified by the original
    /// manifest/hash/ext4 algorithm; existence alone is never adoption evidence.
    /// # Errors
    /// Conflicting dual directories, changed descriptors or partial artifacts
    /// remain unavailable. A rename error is not a no-side-effect assertion.
    pub fn prepare_sealed(
        &self,
        publication: &Publication,
    ) -> Result<CandidateVerification, JournalError> {
        let owner = publication.binding.owner.ok_or(JournalError::Conflict)?;
        let id = &publication.candidate_id;
        if !valid_candidate_id(id) {
            return Err(JournalError::Conflict);
        }
        let staged = self.config.root.join("inbox").join(id);
        let sealed = self.config.root.join("artifacts").join(id);
        if sealed.try_exists()? {
            if staged.try_exists()? {
                return Err(JournalError::Conflict);
            }
        } else {
            let (verified, _) = self.verified_candidate(owner, id)?;
            if verified != *publication {
                return Err(JournalError::Conflict);
            }
            self.seal(publication)?;
        }
        let (verified, evidence) = self.verified_at(owner, id, true)?;
        if verified != *publication {
            return Err(JournalError::Conflict);
        }
        // Also confirms the directory-entry durability on unknown-rename replay.
        File::open(self.config.root.join("inbox"))?.sync_all()?;
        File::open(self.config.root.join("artifacts"))?.sync_all()?;
        Ok(evidence)
    }

    /// Open the exact immutable artifact selected by a protected descriptor.
    /// This is not path authorization: callers must first resolve the descriptor
    /// from their authority and check its private parent. Downloads independently
    /// verify the entire hash before registering any availability fact.
    /// # Errors
    /// Rejects writable/symlinked/replaced/multiple-link or wrong-size files.
    pub fn open_artifact(
        &self,
        artifact: &crate::templates::TemplateArtifact,
    ) -> Result<File, JournalError> {
        if !artifact.is_valid() {
            return Err(JournalError::Conflict);
        }
        let file = self.readonly_file(&artifact.path, artifact.size_bytes)?;
        if file.metadata()?.len() != artifact.size_bytes {
            return Err(JournalError::Conflict);
        }
        Ok(file)
    }

    /// Move the exact staged directory after its durable publication intent exists.
    /// No guest files are mounted, interpreted, or executed on the host.
    /// # Errors
    /// A partial/unknown move stays unavailable; it is never silently repeated.
    pub fn seal(&self, publication: &Publication) -> Result<(), JournalError> {
        let id = &publication.candidate_id;
        if !valid_candidate_id(id) {
            return Err(JournalError::Conflict);
        }
        let source = self.config.root.join("inbox").join(id);
        let destination = self.config.root.join("artifacts").join(id);
        self.private_dir(&source)?;
        if destination.try_exists()? {
            return Err(JournalError::Conflict);
        }
        fs::rename(&source, &destination)?;
        File::open(self.config.root.join("inbox"))?.sync_all()?;
        File::open(self.config.root.join("artifacts"))?.sync_all()?;
        Ok(())
    }
}

fn ext4_free_bytes(file: &mut File, disk_bytes: u64) -> Result<u64, JournalError> {
    let mut block = [0_u8; 1024];
    file.seek(SeekFrom::Start(1024))?;
    file.read_exact(&mut block)?;
    let word = |offset: usize| {
        u64::from(u32::from_le_bytes(
            block[offset..offset + 4].try_into().expect("fixed field"),
        ))
    };
    let shift = word(24);
    if block[56..58] != [0x53, 0xef] || shift > 2 {
        return Err(JournalError::Conflict);
    }
    let size = 1024_u64 << shift;
    let blocks = word(4) | (word(336) << 32);
    let free = word(12) | (word(344) << 32);
    let reserved = word(8) | (word(340) << 32);
    if blocks.checked_mul(size) != Some(disk_bytes) || free > blocks || reserved > blocks {
        return Err(JournalError::Conflict);
    }
    Ok(free.saturating_sub(reserved) * size)
}
