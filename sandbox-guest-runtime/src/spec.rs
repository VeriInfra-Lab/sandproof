use std::collections::BTreeMap;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Already resolved command context. The future SDK adapter owns default/env
/// merging and account resolution; this port does not inherit daemon secrets.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchSpec {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    pub stdin: bool,
    /// A terminal joins stdin/stdout/stderr regardless of the pipe stdin flag.
    pub pty: Option<PtySize>,
    /// Resolved local account. None is the existing current-account component
    /// mode; the authenticated service resolver always supplies Some.
    pub credentials: Option<Credentials>,
}

/// Kernel terminal dimensions, separate from identities or resource budgets.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PtySize {
    pub cols: u16,
    pub rows: u16,
}

impl PtySize {
    /// The pinned Go endpoint narrows wire uint32 dimensions to uint16,
    /// including zero. Keep this explicit; never apply it to identity/budgets.
    #[must_use]
    pub fn from_wire(cols: u32, rows: u32) -> Self {
        let [c0, c1, _, _] = cols.to_le_bytes();
        let [r0, r1, _, _] = rows.to_le_bytes();
        Self {
            cols: u16::from_le_bytes([c0, c1]),
            rows: u16::from_le_bytes([r0, r1]),
        }
    }
}

/// Numeric kernel identity, never inferred from HOME/USER or other environment
/// values. This is supplied by a trusted account resolver, not an SDK field.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Credentials {
    pub uid: u32,
    pub gid: u32,
    pub groups: Vec<u32>,
}

impl Credentials {
    pub(crate) fn validate(&self) -> io::Result<()> {
        if self.uid == u32::MAX
            || self.gid == u32::MAX
            || self.groups.len() > 65_536
            || self.groups.contains(&u32::MAX)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid account identity",
            ));
        }
        Ok(())
    }
}

impl LaunchSpec {
    pub(crate) fn validate(&self) -> io::Result<()> {
        if let Some(credentials) = &self.credentials {
            credentials.validate()?;
        }
        if self.program.is_empty()
            || !self.cwd.is_absolute()
            || self.cwd.as_os_str().as_bytes().contains(&0)
            || self.program.contains('\0')
            || self.args.iter().any(|arg| arg.contains('\0'))
            || self.env.iter().any(|(key, value)| {
                key.is_empty() || key.contains(['=', '\0']) || value.contains('\0')
            })
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid command context",
            ));
        }
        Ok(())
    }
}

/// Explicit component budgets, not production defaults. The owning service
/// must separately bound processes, requests, subscribers and retained ends.
#[derive(Clone, Copy, Debug)]
pub struct ProcessLimits {
    pub chunk_bytes: usize,
    pub launch_bytes: usize,
    pub handshake_timeout: Duration,
}

impl ProcessLimits {
    pub(crate) fn validate(self) -> io::Result<Self> {
        if self.chunk_bytes == 0
            || self.chunk_bytes.checked_mul(2).is_none()
            || self.launch_bytes == 0
            || u32::try_from(self.launch_bytes).is_err()
            || self.handshake_timeout.is_zero()
            || std::time::Instant::now()
                .checked_add(self.handshake_timeout)
                .is_none()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid process budget",
            ));
        }
        Ok(self)
    }
}
