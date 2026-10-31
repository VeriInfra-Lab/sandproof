//! Fixed project labels on original material directory FDs. Deployment alone
//! enables and reads back the filesystem hard quotas; these calls cannot change
//! a quota or choose another device. A changed external quota remains a breach
//! of the explicit protected-deployment guarantee, not a monitored condition.
use crate::{HelperLimits, OwnedChild, RuntimeError};
use rustix::fs::{self, Mode, OFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, Metadata},
    io::{self, Read},
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    path::{Component, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

const TOOL_MAX_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectTool {
    pub path: PathBuf,
    pub sha256: [u8; 32],
}

/// Protected operator input, never accepted from a lifecycle work request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialProjectSettings {
    pub project_id: u32,
    pub hard_bytes: u64,
    pub hard_inodes: u64,
    pub chattr: ProjectTool,
    pub lsattr: ProjectTool,
}
impl MaterialProjectSettings {
    /// Pure bounds only; this does not attest a kernel quota or tool file.
    /// # Errors
    /// Rejects zero/overflowing bounds and noncanonical fixed tool paths.
    pub fn validate(&self) -> Result<(), RuntimeError> {
        if self.project_id == 0
            || self.hard_bytes == 0
            || i64::try_from(self.hard_bytes).is_err()
            || self.hard_inodes == 0
            || i64::try_from(self.hard_inodes).is_err()
            || [&self.chattr, &self.lsattr].into_iter().any(|tool| {
                !tool.path.is_absolute()
                    || tool.path.file_name().is_none()
                    || tool
                        .path
                        .components()
                        .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
            })
        {
            return Err(error("invalid protected material project"));
        }
        Ok(())
    }
}

struct Tool {
    file: File,
    stamp: Stamp,
}
#[derive(PartialEq, Eq)]
struct Stamp {
    device: u64,
    inode: u64,
    bytes: u64,
    modified: (i64, i64),
    changed: (i64, i64),
    mode: u32,
    uid: u32,
}
impl Stamp {
    fn read(meta: &Metadata) -> Self {
        Self {
            device: meta.dev(),
            inode: meta.ino(),
            bytes: meta.len(),
            modified: (meta.mtime(), meta.mtime_nsec()),
            changed: (meta.ctime(), meta.ctime_nsec()),
            mode: meta.mode(),
            uid: meta.uid(),
        }
    }
}
fn error(message: &'static str) -> RuntimeError {
    RuntimeError::IdentityMismatch(message)
}
fn errno(error: rustix::io::Errno) -> RuntimeError {
    RuntimeError::Io(io::Error::from(error))
}
impl Tool {
    fn open(settings: &ProjectTool) -> Result<Self, RuntimeError> {
        crate::check_trusted_path(&settings.path)?;
        let mut file = File::from(
            fs::open(
                &settings.path,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(errno)?,
        );
        let meta = file.metadata()?;
        if !meta.is_file()
            || meta.uid() != 0
            || meta.mode() & 0o022 != 0
            || meta.mode() & 0o111 == 0
            || meta.len() == 0
            || meta.len() > TOOL_MAX_BYTES
        {
            return Err(error("material project tool identity"));
        }
        let stamp = Stamp::read(&meta);
        let mut hash = Sha256::new();
        let mut buffer = [0_u8; 8192];
        let mut bytes = 0_u64;
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            bytes += count as u64;
            if bytes > TOOL_MAX_BYTES {
                return Err(error("material project tool grew"));
            }
            hash.update(&buffer[..count]);
        }
        let digest: [u8; 32] = hash.finalize().into();
        if bytes != meta.len()
            || digest != settings.sha256
            || Stamp::read(&file.metadata()?) != stamp
        {
            return Err(error("material project tool fingerprint"));
        }
        Ok(Self { file, stamp })
    }
    fn run(&self, args: &[&str]) -> Result<Vec<u8>, RuntimeError> {
        if Stamp::read(&self.file.metadata()?) != self.stamp {
            return Err(error("material project tool changed"));
        }
        let executable = format!("/proc/{}/fd/{}", std::process::id(), self.file.as_raw_fd());
        let mut command = Command::new(executable);
        command
            .args(args)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("LANG", "C")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let result = OwnedChild::spawn(&mut command)?.exchange(
            &[],
            HelperLimits {
                timeout: Duration::from_secs(3),
                input_bytes: 0,
                stdout_bytes: 1024,
                stderr_bytes: 1024,
            },
        )?;
        if !result.status.success()
            || !result.stderr.is_empty()
            || Stamp::read(&self.file.metadata()?) != self.stamp
        {
            return Err(error("material project tool result unknown"));
        }
        Ok(result.stdout)
    }
}

pub(crate) struct MaterialProject {
    settings: MaterialProjectSettings,
    device: u64,
    store: File,
    chattr: Tool,
    lsattr: Tool,
}
impl MaterialProject {
    pub(crate) fn open(
        settings: MaterialProjectSettings,
        store: &File,
    ) -> Result<Self, RuntimeError> {
        settings.validate()?;
        crate::snapshot_store::require_same_xfs_mount(store, store)?;
        let result = Self {
            device: store.metadata()?.dev(),
            store: store.try_clone()?,
            chattr: Tool::open(&settings.chattr)?,
            lsattr: Tool::open(&settings.lsattr)?,
            settings,
        };
        result.require(store)?;
        Ok(result)
    }
    pub(crate) fn provision(
        settings: MaterialProjectSettings,
        store: &File,
    ) -> Result<Self, RuntimeError> {
        settings.validate()?;
        crate::snapshot_store::require_same_xfs_mount(store, store)?;
        let result = Self {
            device: store.metadata()?.dev(),
            store: store.try_clone()?,
            chattr: Tool::open(&settings.chattr)?,
            lsattr: Tool::open(&settings.lsattr)?,
            settings,
        };
        result.assign_new_empty(store)?;
        Ok(result)
    }
    fn target(&self, directory: &File) -> Result<String, RuntimeError> {
        crate::snapshot_store::require_same_xfs_mount(&self.store, directory)?;
        let meta = directory.metadata()?;
        if !meta.is_dir()
            || meta.dev() != self.device
            || meta.uid() != 0
            || meta.mode() & 0o077 != 0
        {
            return Err(error("material project original directory"));
        }
        // The final dot is a directory, not the proc FD symlink rejected by
        // e2fsprogs O_NOFOLLOW. The original FD stays held for every tool call.
        Ok(format!(
            "/proc/{}/fd/{}/.",
            std::process::id(),
            directory.as_raw_fd()
        ))
    }
    pub(crate) fn require(&self, directory: &File) -> Result<(), RuntimeError> {
        let target = self.target(directory)?;
        let bytes = self.lsattr.run(&["-pd", &target])?;
        if !project_matches(&bytes, self.settings.project_id, &target) {
            return Err(error("material project label or inheritance differs"));
        }
        self.target(directory)?;
        Ok(())
    }
    pub(crate) fn assign_new_empty(&self, directory: &File) -> Result<(), RuntimeError> {
        let target = self.target(directory)?;
        let mut entries = fs::Dir::read_from(directory).map_err(errno)?;
        for entry in &mut entries {
            let entry = entry.map_err(errno)?;
            if !matches!(entry.file_name().to_bytes(), b"." | b"..") {
                return Err(error(
                    "material project assignment requires new empty directory",
                ));
            }
        }
        let output =
            self.chattr
                .run(&["-p", &self.settings.project_id.to_string(), "+P", &target])?;
        if !output.is_empty() {
            return Err(error("material project unexpected output"));
        }
        self.require(directory)?;
        directory.sync_all()?;
        Ok(())
    }
}
fn project_matches(bytes: &[u8], project: u32, target: &str) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let mut fields = text.split_ascii_whitespace();
    fields.next().and_then(|value| value.parse::<u32>().ok()) == Some(project)
        && fields.next().is_some_and(|flags| {
            flags.len() == 22
                && flags.contains('P')
                && flags
                    .bytes()
                    .all(|byte| byte == b'-' || byte.is_ascii_alphabetic())
        })
        && fields.next() == Some(target)
        && fields.next().is_none()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_original_project_frame_is_required() {
        let target = "/proc/11/fd/7/.";
        let good = format!("63801 -------------------P-- {target}\n");
        assert!(project_matches(good.as_bytes(), 63801, target));
        for wrong in [
            good.replace("63801", "0"),
            good.replace('P', "-"),
            good.replace("/fd/7/", "/fd/8/"),
            format!("{good}noise\n"),
            String::new(),
        ] {
            assert!(!project_matches(wrong.as_bytes(), 63801, target));
        }
    }
}
