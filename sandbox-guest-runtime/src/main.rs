#![forbid(unsafe_code)]

#[cfg(target_os = "linux")]
fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    let valid = args.len() == 5 && args[1] == "--gate" && args[3] == "--limit";
    let maximum = valid
        .then(|| args[4].to_str().and_then(|s| s.parse::<u32>().ok()))
        .flatten();
    let Some(maximum) = maximum.filter(|value| *value > 0) else {
        std::process::exit(125);
    };
    if sandproof_guest_runtime::worker::run(std::path::Path::new(&args[2]), maximum as usize)
        .is_err()
    {
        std::process::exit(125);
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("the guest worker requires Linux");
    std::process::exit(1);
}
