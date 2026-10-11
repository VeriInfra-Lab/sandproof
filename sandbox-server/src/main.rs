#![forbid(unsafe_code)]

#[cfg(target_os = "linux")]
mod gateway;

#[cfg(target_os = "linux")]
mod storage;

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("sandproof-server requires Linux/KVM");
    std::process::exit(1);
}

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    match std::env::args_os().nth(1) {
        Some(arg) if arg == "relay" => gateway::relay().await,
        _ => linux::startup::run().await,
    }
}
