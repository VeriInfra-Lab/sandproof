//! Bounded read-only kernel observations; these never release logical quotas.
use std::{error::Error, fs::File, io::Read};

pub(super) fn available_compute() -> Result<(u64, u64), Box<dyn Error>> {
    let cpu = u64::try_from(std::thread::available_parallelism()?.get())?;
    let mut input = String::new();
    File::open("/proc/meminfo")?
        .take(64 * 1024 + 1)
        .read_to_string(&mut input)?;
    if input.len() > 64 * 1024 {
        return Err("host memory observation exceeds fixed bound".into());
    }
    Ok((cpu, memory_available(&input)?))
}
fn memory_available(input: &str) -> Result<u64, Box<dyn Error>> {
    let mut available = None;
    for line in input.lines() {
        let Some(value) = line.strip_prefix("MemAvailable:") else {
            continue;
        };
        let mut fields = value.split_whitespace();
        let bytes = fields
            .next()
            .ok_or("missing host memory count")?
            .parse::<u64>()?
            .checked_mul(1024)
            .ok_or("host memory count overflow")?;
        if fields.next() != Some("kB")
            || fields.next().is_some()
            || available.replace(bytes).is_some()
        {
            return Err("ambiguous host memory observation".into());
        }
    }
    available.ok_or_else(|| "missing host available memory".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_memory_veto_requires_exact_units_and_one_kernel_field() {
        assert_eq!(
            memory_available("MemTotal: 100 kB\nMemAvailable: 42 kB\n").unwrap(),
            42 * 1024
        );
        assert_eq!(memory_available("MemAvailable: 0 kB\n").unwrap(), 0);
        for value in [
            "MemFree: 1 kB",
            "MemAvailable: 1 GB",
            "MemAvailable: 1 kB extra",
            "MemAvailable: 1 kB\nMemAvailable: 2 kB",
            "MemAvailable: 18446744073709551615 kB",
        ] {
            assert!(memory_available(value).is_err());
        }
    }
    #[test]
    fn real_host_kernel_observation_has_finite_cpu_and_memory() {
        let (cpu, _) = available_compute().unwrap();
        assert!(cpu > 0);
    }
}
