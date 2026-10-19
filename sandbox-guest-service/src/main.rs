#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    sandproof_guest_service::daemon::entry::main()
}

#[cfg(not(target_os = "linux"))]
fn main() -> std::process::ExitCode {
    eprintln!("sandproof-guest-service requires Linux");
    std::process::ExitCode::FAILURE
}
