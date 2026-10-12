//! Private single-use worker. Only the installed parent's private Unix socket
//! can provide its launch bytes. It executes no user code before complete input.
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

use crate::LaunchSpec;

pub(crate) const HELLO: &[u8; 4] = b"SG01";
pub(crate) const READY: &[u8; 4] = b"SG02";
pub(crate) const ERROR: &[u8; 4] = b"SGE1";

/// Run the private worker protocol, then replace this exact process by exec.
/// The worker binary is single-threaded; success does not return.
/// # Errors
/// Returns incomplete/invalid launch input, context or actual exec failure.
pub fn run(path: &Path, maximum: usize) -> io::Result<()> {
    let mut gate = UnixStream::connect(path)?;
    gate.write_all(HELLO)?;
    let result = execute(&mut gate, maximum);
    if let Err(error) = &result {
        // A diagnostic code on the PRIVATE gate, never mixed into user stderr.
        // A missing/truncated diagnostic is unknown, not a replay instruction.
        let code = error.raw_os_error().unwrap_or(22);
        let _ = gate.write_all(ERROR);
        let _ = gate.write_all(&code.to_be_bytes());
    }
    result
}

fn execute(gate: &mut UnixStream, maximum: usize) -> io::Result<()> {
    let mut header = [0; 4];
    gate.read_exact(&mut header)?;
    let length = u32::from_be_bytes(header) as usize;
    if maximum == 0 || length == 0 || length > maximum {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "launch budget"));
    }
    let mut data = vec![0; length];
    gate.read_exact(&mut data)?;
    let spec: LaunchSpec = serde_json::from_slice(&data)?;
    spec.validate()?;
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
    let error = Command::new(spec.program)
        .args(spec.args)
        .current_dir(spec.cwd)
        .env_clear()
        .envs(spec.env)
        .exec();
    Err(error)
}
