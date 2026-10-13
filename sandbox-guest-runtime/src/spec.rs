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
}

impl LaunchSpec {
    pub(crate) fn validate(&self) -> io::Result<()> {
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
