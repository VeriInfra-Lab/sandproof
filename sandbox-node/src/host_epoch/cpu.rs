use super::{Result, error, files::host_text};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::Path};

const MAX_CPU_ID: u32 = 8191;
const MAX_ALLOWED: usize = 256;
const HOMOGENEOUS: [&str; 6] = [
    "vendor_id",
    "cpu family",
    "model",
    "stepping",
    "microcode",
    "flags",
];
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CpuDescription {
    processor: u32,
    stable: BTreeMap<String, String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CpuDomain {
    online: Vec<u32>,
    manager_allowed: Vec<u32>,
    descriptions: Vec<CpuDescription>,
}

impl CpuDomain {
    pub(super) fn allowed_cpus(&self) -> &[u32] {
        &self.manager_allowed
    }
}

fn number(value: &str) -> Result<u32> {
    if value.is_empty()
        || !value.bytes().all(|b| b.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err(error("noncanonical CPU identifier"));
    }
    let id = value
        .parse::<u32>()
        .map_err(|_| error("CPU identifier overflow"))?;
    if id > MAX_CPU_ID {
        return Err(error("CPU identifier exceeds bootstrap range"));
    }
    Ok(id)
}
fn list(value: &str, maximum: usize) -> Result<Vec<u32>> {
    let mut cpus = Vec::new();
    for part in value.split(',') {
        let (start, end) = if let Some((start, end)) = part.split_once('-') {
            (number(start)?, number(end)?)
        } else {
            let id = number(part)?;
            (id, id)
        };
        if start > end || cpus.last().is_some_and(|last| *last >= start) {
            return Err(error("CPU ranges overlap or are unordered"));
        }
        for cpu in start..=end {
            if cpus.len() == maximum {
                return Err(error("CPU domain exceeds bootstrap count"));
            }
            cpus.push(cpu);
        }
    }
    if cpus.is_empty() {
        return Err(error("empty CPU domain"));
    }
    Ok(cpus)
}
fn status_allowed(status: &str) -> Result<Vec<u32>> {
    let mut allowed = None;
    for line in status.lines() {
        if let Some(value) = line.strip_prefix("Cpus_allowed_list:") {
            if allowed.is_some() {
                return Err(error("duplicate manager CPU domain"));
            }
            allowed = Some(list(value.trim(), MAX_ALLOWED)?);
        }
    }
    allowed.ok_or_else(|| error("missing manager CPU domain"))
}
fn descriptions(cpuinfo: &str, allowed: &[u32]) -> Result<Vec<CpuDescription>> {
    let mut all = BTreeMap::new();
    for block in cpuinfo.split("\n\n").filter(|part| !part.trim().is_empty()) {
        if all.len() > MAX_CPU_ID as usize {
            return Err(error("too many CPU descriptions"));
        }
        let mut fields = BTreeMap::new();
        let mut processor = None;
        for line in block.lines() {
            let (key, value) = line
                .split_once(':')
                .ok_or_else(|| error("malformed CPU description"))?;
            let key = key.trim();
            let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
            if key.is_empty() || key.len() > 128 || value.len() > 16384 {
                return Err(error("CPU field exceeds bootstrap bound"));
            }
            if key == "processor" {
                if processor.replace(number(&value)?).is_some() {
                    return Err(error("duplicate processor field"));
                }
            } else if !matches!(key, "cpu MHz" | "bogomips")
                && fields.insert(key.to_owned(), value).is_some()
            {
                return Err(error("duplicate stable CPU field"));
            }
        }
        let processor = processor.ok_or_else(|| error("missing processor identity"))?;
        if all.insert(processor, fields).is_some() {
            return Err(error("duplicate CPU description"));
        }
    }
    let mut result = Vec::new();
    for id in allowed {
        let stable = all
            .remove(id)
            .ok_or_else(|| error("allowed CPU has no actual description"))?;
        if HOMOGENEOUS
            .iter()
            .any(|key| stable.get(*key).is_none_or(String::is_empty))
        {
            return Err(error(
                "allowed CPU lacks stable identity/features/microcode",
            ));
        }
        if !matches!(
            stable.get("vendor_id").map(String::as_str),
            Some("GenuineIntel" | "AuthenticAMD")
        ) {
            return Err(error("unsupported CPU vendor in fixed bootstrap profile"));
        }
        if let Some(first) = result.first() {
            let first: &CpuDescription = first;
            if HOMOGENEOUS
                .iter()
                .any(|key| first.stable.get(*key) != stable.get(*key))
            {
                return Err(error("heterogeneous allowed CPU domain unsupported"));
            }
        }
        result.push(CpuDescription {
            processor: *id,
            stable,
        });
    }
    Ok(result)
}

pub(super) fn read_domain() -> Result<CpuDomain> {
    let online = list(
        &host_text(Path::new("/sys/devices/system/cpu/online"), 65536)?,
        MAX_CPU_ID as usize + 1,
    )?;
    let allowed = status_allowed(&host_text(
        Path::new("/proc/thread-self/status"),
        128 * 1024,
    )?)?;
    if allowed.iter().any(|cpu| online.binary_search(cpu).is_err()) {
        return Err(error("manager CPU domain includes offline CPUs"));
    }
    let descriptions = descriptions(
        &host_text(Path::new("/proc/cpuinfo"), 16 * 1024 * 1024)?,
        &allowed,
    )?;
    let mut count = 0;
    for entry in fs::read_dir("/proc/self/task")? {
        count += 1;
        if count > 1024 {
            return Err(error("manager thread observation budget exceeded"));
        }
        let entry = entry?;
        if !entry
            .file_name()
            .as_encoded_bytes()
            .iter()
            .all(u8::is_ascii_digit)
            || status_allowed(&host_text(&entry.path().join("status"), 128 * 1024)?)? != allowed
        {
            return Err(error("manager threads differ in declared CPU domain"));
        }
    }
    if count == 0 {
        return Err(error("manager thread observation is empty"));
    }
    // Manager threads are not VMM threads. No claim about the original VM's
    // actual affinity or guest CPUID is inferred from this current-process view.
    Ok(CpuDomain {
        online,
        manager_allowed: allowed,
        descriptions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const CPU: &str = "processor : 0\nvendor_id : GenuineIntel\ncpu family : 6\nmodel : 85\nstepping : 7\nmicrocode : 0x123\nflags : fpu sse\ncpu MHz : 1000\nbogomips : 2000\nphysical id : 0\n";
    #[test]
    fn cpu_ranges_reject_ambiguous_unbounded_or_missing_sets() {
        assert_eq!(list("0-2,5", 4).unwrap(), [0, 1, 2, 5]);
        for value in [
            "", "1-0", "0,0", "2,1", "0-2,2", "01", "+1", "8192", "0-8191",
        ] {
            assert!(list(value, 256).is_err(), "{value}");
        }
        assert_eq!(
            status_allowed("Name: manager\nCpus_allowed_list:\t2-3\n").unwrap(),
            [2, 3]
        );
        assert!(status_allowed("Cpus_allowed_list: 0\nCpus_allowed_list: 0").is_err());
        assert!(status_allowed("Cpus_allowed: ff").is_err());
    }
    #[test]
    fn all_allowed_cpus_require_stable_matching_features_and_microcode() {
        let first = descriptions(CPU, &[0]).unwrap();
        let varied = CPU.replace("1000", "4000").replace("2000", "9000");
        assert_eq!(first, descriptions(&varied, &[0]).unwrap());
        assert!(descriptions(CPU, &[0, 1]).is_err());
        let second = CPU
            .replace("processor : 0", "processor : 1")
            .replace("physical id : 0", "physical id : 1");
        let both = format!("{CPU}\n{second}");
        assert_eq!(descriptions(&both, &[0, 1]).unwrap().len(), 2);
        assert!(descriptions(&both.replace("processor : 1", "processor : 0"), &[0, 1]).is_err());
        let changed = second.replace("0x123", "0x124");
        assert!(descriptions(&format!("{CPU}\n{changed}"), &[0, 1]).is_err());
        for field in [
            "vendor_id",
            "cpu family",
            "model",
            "stepping",
            "microcode",
            "flags",
        ] {
            let incomplete = CPU
                .lines()
                .filter(|line| line.split_once(':').unwrap().0.trim() != field)
                .collect::<Vec<_>>()
                .join("\n");
            assert!(descriptions(&incomplete, &[0]).is_err());
        }
    }
}
