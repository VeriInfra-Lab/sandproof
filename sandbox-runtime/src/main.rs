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
    let network_mode = args.get(1).is_some_and(|mode| mode.ends_with("-net"));
    if (network_mode && args.len() != 11)
        || (!network_mode && args.len() != 10)
        || !matches!(
            args[1].as_str(),
            "launch"
                | "observe"
                | "stop"
                | "cleanup"
                | "smoke"
                | "smoke-net"
                | "launch-net"
                | "observe-net"
                | "stop-net"
                | "cleanup-net"
        )
    {
        return Err("usage: sandproof-vm <launch|observe|stop|cleanup|smoke>[-net] <resource-id> <jailer> <firecracker> <chroot-base> <kernel> <rootfs> <vm-uid> <vm-gid> [private-netns]; network commands require SANDBOX_ENVD_TOKEN_SHA512 (smoke-net has a public test default)".into());
    }
    let access_token_hash = if network_mode {
        Some(std::env::var("SANDBOX_ENVD_TOKEN_SHA512").or_else(|error| {
            if network_smoke {
                Ok("704fd7223ef9ff63d62a667c6b756dc79b0124686d5e5fdb27ffc2761e503d2ea2f5915a118321927e0cdf8e43b8e487364be0857de54683f30ebf700e96ec67".to_owned())
            } else {
                Err(error)
            }
        })?)
    } else {
        None
    };
    let config = RuntimeConfig {
        guest_instance_id: args[2].clone(),
        console_policy: None,
        host_slot: None,
        jailer: PathBuf::from(&args[3]),
        firecracker: PathBuf::from(&args[4]),
        chroot_base: PathBuf::from(&args[5]),
        kernel: PathBuf::from(&args[6]),
        rootfs: PathBuf::from(&args[7]),
        vm_uid: args[8].parse()?,
        vm_gid: args[9].parse()?,
        vcpu_count: 1,
        memory_mib: 512,
        host_resources: std::env::var("SANDBOX_HOST_RESOURCES")
            .ok()
            .map(|value| serde_json::from_str(&value))
            .transpose()?,
        private_netns: network_mode.then(|| PathBuf::from(&args[10])),
        access_token_hash,
    };
    let runtime = Runtime::new(config)?;
    let observation = match args[1].as_str() {
        "launch" | "launch-net" => runtime.launch(&args[2])?,
        "observe" | "observe-net" => runtime.observe(&args[2])?,
        "stop" | "stop-net" => runtime.stop(&args[2])?,
        "cleanup" | "cleanup-net" => {
            runtime.cleanup(&args[2])?;
            runtime.observe(&args[2])?
        }
        "smoke" | "smoke-net" => smoke(
            &runtime,
            &args[2],
            std::path::Path::new(&args[5]),
            network_smoke,
        )?,
        _ => unreachable!(),
    };
    println!("{}", serde_json::to_string(&observation)?);
    Ok(())
}

#[cfg(target_os = "linux")]
fn smoke(
    runtime: &sandbox_runtime::Runtime,
    id: &str,
    chroot_base: &std::path::Path,
    network: bool,
) -> Result<sandbox_runtime::Observation, Box<dyn std::error::Error>> {
    let launched = runtime.launch(id)?;
    println!("{}", serde_json::to_string(&launched)?);
    let checks = (|| -> Result<(), Box<dyn std::error::Error>> {
        let log_path = chroot_base.join(".claims").join(id).join("console.log");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            let log = std::fs::read_to_string(&log_path)?;
            if log.contains("Ubuntu 24.04") && log.contains("login:") {
                break;
            }
            if std::time::Instant::now() >= deadline {
                return Err("guest login marker was not observed within 15 seconds".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let observed = runtime.observe(id)?;
        if observed.pid != launched.pid || observed.start_time_ticks != launched.start_time_ticks {
            return Err("VM identity changed before second observation".into());
        }
        println!("{}", serde_json::to_string(&observed)?);
        let ping_ok = if network {
            std::process::Command::new("/usr/bin/ip")
                .args([
                    "netns",
                    "exec",
                    id,
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
        if !ping_ok {
            return Err("guest was not reachable inside the private namespace".into());
        }
        Ok(())
    })();
    let stopped = runtime.stop(id)?;
    runtime.cleanup(id)?;
    checks?;
    Ok(stopped)
}
