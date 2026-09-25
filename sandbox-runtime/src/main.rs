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
    let network_smoke = args.get(1).is_some_and(|mode| mode == "smoke-net");
    if (network_smoke && args.len() != 11)
        || (!network_smoke && args.len() != 10)
        || !matches!(
            args[1].as_str(),
            "launch" | "observe" | "stop" | "cleanup" | "smoke" | "smoke-net"
        )
    {
        return Err("usage: sandproof-vm <launch|observe|stop|cleanup|smoke|smoke-net> <resource-id> <jailer> <firecracker> <chroot-base> <kernel> <rootfs> <vm-uid> <vm-gid> [private-netns-for-smoke-net]".into());
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
        daemonize: args[1] != "smoke" && !network_smoke,
        private_netns: network_smoke.then(|| PathBuf::from(&args[10])),
        // Fixed public test-only digest. Real callers must provide the hash
        // of a fresh high-entropy token through their authenticated service.
        access_token_hash: network_smoke.then(|| "704fd7223ef9ff63d62a667c6b756dc79b0124686d5e5fdb27ffc2761e503d2ea2f5915a118321927e0cdf8e43b8e487364be0857de54683f30ebf700e96ec67".to_owned()),
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
        "smoke" | "smoke-net" => {
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
            let ping_ok = if network_smoke {
                std::process::Command::new("/usr/bin/ip")
                    .args([
                        "netns",
                        "exec",
                        &args[2],
                        "/usr/bin/ping",
                        "-c",
                        "1",
                        "-W",
                        "2",
                        "172.16.0.2",
                    ])
                    .status()?
                    .success()
            } else {
                true
            };
            let stopped = runtime.stop(&args[2])?;
            runtime.cleanup(&args[2])?;
            if !ping_ok {
                return Err("guest was not reachable inside the private namespace".into());
            }
            stopped
        }
        _ => unreachable!(),
    };
    println!("{}", serde_json::to_string(&observation)?);
    Ok(())
}
