//! Readonly CPU affinity facts for an exact original process and its threads.
//! No CPUID/KVM query, affinity mutation, reservation or lifetime permission.
use super::{Runtime, RuntimeError, RuntimeProcessIdentity, parse_proc_stat};
use rustix::fs::{self as rfs, Mode, OFlags};
use std::{collections::BTreeMap, fs::{self, File}, io::{self, Read}, path::Path, time::{Duration, Instant}};

const MAX_CPU: u32 = 8191;
const MAX_CPUS: usize = 256;
const MAX_THREADS: usize = 1024;
const OBSERVATION_TIME: Duration = Duration::from_secs(2);

fn error(reason: &'static str) -> RuntimeError { RuntimeError::IdentityMismatch(reason) }

pub(super) fn validate(cpus: &[u32]) -> Result<(), RuntimeError> {
    if cpus.is_empty() || cpus.len() > MAX_CPUS {
        return Err(error("CPU domain is empty or exceeds supported count"));
    }
    let mut previous = None;
    for &cpu in cpus {
        if cpu > MAX_CPU || previous.is_some_and(|value| value >= cpu) {
            return Err(error("CPU domain is not canonical and bounded"));
        }
        previous = Some(cpu);
    }
    Ok(())
}

// Both lists are canonical. This linear comparison is the actual subset guard;
// validating a profile string or observing only one thread is insufficient.
fn covered(allowed: &[u32], observed: &[u32]) -> bool {
    if validate(allowed).is_err() || validate(observed).is_err() { return false; }
    let mut a=0;
    let mut o=0;
    while a<allowed.len() && o<observed.len() {
        if allowed[a] == observed[o] { o+=1; a+=1; }
        else if allowed[a] < observed[o] { a+=1; }
        else { return false; }
    }
    o==observed.len()
}

fn number(bytes: &[u8]) -> Result<u32, RuntimeError> {
    if bytes.is_empty() || bytes.len()>10 || bytes.iter().any(|b| !b.is_ascii_digit())
        || (bytes.len()>1 && bytes[0]==b'0') {
        return Err(error("noncanonical CPU/process number"));
    }
    std::str::from_utf8(bytes).map_err(|_| error("invalid numeric field"))?
        .parse().map_err(|_| error("numeric field overflow"))
}
fn cpu_list(bytes: &[u8]) -> Result<Vec<u32>, RuntimeError> {
    let mut result=Vec::new();
    for range in bytes.split(|b| *b==b',') {
        let (start,end)=if let Some(at)=range.iter().position(|b| *b==b'-') {
            (number(&range[..at])?,number(&range[at+1..])?)
        } else { let value=number(range)?; (value,value) };
        if start>end || end>MAX_CPU { return Err(error("CPU range exceeds supported domain")); }
        for cpu in start..=end {
            if result.len()==MAX_CPUS { return Err(error("CPU list exceeds supported count")); }
            result.push(cpu);
        }
    }
    validate(&result)?;
    Ok(result)
}
fn status(bytes: &[u8], process: u32, thread: u32) -> Result<Vec<u32>, RuntimeError> {
    let mut pid=None; let mut tgid=None; let mut cpus=None;
    for line in bytes.split(|b| *b==b'\n') {
        if let Some(value)=line.strip_prefix(b"Pid:") {
            if pid.replace(number(value.trim_ascii())?).is_some() { return Err(error("duplicate thread pid")); }
        } else if let Some(value)=line.strip_prefix(b"Tgid:") {
            if tgid.replace(number(value.trim_ascii())?).is_some() { return Err(error("duplicate thread tgid")); }
        } else if let Some(value)=line.strip_prefix(b"Cpus_allowed_list:")
            && cpus.replace(cpu_list(value.trim_ascii())?).is_some() {
                return Err(error("duplicate thread CPU domain"));
        }
    }
    if pid!=Some(thread) || tgid!=Some(process) { return Err(error("thread is not in original process")); }
    cpus.ok_or_else(||error("thread CPU domain missing"))
}
fn read(path: &Path, maximum: u64) -> Result<Vec<u8>, RuntimeError> {
    let mut file=File::from(rfs::open(path,OFlags::RDONLY|OFlags::CLOEXEC|OFlags::NOFOLLOW|OFlags::NONBLOCK,Mode::empty()).map_err(io::Error::from)?);
    if !file.metadata()?.is_file() { return Err(error("CPU metadata not a regular proc file")); }
    let mut bytes=Vec::new();
    Read::by_ref(&mut file).take(maximum+1).read_to_end(&mut bytes)?;
    if bytes.len() as u64>maximum { return Err(error("CPU metadata exceeds read budget")); }
    Ok(bytes)
}
fn start(bytes: &[u8], thread: u32) -> Result<u64,RuntimeError> {
    let space=bytes.iter().position(|b| *b==b' ').ok_or_else(||error("thread stat pid missing"))?;
    if number(&bytes[..space])?!=thread { return Err(error("thread stat pid differs")); }
    let close=bytes.iter().rposition(|b| *b==b')').ok_or_else(||error("thread stat name missing"))?;
    // Kernel comm is arbitrary bytes. Only parse the ASCII suffix, preserving
    // the original parser's field order without requiring a UTF-8 task name.
    let suffix=std::str::from_utf8(&bytes[close+1..]).map_err(|_|error("thread stat fields invalid"))?;
    let normalized=format!("{thread} (thread){suffix}");
    let ticks=parse_proc_stat(&normalized)?.start_time_ticks;
    if ticks==0 { return Err(error("thread start identity missing")); }
    Ok(ticks)
}
#[derive(Debug,PartialEq,Eq)]
struct Thread { start: u64, cpus: Vec<u32> }
fn snapshot(process: u32, deadline: Instant) -> Result<BTreeMap<u32,Thread>,RuntimeError> {
    let mut threads=BTreeMap::new();
    for entry in fs::read_dir(format!("/proc/{process}/task"))? {
        if threads.len()==MAX_THREADS || Instant::now()>=deadline { return Err(error("CPU thread observation budget")); }
        let entry=entry?;
        let thread=number(entry.file_name().as_encoded_bytes())?;
        let first=start(&read(&entry.path().join("stat"),16*1024)?,thread)?;
        let cpus=status(&read(&entry.path().join("status"),128*1024)?,process,thread)?;
        if start(&read(&entry.path().join("stat"),16*1024)?,thread)?!=first
            || threads.insert(thread,Thread{start:first,cpus}).is_some() {
            return Err(error("thread identity changed during CPU observation"));
        }
    }
    if !threads.contains_key(&process) || Instant::now()>=deadline {
        return Err(error("original CPU thread set missing or expired"));
    }
    Ok(threads)
}
fn original_threads(source: &RuntimeProcessIdentity) -> Result<BTreeMap<u32,Thread>,RuntimeError> {
    source.validate_fields()?;
    if super::helper_gate::boot_id()?!=source.boot_id { return Err(error("CPU observation boot differs")); }
    let deadline=Instant::now()+OBSERVATION_TIME;
    let first=snapshot(source.pid,deadline)?;
    let second=snapshot(source.pid,deadline)?;
    if first!=second || first.get(&source.pid).is_none_or(|main| main.start!=source.start_time_ticks) {
        return Err(error("original CPU process/thread identities changed"));
    }
    Ok(first)
}

fn require_covered_threads(allowed:&[u32],launch:&[u32],threads:&BTreeMap<u32,Thread>) -> Result<(),RuntimeError> {
    if !covered(allowed,launch) { return Err(error("original launch CPU domain escapes protected domain")); }
    if threads.is_empty() || threads.values().any(|thread|!covered(allowed,&thread.cpus)) {
        return Err(error("original VMM thread escapes protected CPU domain"));
    }
    Ok(())
}

/// Used only while the original owned launch-gate child is still blocked.
/// The saved value is an actual inherited mask, never a later manager sample.
pub(super) fn launch_domain(source: &RuntimeProcessIdentity) -> Result<Vec<u32>,RuntimeError> {
    let threads=original_threads(source)?;
    if threads.len()!=1 { return Err(error("launch gate must have one original thread")); }
    threads.into_values().next().map(|thread|thread.cpus).ok_or_else(||error("launch CPU domain missing"))
}

impl Runtime {
    /// Check both the recorded pre-exec inherited mask and every current VMM
    /// thread against the ORIGINAL protected CPU domain. No affinity is changed.
    /// The fixed jailer/VMM and deployment must not widen masks between checks;
    /// equal observations alone do not prove that temporal condition or XSAVE.
    /// # Errors
    /// Missing/changed threads, record, proc view or any unbounded/unknown field
    /// stays unresolved. This does not produce capture, Stop or release rights.
    pub fn verify_original_cpu_domain(&self,id:&str,expected:&RuntimeProcessIdentity,allowed:&[u32]) -> Result<(),RuntimeError> {
        validate(allowed)?;
        super::uid_retirement::require_host_namespaces(Path::new("/proc"))?;
        let original=self.open_original_source(id,expected)?;
        let record=self.read_record(id)?;
        let threads=original_threads(expected)?;
        require_covered_threads(allowed,&record.launch_cpu_domain,&threads)?;
        self.verify_original_source(&original,false)?;
        if self.read_record(id)?!=record { return Err(error("CPU observation original claim changed")); }
        Ok(())
    }
}

#[cfg(test)]
#[path="cpu_domain_tests.rs"]
mod tests;
