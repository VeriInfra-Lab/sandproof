#[cfg(target_os = "linux")]
fn main() {
    if let Err(error) = run() {
        eprintln!("sandproof-vm: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("sandproof-vm requires Linux/KVM");
    std::process::exit(1);
}

#[cfg(target_os = "linux")]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    use sandbox_runtime::{Runtime, RuntimeConfig};
    use std::path::PathBuf;

    let args: Vec<String> = std::env::args().collect();
    if args.len() != 10
        || !matches!(
            args[1].as_str(),
            "launch" | "observe" | "stop" | "cleanup" | "smoke"
        )
    {
        return Err("usage: sandproof-vm <launch|observe|stop|cleanup|smoke> <resource-id> <jailer> <firecracker> <chroot-base> <kernel> <rootfs> <vm-uid> <vm-gid>".into());
    }
    let config = RuntimeConfig {
        jailer: PathBuf::from(&args[3]),
        firecracker: PathBuf::from(&args[4]),
        chroot_base: PathBuf::from(&args[5]),
        kernel: PathBuf::from(&args[6]),
        rootfs: PathBuf::from(&args[7]),
        vm_uid: args[8].parse()?,
        vm_gid: args[9].parse()?,
        vcpu_count: 1,
        memory_mib: 512,
        daemonize: args[1] != "smoke",
    };
    let runtime = Runtime::new(config)?;
    let observation = match args[1].as_str() {
        "launch" => runtime.launch(&args[2])?,
        "observe" => runtime.observe(&args[2])?,
        "stop" => runtime.stop(&args[2])?,
        "cleanup" => {
            runtime.cleanup(&args[2])?;
            runtime.observe(&args[2])?
        }
        "smoke" => {
            let launched = runtime.launch(&args[2])?;
            println!("{}", serde_json::to_string(&launched)?);
            let log_path = std::path::Path::new(&args[5])
                .join(".claims")
                .join(&args[2])
                .join("console.log");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
            loop {
                let log = std::fs::read_to_string(&log_path)?;
                if log.contains("Ubuntu 24.04") && log.contains("login:") {
                    break;
                }
                if std::time::Instant::now() >= deadline {
                    let _ = runtime.stop(&args[2]);
                    return Err("guest login marker was not observed within 15 seconds".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            let observed = runtime.observe(&args[2])?;
            if observed.pid != launched.pid
                || observed.start_time_ticks != launched.start_time_ticks
            {
                return Err("VM identity changed before second observation".into());
            }
            println!("{}", serde_json::to_string(&observed)?);
            let stopped = runtime.stop(&args[2])?;
            runtime.cleanup(&args[2])?;
            stopped
        }
        _ => unreachable!(),
    };
    println!("{}", serde_json::to_string(&observation)?);
    Ok(())
}
