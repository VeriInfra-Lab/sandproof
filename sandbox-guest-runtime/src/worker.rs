//! Private single-use worker. Only the installed parent's private Unix socket
//! can provide its launch bytes. It executes no user code before complete input.
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

use crate::LaunchSpec;

// v2 separates native errors from protocol faults. Refuse an older worker
// before launch bytes: its generic error marker cannot prove this distinction.
const _: () = assert!(crate::build_info::WORKER_PROTOCOL_VERSION == 2);
pub(crate) const HELLO: &[u8; 4] = b"SG11";
pub(crate) const READY: &[u8; 4] = b"SG12";
pub(crate) const ERROR: &[u8; 4] = b"SGE1";
const FAULT: &[u8; 4] = b"SGF1";
// Retained for decoding the existing v2 handshake; current workers do not
// repeat the parent's cwd preflight after pipe/PTY allocation.
pub(crate) const CWD_MISSING: &[u8; 4] = b"SGC1";
pub(crate) const ENVIRONMENT_NUL: &[u8; 4] = b"SGN1";
pub(crate) const ARGUMENT_NUL: &[u8; 4] = b"SGN2";

/// Run the private worker protocol, then replace this exact process by exec.
/// The worker binary is single-threaded; success does not return.
/// # Errors
/// Returns incomplete/invalid launch input, context or actual exec failure.
pub fn run(path: &Path, maximum: usize) -> io::Result<()> {
    let mut gate = UnixStream::connect(path)?;
    gate.write_all(HELLO)?;
    let mut native_phase = false;
    let result = execute(&mut gate, maximum, &mut native_phase);
    if let Err(error) = &result {
        // A diagnostic code on the PRIVATE gate, never mixed into user stderr.
        // A missing/truncated diagnostic is unknown, not a replay instruction.
        let native = error.raw_os_error();
        let code = native.unwrap_or(22);
        let marker = if native_phase && native.is_some_and(|code| code > 0) {
            ERROR
        } else {
            FAULT
        };
        let _ = gate.write_all(marker);
        let _ = gate.write_all(&code.to_be_bytes());
    }
    result
}

fn execute(gate: &mut UnixStream, maximum: usize, native_phase: &mut bool) -> io::Result<()> {
    let mut header = [0; 4];
    gate.read_exact(&mut header)?;
    let length = u32::from_be_bytes(header) as usize;
    if maximum == 0 || length == 0 || length > maximum {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "launch budget"));
    }
    let mut data = vec![0; length];
    gate.read_exact(&mut data)?;
    let mut spec: LaunchSpec = serde_json::from_slice(&data)?;
    spec.validate()?;
    let wrapped = spec.envd_wrapper_path.is_some();
    // Resolve only optional trusted priority helpers while still gated and
    // under the daemon identity/cwd. The shell and all user code wait for the
    // original durable PID claim and release. Parent cancellation can kill
    // this exact unreleased worker even if filesystem lookup blocks.
    crate::launch_wrapper::prepare(&mut spec, maximum)?;
    // The parent already performed the one cwd Stat before preparing I/O.
    // A later filesystem change must reach actual chdir after release.
    if wrapped && reject_nul(gate, &spec)? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "launch contains NUL",
        ));
    }
    gate.write_all(READY)?;
    let mut release = [0];
    gate.read_exact(&mut release)?;
    if release != [1] {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid release",
        ));
    }
    let mut extra = [0];
    if gate.read(&mut extra)? != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "extra launch bytes",
        ));
    }
    // Only a native OS error after the complete original release may be
    // reported as ExecFailed. Gate, decode and internal preparation faults
    // remain Unknown to the release observer, never a user spawn diagnostic.
    *native_phase = true;
    if spec.pty.is_some() {
        // Only this single-thread worker changes session/controlling terminal,
        // after its gate is released. The daemon never becomes a tty session.
        rustix::process::setsid()?;
        rustix::process::ioctl_tiocsctty(std::io::stdin())?;
    }
    if let Some(credentials) = &spec.credentials {
        switch_account(credentials)?;
    }
    let error = Command::new(spec.program)
        .args(spec.args)
        .current_dir(spec.cwd)
        .env_clear()
        .envs(spec.env)
        .exec();
    Err(error)
}

// Go Cmd.environ runs before StartProcess's argv/cwd string conversion. These
// are validation observations, not actual syscall errno or exec receipts.
fn reject_nul(gate: &mut UnixStream, spec: &LaunchSpec) -> io::Result<bool> {
    use std::os::unix::ffi::OsStrExt;
    let marker = if spec
        .env
        .iter()
        .any(|(key, value)| key.contains('\0') || value.contains('\0'))
    {
        ENVIRONMENT_NUL
    } else if spec.program.contains('\0')
        || spec.args.iter().any(|arg| arg.contains('\0'))
        || spec.cwd.as_os_str().as_bytes().contains(&0)
    {
        ARGUMENT_NUL
    } else {
        return Ok(false);
    };
    gate.write_all(marker)?;
    Ok(true)
}

// Only called inside this private, single-threaded worker, after release and
// before any user exec or cwd/PATH lookup. A partial failure terminates this
// worker through the private diagnostic; it never falls back to daemon root.
fn switch_account(credentials: &crate::Credentials) -> io::Result<()> {
    use rustix::process::{Gid, Uid};
    use rustix::thread::{set_thread_groups, set_thread_res_gid, set_thread_res_uid};
    credentials.validate()?;
    let mut groups: Vec<_> = credentials
        .groups
        .iter()
        .copied()
        .map(Gid::from_raw)
        .collect();
    groups.sort_unstable_by_key(|gid| gid.as_raw());
    let mut current = rustix::process::getgroups()?;
    current.sort_unstable_by_key(|gid| gid.as_raw());
    // Equal groups need no privilege. A failed DIFFERENT group set never
    // silently inherits the daemon's supplementary groups.
    if current != groups {
        set_thread_groups(&groups)?;
    }
    let gid = Gid::from_raw(credentials.gid);
    let uid = Uid::from_raw(credentials.uid);
    set_thread_res_gid(gid, gid, gid)?;
    set_thread_res_uid(uid, uid, uid)?;
    Ok(())
}
