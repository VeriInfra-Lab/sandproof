//! Read-only current state for the identical source, never a durable permit.
use super::super::{
    Pid, PidfdFlags, ProcessRecord, errno_to_io, helper_gate, pidfd_open, read_proc, validate_id,
    verified_process, wait_pidfd_exit,
};
use super::{OriginalVmSource, PausedOriginalVm, Runtime, RuntimeError, RuntimeProcessIdentity};
use serde::Deserialize;
use std::{
    io::{Read, Write},
    os::{fd::AsRawFd, unix::net::UnixStream},
    path::Path,
    time::Duration,
};

/// A current observation only. Neither Paused nor Exited proves that an old
/// Full worker ended, or grants capture, resume, Stop or position release.
pub enum OriginalPauseObservation {
    /// Same original live source and a Paused response on its checked stream.
    Paused(PausedOriginalVm),
    /// Same original source explicitly reports Running. An old request may
    /// still arrive later; this does not revoke its persistent responsibility.
    Running,
    /// Kernel-confirmed execution exit. An unreaped process cell or other
    /// writers/materials may still exist and remain the original work's duty.
    Exited,
}

#[derive(Debug, PartialEq, Eq)]
enum State {
    Paused,
    Running,
}

impl Runtime {
    /// Reopen and inspect only the source named by the original durable work.
    /// No Pause, Full, Stop, repair, initialization or replacement VM is sent.
    /// The caller retains its original slot/work/fence and writer conditions;
    /// a private handle records this observation, not future continuity.
    /// # Errors
    /// Mismatched source/record/boot/peer is `IdentityMismatch`. Missing or
    /// unavailable records, unknown state, bad/error replies or timeouts stay
    /// errors. A missing proc entry alone never establishes execution exit.
    pub fn observe_original_pause_state(
        &self,
        id: &str,
        expected: &RuntimeProcessIdentity,
    ) -> Result<OriginalPauseObservation, RuntimeError> {
        validate_id(id)?;
        require_record(&self.read_record(id)?, id, expected)?;
        super::super::uid_retirement::require_host_namespaces(Path::new("/proc"))?;
        if source_exited(self, id, expected)? {
            if !source_exited(self, id, expected)? {
                return Err(mismatch("original source changed during exit observation"));
            }
            return Ok(OriginalPauseObservation::Exited);
        }
        let original = self.open_original_source(id, expected)?;
        inspect(original, self.config.vm_uid, self.config.vm_gid, |source| {
            if source_exited(self, id, expected)? {
                return Ok(true);
            }
            self.verify_original_source(source, false)?;
            Ok(false)
        })
    }
}

fn inspect(
    original: OriginalVmSource,
    uid: u32,
    gid: u32,
    mut verify: impl FnMut(&OriginalVmSource) -> Result<bool, RuntimeError>,
) -> Result<OriginalPauseObservation, RuntimeError> {
    if verify(&original)? {
        return Ok(OriginalPauseObservation::Exited);
    }
    let state = query(&original, uid, gid)?;
    if verify(&original)? {
        return Ok(OriginalPauseObservation::Exited);
    }
    Ok(match state {
        State::Paused => OriginalPauseObservation::Paused(PausedOriginalVm { original }),
        State::Running => OriginalPauseObservation::Running,
    })
}

fn mismatch(message: &'static str) -> RuntimeError {
    RuntimeError::IdentityMismatch(message)
}

fn require_record(
    record: &ProcessRecord,
    id: &str,
    expected: &RuntimeProcessIdentity,
) -> Result<(), RuntimeError> {
    expected.validate_fields()?;
    if record.resource_id != id || record.source_identity()? != *expected {
        return Err(mismatch("pause observation original record differs"));
    }
    if helper_gate::boot_id()? != expected.boot_id {
        return Err(mismatch("pause observation host boot changed"));
    }
    Ok(())
}

fn source_exited(
    runtime: &Runtime,
    id: &str,
    expected: &RuntimeProcessIdentity,
) -> Result<bool, RuntimeError> {
    let record = runtime.read_record(id)?;
    require_record(&record, id, expected)?;
    let pid = Pid::from_raw(
        i32::try_from(expected.pid).map_err(|_| mismatch("pause observation PID range"))?,
    )
    .ok_or_else(|| mismatch("pause observation zero PID"))?;
    let pidfd = match pidfd_open(pid, PidfdFlags::empty()) {
        Ok(fd) => fd,
        Err(error) if error == rustix::io::Errno::SRCH => return Ok(true),
        Err(error) => return Err(errno_to_io(error).into()),
    };
    if known_exit(&pidfd, read_proc(expected.pid)?, expected)? {
        return Ok(true);
    }
    if verified_process(&record)?.is_none() {
        if wait_pidfd_exit(&pidfd, Duration::ZERO)? {
            return Ok(true);
        }
        return Err(RuntimeError::Protocol(
            "live original source could not be verified",
        ));
    }
    Ok(false)
}

fn known_exit(
    pidfd: &std::os::fd::OwnedFd,
    status: Option<super::super::ProcStatus>,
    expected: &RuntimeProcessIdentity,
) -> Result<bool, RuntimeError> {
    if let Some(ref status) = status
        && status.start_time_ticks != expected.start_time_ticks
    {
        return Err(mismatch("pause observation PID was reused"));
    }
    let exited = wait_pidfd_exit(pidfd, Duration::ZERO)?;
    if status.is_none() && !exited {
        return Err(RuntimeError::Protocol(
            "live source has no observable proc identity",
        ));
    }
    Ok(exited)
}

fn query(source: &OriginalVmSource, uid: u32, gid: u32) -> Result<State, RuntimeError> {
    source.identity.validate_fields()?;
    // Resolve through this held directory, even if its pathname was renamed.
    // Check the actual connected endpoint; never inspect one socket then use
    // a second connection or accept an arbitrary process sharing the UID.
    let path = format!(
        "/proc/self/fd/{}/{}",
        source.root.as_raw_fd(),
        super::super::API_SOCKET
    );
    let mut stream = UnixStream::connect(path)?;
    let peer = rustix::net::sockopt::socket_peercred(&stream).map_err(errno_to_io)?;
    if u32::try_from(peer.pid.as_raw_nonzero().get()).ok() != Some(source.identity.pid)
        || peer.uid.as_raw() != uid
        || peer.gid.as_raw() != gid
    {
        return Err(mismatch(
            "pause observation connected peer differs from original source",
        ));
    }
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    stream.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;
    decode(&read_reply(&mut stream)?, &source.resource_id)
}

fn read_reply(stream: &mut UnixStream) -> Result<Vec<u8>, RuntimeError> {
    let mut response = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end = loop {
        if let Some(end) = response.windows(4).position(|v| v == b"\r\n\r\n") {
            break end;
        }
        if response.len() > 65_536 {
            return Err(RuntimeError::Protocol("pause observation header limit"));
        }
        let count = stream.read(&mut chunk)?;
        if count == 0 {
            return Err(RuntimeError::Protocol(
                "pause observation incomplete headers",
            ));
        }
        response.extend_from_slice(&chunk[..count]);
    };
    if header_end > 65_536 {
        return Err(RuntimeError::Protocol("pause observation header limit"));
    }
    let header = std::str::from_utf8(&response[..header_end])
        .map_err(|_| RuntimeError::Protocol("pause observation invalid headers"))?;
    let mut lines = header.split("\r\n");
    let status = lines.next().unwrap_or_default();
    let mut parts = status.split_whitespace();
    if !matches!(parts.next(), Some("HTTP/1.1" | "HTTP/1.0")) {
        return Err(RuntimeError::Protocol("pause observation HTTP version"));
    }
    let status: u16 = parts
        .next()
        .and_then(|v| v.parse().ok())
        .ok_or(RuntimeError::Protocol("pause observation HTTP status"))?;
    if status != 200 {
        return Err(RuntimeError::Api {
            status,
            body: String::new(),
        });
    }
    let mut length = None;
    for line in lines {
        let (key, value) = line
            .split_once(':')
            .ok_or(RuntimeError::Protocol("pause observation malformed header"))?;
        if key.eq_ignore_ascii_case("transfer-encoding") {
            return Err(RuntimeError::Protocol(
                "pause observation unsupported transfer encoding",
            ));
        }
        if key.eq_ignore_ascii_case("content-length") {
            let value = value.trim();
            if length.is_some() || value.is_empty() || !value.bytes().all(|v| v.is_ascii_digit()) {
                return Err(RuntimeError::Protocol("pause observation ambiguous length"));
            }
            length = Some(
                value
                    .parse::<usize>()
                    .map_err(|_| RuntimeError::Protocol("pause observation length overflow"))?,
            );
        }
    }
    let length = length.ok_or(RuntimeError::Protocol("pause observation missing length"))?;
    if length > 1_048_576 {
        return Err(RuntimeError::Protocol("pause observation body limit"));
    }
    let body = header_end + 4;
    while response.len() - body < length {
        let count = stream.read(&mut chunk)?;
        if count == 0 {
            return Err(RuntimeError::Protocol("pause observation truncated body"));
        }
        response.extend_from_slice(&chunk[..count]);
    }
    Ok(response[body..body + length].to_vec())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    app_name: String,
    id: String,
    state: String,
    vmm_version: String,
}
fn decode(bytes: &[u8], id: &str) -> Result<State, RuntimeError> {
    let reply: Reply = serde_json::from_slice(bytes)?;
    if reply.id != id || reply.app_name != "Firecracker" || reply.vmm_version != "1.16.1" {
        return Err(mismatch("pause observation instance differs"));
    }
    match reply.state.as_str() {
        "Paused" => Ok(State::Paused),
        "Running" => Ok(State::Running),
        _ => Err(RuntimeError::Protocol("original source state is unknown")),
    }
}

#[cfg(test)]
#[path = "paused_observation_tests.rs"]
mod tests;
