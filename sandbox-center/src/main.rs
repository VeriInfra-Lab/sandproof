#![forbid(unsafe_code)]

#[cfg(unix)]
mod artifacts;
#[cfg(unix)]
mod audit;
#[cfg(unix)]
mod boundaries;
#[cfg(unix)]
mod build_proxy;
mod cli;
#[cfg(unix)]
mod entry;
#[cfg(unix)]
mod observation;
#[cfg(unix)]
mod runtime;

use cli::Command;

fn main() {
    let result = Command::parse(std::env::args().skip(1)).and_then(execute);
    if let Err(message) = result {
        // Fixed categories only: configuration paths, keys and native errors
        // never become public startup diagnostics.
        eprintln!("{message}");
        std::process::exit(1);
    }
}

fn execute(command: Command) -> Result<(), &'static str> {
    if command == Command::Versions {
        return serde_json::to_writer(
            std::io::stdout().lock(),
            &sandproof_protocol::internal::ContractVersions::current(),
        )
        .map_err(|_| "contract metadata output failed");
    }
    #[cfg(unix)]
    {
        entry::execute(command)
    }
    #[cfg(not(unix))]
    {
        Err("center deployment requires a Unix platform")
    }
}
