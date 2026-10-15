//! The sole formal guest binary path. No request fixtures or test-only routes.
use crate::daemon::{
    Prepared,
    config::{CONFIG_BYTES, Configuration},
};
use sandproof_guest_runtime::startup_files;
use std::{
    ffi::OsString,
    fs::File,
    io::{self, Read},
    path::PathBuf,
    process::ExitCode,
};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Check,
    Initialize,
    Serve,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Error {
    Arguments,
    Input,
    Configuration,
    Runtime,
    Signal,
    Service(crate::daemon::Error),
}

/// Execute the one formal CLI, reporting only payload-free stage errors.
#[must_use]
pub fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("guest service failed: {error:?}");
            if error == Error::Arguments {
                eprintln!(
                    "usage: sandproof-guest-service check|initialize|serve --config /absolute/config.json"
                );
            }
            ExitCode::FAILURE
        }
    }
}

fn arguments(mut args: impl Iterator<Item = OsString>) -> Result<(Action, PathBuf), Error> {
    let action = match args.next().as_deref().and_then(std::ffi::OsStr::to_str) {
        Some("check") => Action::Check,
        Some("initialize") => Action::Initialize,
        Some("serve") => Action::Serve,
        _ => return Err(Error::Arguments),
    };
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--config")) {
        return Err(Error::Arguments);
    }
    let path = PathBuf::from(args.next().ok_or(Error::Arguments)?);
    if !path.is_absolute() || args.next().is_some() {
        return Err(Error::Arguments);
    }
    Ok((action, path))
}

fn read(mut file: File) -> Result<Vec<u8>, Error> {
    if file.metadata().map_err(|_| Error::Input)?.len() > CONFIG_BYTES as u64 {
        return Err(Error::Input);
    }
    let mut bytes = Vec::new();
    let mut scratch = [0; 4096];
    loop {
        let maximum = scratch.len().min(CONFIG_BYTES - bytes.len() + 1);
        let n = match file.read(&mut scratch[..maximum]) {
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return Err(Error::Input),
        };
        if n == 0 {
            return Ok(bytes);
        }
        if n > CONFIG_BYTES - bytes.len() {
            return Err(Error::Input);
        }
        bytes.try_reserve_exact(n).map_err(|_| Error::Input)?;
        bytes.extend_from_slice(&scratch[..n]);
    }
}

fn run() -> Result<(), Error> {
    if std::env::args_os()
        .skip(1)
        .eq([OsString::from("--version")])
    {
        println!("{}", crate::build_info::service());
        return Ok(());
    }
    let (action, path) = arguments(std::env::args_os().skip(1))?;
    let file = startup_files::configuration(&path, startup_files::current_owner(), CONFIG_BYTES)
        .map_err(|_| Error::Input)?;
    let bytes = read(file)?;
    let config = Configuration::decode(&bytes).map_err(|_| Error::Configuration)?;
    drop(bytes);
    let workers = config.runtime.worker_threads;
    let blocking = config.runtime.blocking_threads;
    let stack = config.runtime.thread_stack_bytes;
    let shutdown = config.runtime.shutdown_timeout;
    let prepared = Prepared::new(config).map_err(Error::Service)?;
    if action == Action::Check {
        println!("configuration valid; no daemon started");
        return Ok(());
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .max_blocking_threads(blocking)
        .thread_stack_size(stack)
        .enable_all()
        .build()
        .map_err(|_| Error::Runtime)?;
    let result = runtime.block_on(async move {
        if action == Action::Initialize {
            prepared.initialize().await.map_err(Error::Service)?;
            println!("new VM state initialized");
        } else {
            use tokio::signal::unix::{SignalKind, signal};
            let mut terminate = signal(SignalKind::terminate()).map_err(|_| Error::Signal)?;
            let mut interrupt = signal(SignalKind::interrupt()).map_err(|_| Error::Signal)?;
            let shutdown = async move {
                tokio::select! { _ = terminate.recv() => {}, _ = interrupt.recv() => {} }
            };
            let report = prepared.serve(shutdown).await.map_err(Error::Service)?;
            println!("transport stopped: {report:?}");
        }
        Ok(())
    });
    // Waiting is bounded, but a timeout does not certify command/file cleanup.
    // VM lifetime and destruction remain the host's independent responsibility.
    runtime.shutdown_timeout(shutdown);
    result
}
