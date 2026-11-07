//! Finite original-key cleanup, including stages that never became bundles.
//! The durable service owns writer/reader drain and eligibility, not this plan.
use super::retirement::DirectoryIdentity;
use super::{FileStamp, SnapshotStore, invalid, key_name, matches_entry, os_error};
use crate::RuntimeError;
use rustix::fs::{self, AtFlags, Mode, OFlags};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsStr,
    fs::{File, Permissions},
    os::unix::fs::{MetadataExt, PermissionsExt},
};

const FILES: [&str; 4] = ["manifest.json", "state.bin", "memory.bin", "disk-0.img"];
const MARKERS: [&str; 4] = [
    "completed-",
    "completion-stage-",
    "api-returned-",
    "api-return-stage-",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RetiredDirectory {
    identity: DirectoryIdentity,
    files: Vec<Option<FileStamp>>,
}

/// Read-only preparation creates no cleanup authority. Persist the exact bytes
/// before calling retirement; a missing file never authorizes deleting a new one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureStoreRetirement {
    version: u32,
    key: [u8; 32],
    parent: DirectoryIdentity,
    stage: Option<RetiredDirectory>,
    capture: Option<RetiredDirectory>,
    markers: Vec<Option<FileStamp>>,
}

pub(crate) fn directory_at(parent: &File, name: &str) -> Result<Option<File>, RuntimeError> {
    match fs::openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => {
            let file = File::from(fd);
            matches_entry(parent, OsStr::new(name), &file)?;
            Ok(Some(file))
        }
        Err(rustix::io::Errno::NOENT) => Ok(None),
        Err(error) => Err(os_error(error)),
    }
}
fn leaf(parent: &File, name: &str, owner: u32) -> Result<Option<FileStamp>, RuntimeError> {
    let file = match fs::openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => File::from(fd),
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(error) => return Err(os_error(error)),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || metadata.uid() != owner
        || metadata.nlink() != 1
        || metadata.mode() & 0o7022 != 0
    {
        return Err(invalid("retiring material file type, owner, links or mode"));
    }
    matches_entry(parent, OsStr::new(name), &file)?;
    Ok(Some(FileStamp::from_metadata(&metadata)))
}
fn known(directory: &File, names: &[&str]) -> Result<(), RuntimeError> {
    for entry in fs::Dir::read_from(directory).map_err(os_error)? {
        let entry = entry.map_err(os_error)?;
        let name = entry.file_name().to_bytes();
        if name != b"." && name != b".." && !names.iter().any(|known| known.as_bytes() == name) {
            return Err(invalid("unplanned entry in retiring material directory"));
        }
    }
    Ok(())
}
impl RetiredDirectory {
    pub(crate) fn capture(
        parent: &File,
        name: &str,
        owners: &[u32],
        names: &[&str],
    ) -> Result<Option<Self>, RuntimeError> {
        let Some(directory) = directory_at(parent, name)? else {
            return Ok(None);
        };
        let metadata = directory.metadata()?;
        if !owners.contains(&metadata.uid())
            || ![0o500, 0o700].contains(&(metadata.mode() & 0o7777))
        {
            return Err(invalid("retiring material directory owner or mode"));
        }
        known(&directory, names)?;
        let files = names
            .iter()
            .map(|name| leaf(&directory, name, metadata.uid()))
            .collect::<Result<_, _>>()?;
        Ok(Some(Self {
            identity: DirectoryIdentity::capture(&directory)?,
            files,
        }))
    }
    pub(crate) fn verify(
        parent: &File,
        name: &str,
        plan: Option<&Self>,
        names: &[&str],
    ) -> Result<Option<File>, RuntimeError> {
        if plan.is_some_and(|plan| plan.files.len() != names.len()) {
            return Err(invalid("material retirement file set shape differs"));
        }
        let Some(directory) = directory_at(parent, name)? else {
            return Ok(None);
        };
        let plan = plan.ok_or_else(|| invalid("unplanned material directory appeared"))?;
        if DirectoryIdentity::capture(&directory)? != plan.identity
            || ![0o500, 0o700].contains(&(directory.metadata()?.mode() & 0o7777))
        {
            return Err(invalid("retiring material original directory changed"));
        }
        known(&directory, names)?;
        let owner = directory.metadata()?.uid();
        for (name, expected) in names.iter().zip(&plan.files) {
            verify_leaf(&directory, name, owner, expected.as_ref())?;
        }
        Ok(Some(directory))
    }
    pub(crate) fn retire(
        parent: &File,
        name: &str,
        plan: Option<&Self>,
        names: &[&str],
    ) -> Result<(), RuntimeError> {
        let Some(directory) = Self::verify(parent, name, plan, names)? else {
            parent.sync_all()?;
            return Ok(());
        };
        let plan = plan.ok_or_else(|| invalid("missing material retirement plan"))?;
        directory.set_permissions(Permissions::from_mode(0o700))?;
        directory.sync_all()?;
        let owner = directory.metadata()?.uid();
        for (name, expected) in names.iter().zip(&plan.files) {
            if verify_leaf(&directory, name, owner, expected.as_ref())? {
                fs::unlinkat(&directory, *name, AtFlags::empty()).map_err(os_error)?;
                directory.sync_all()?;
            }
        }
        matches_entry(parent, OsStr::new(name), &directory)?;
        fs::unlinkat(parent, name, AtFlags::REMOVEDIR).map_err(os_error)?;
        parent.sync_all()?;
        Ok(())
    }
}
fn verify_leaf(
    parent: &File,
    name: &str,
    owner: u32,
    expected: Option<&FileStamp>,
) -> Result<bool, RuntimeError> {
    match leaf(parent, name, owner)? {
        None => Ok(false),
        Some(actual) if Some(&actual) == expected => Ok(true),
        Some(_) => Err(invalid(
            "retiring material original file changed or appeared",
        )),
    }
}
impl CaptureStoreRetirement {
    #[must_use]
    pub fn directory_key(&self) -> &[u8; 32] {
        &self.key
    }
}
impl SnapshotStore {
    pub(crate) fn retirement_manifest_sha256(
        &self,
        key: [u8; 32],
    ) -> Result<[u8; 32], RuntimeError> {
        use sha2::{Digest, Sha256};
        use std::io::Read;
        let name = key_name("capture-", &key);
        let directory = directory_at(&self.directory, &name)?
            .ok_or_else(|| invalid("retiring committed bundle is absent"))?;
        let metadata = directory.metadata()?;
        if metadata.uid() != self.owner || metadata.mode() & 0o7777 != 0o500 {
            return Err(invalid("retiring committed bundle owner or mode"));
        }
        let file = super::bundle::original_file(
            &directory,
            "manifest.json",
            self.owner,
            super::MAX_MANIFEST_BYTES as u64,
        )?;
        let before = FileStamp::from_metadata(&file.metadata()?);
        let mut bytes = Vec::new();
        (&file)
            .take(super::MAX_MANIFEST_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 != before.length
            || before != FileStamp::from_metadata(&file.metadata()?)
        {
            return Err(invalid("retiring manifest changed while reading"));
        }
        matches_entry(&directory, OsStr::new("manifest.json"), &file)?;
        Ok(Sha256::digest(&bytes).into())
    }

    /// Capture fixed stage/bundle and four marker identities without changing
    /// files or depending on a valid manifest. Partial output remains removable.
    /// # Errors
    /// Foreign entries, nonregular files or untrusted parents refuse preparation.
    pub fn capture_retirement_plan(
        &self,
        key: [u8; 32],
    ) -> Result<CaptureStoreRetirement, RuntimeError> {
        let stage = RetiredDirectory::capture(
            &self.directory,
            &key_name("staging-", &key),
            &[self.owner],
            &FILES,
        )?;
        let capture = RetiredDirectory::capture(
            &self.directory,
            &key_name("capture-", &key),
            &[self.owner],
            &FILES,
        )?;
        if stage.is_some() && capture.is_some() {
            return Err(invalid("capture and stage cannot both be original"));
        }
        let markers = MARKERS
            .iter()
            .map(|prefix| leaf(&self.directory, &key_name(prefix, &key), self.owner))
            .collect::<Result<_, _>>()?;
        Ok(CaptureStoreRetirement {
            version: 1,
            key,
            parent: DirectoryIdentity::capture(&self.directory)?,
            stage,
            capture,
            markers,
        })
    }
    pub(crate) fn verify_capture_retirement(
        &self,
        plan: &CaptureStoreRetirement,
    ) -> Result<(), RuntimeError> {
        if plan.version != 1
            || plan.markers.len() != MARKERS.len()
            || (plan.stage.is_some() && plan.capture.is_some())
            || DirectoryIdentity::capture(&self.directory)? != plan.parent
        {
            return Err(invalid("material retirement store or shape differs"));
        }
        RetiredDirectory::verify(
            &self.directory,
            &key_name("staging-", &plan.key),
            plan.stage.as_ref(),
            &FILES,
        )?;
        RetiredDirectory::verify(
            &self.directory,
            &key_name("capture-", &plan.key),
            plan.capture.as_ref(),
            &FILES,
        )?;
        for (prefix, expected) in MARKERS.iter().zip(&plan.markers) {
            verify_leaf(
                &self.directory,
                &key_name(prefix, &plan.key),
                self.owner,
                expected.as_ref(),
            )?;
        }
        Ok(())
    }
    /// Remove only this saved finite set; missing entries permit replay, while
    /// replacements and late output refuse. Errors retain the original charge.
    /// # Errors
    /// Changed entries or failed unlink/fsync do not confirm reclamation.
    pub fn retire_capture_material(
        &self,
        plan: &CaptureStoreRetirement,
    ) -> Result<(), RuntimeError> {
        self.verify_capture_retirement(plan)?;
        RetiredDirectory::retire(
            &self.directory,
            &key_name("staging-", &plan.key),
            plan.stage.as_ref(),
            &FILES,
        )?;
        RetiredDirectory::retire(
            &self.directory,
            &key_name("capture-", &plan.key),
            plan.capture.as_ref(),
            &FILES,
        )?;
        for (prefix, expected) in MARKERS.iter().zip(&plan.markers) {
            let name = key_name(prefix, &plan.key);
            if verify_leaf(&self.directory, &name, self.owner, expected.as_ref())? {
                fs::unlinkat(&self.directory, name.as_str(), AtFlags::empty()).map_err(os_error)?;
                self.directory.sync_all()?;
            }
        }
        self.directory.sync_all()?;
        self.verify_capture_absence(plan)
    }
    fn verify_capture_absence(&self, plan: &CaptureStoreRetirement) -> Result<(), RuntimeError> {
        for prefix in [
            "staging-",
            "capture-",
            "completed-",
            "completion-stage-",
            "api-returned-",
            "api-return-stage-",
        ] {
            match fs::statat(
                &self.directory,
                key_name(prefix, &plan.key).as_str(),
                AtFlags::SYMLINK_NOFOLLOW,
            ) {
                Err(rustix::io::Errno::NOENT) => {}
                Ok(_) => return Err(invalid("retired material remains present")),
                Err(error) => return Err(os_error(error)),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "snapshot_material_retirement_tests.rs"]
mod tests;
