#![forbid(unsafe_code)]

mod cli;

#[cfg(target_os = "linux")]
mod storage;

#[cfg(not(target_os = "linux"))]
fn main() {
    // Exercise the same parser, but never expose another platform backend.
    let _ = cli::parse(std::env::args_os().skip(1));
    eprintln!("sandproof-host requires Linux/KVM");
    std::process::exit(1);
}

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use cli::Command;
    match cli::parse(std::env::args_os().skip(1))? {
        Command::Initialize(path) => linux::host_entry::initialize(&path),
        Command::ProvisionBootstrap(path) => linux::host_entry::provision_bootstrap(&path),
        Command::Serve(path) => linux::host_entry::serve(&path).await,
        Command::Audit(path) => linux::host_entry::audit(&path),
        Command::ObserveCenter(path) => linux::host_entry::observe_center(&path).await,
        Command::Relay => sandproof_host_service::data_gateway::relay().await,
        Command::ContractVersions => {
            use std::io::Write;
            let mut output = std::io::stdout().lock();
            serde_json::to_writer(
                &mut output,
                &sandproof_protocol::internal::ContractVersions::current(),
            )?;
            writeln!(output)?;
            Ok(())
        }
    }
}
