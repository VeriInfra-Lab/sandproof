//! UID absence observed only while the original incarnation is closed and the
//! exclusive position/helper protocol forbids any later old-generation spawn.
use super::RuntimeError;
use std::{
    ffi::OsStr,
    fs,
    io::{self, Read},
    os::unix::fs::MetadataExt,
    path::Path,
    time::{Duration, Instant},
};

const MAX_STATUS_BYTES: u64 = 128 * 1024;
const MAX_MOUNT_BYTES: u64 = 1024 * 1024;

struct Budget {
    deadline: Instant,
    entries: usize,
    limit: usize,
}
impl Budget {
    fn check(&self) -> Result<(), RuntimeError> {
        if Instant::now() >= self.deadline {
            return Err(RuntimeError::TimedOut("retired UID observation"));
        }
        Ok(())
    }
    fn entry(&mut self) -> Result<(), RuntimeError> {
        self.check()?;
        if self.entries >= self.limit {
            return Err(error("retired UID observation entry budget"));
        }
        self.entries += 1;
        Ok(())
    }
}

fn read_metadata(path: &Path, maximum: u64) -> Result<Vec<u8>, io::Error> {
    let file = fs::File::from(
        rustix::fs::open(
            path,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::NONBLOCK
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map_err(io::Error::from)?,
    );
    if !file.metadata()?.is_file() {
        return Err(io::Error::from(io::ErrorKind::InvalidData));
    }
    let mut bytes = Vec::new();
    file.take(maximum + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > maximum {
        return Err(io::Error::from(io::ErrorKind::InvalidData));
    }
    Ok(bytes)
}

fn error(message: &'static str) -> RuntimeError {
    RuntimeError::IdentityMismatch(message)
}

fn numeric(name: &OsStr) -> bool {
    name.as_encoded_bytes().iter().all(u8::is_ascii_digit) && !name.is_empty()
}

fn vanished(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::NotFound
        || error.raw_os_error() == Some(rustix::io::Errno::SRCH.raw_os_error())
}

// Missing status/task is not sufficient: a still-present directory remains
// unresolved. A replacement at the same pathname also fails conservatively.
fn directory_vanished(path: &Path, failure: &io::Error) -> Result<bool, RuntimeError> {
    if !vanished(failure) {
        return Err(io::Error::new(failure.kind(), failure.to_string()).into());
    }
    match fs::symlink_metadata(path) {
        Err(error) if vanished(&error) => Ok(true),
        Err(error) => Err(error.into()),
        Ok(_) => Err(error("UID observation missing for present process/task")),
    }
}

fn status_uids(status: &[u8]) -> Result<[u32; 4], RuntimeError> {
    let mut result = None;
    for line in status.split(|byte| *byte == b'\n') {
        let Some(values) = line.strip_prefix(b"Uid:") else {
            continue;
        };
        if result.is_some() {
            return Err(error("duplicate proc UID fields"));
        }
        let values = values
            .split(u8::is_ascii_whitespace)
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        if values.len() != 4 {
            return Err(error("proc UID field count"));
        }
        let mut parsed = [0_u32; 4];
        for (index, value) in values.into_iter().enumerate() {
            if !value.iter().all(u8::is_ascii_digit) {
                return Err(error("nondecimal proc UID field"));
            }
            parsed[index] = std::str::from_utf8(value)
                .map_err(|_| error("proc UID encoding"))?
                .parse()
                .map_err(|_| error("proc UID range"))?;
        }
        result = Some(parsed);
    }
    result.ok_or(error("missing proc UID fields"))
}

fn require_task_unused(task: &Path, uid: u32) -> Result<(), RuntimeError> {
    let status = match read_metadata(&task.join("status"), MAX_STATUS_BYTES) {
        Ok(status) => status,
        Err(error) if directory_vanished(task, &error)? => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    // Do not skip zombies or accept only the effective UID. A non-leader
    // thread may retain another real/saved/fs identity or a stale namespace FD.
    if status_uids(&status)?.contains(&uid) {
        return Err(error("retired UID still has a task"));
    }
    Ok(())
}

fn require_uid_unheld(proc_root: &Path, uid: u32, budget: &mut Budget) -> Result<(), RuntimeError> {
    scan_uid_with(proc_root, uid, budget, |_| Ok(()))
}

fn scan_uid_with(
    proc_root: &Path,
    uid: u32,
    budget: &mut Budget,
    mut before_task: impl FnMut(&Path) -> Result<(), RuntimeError>,
) -> Result<(), RuntimeError> {
    if uid == 0 || uid == u32::MAX {
        return Err(error("invalid retired VM UID"));
    }
    for process in fs::read_dir(proc_root)? {
        budget.entry()?;
        let process = process?;
        if !numeric(&process.file_name()) {
            continue;
        }
        let tasks = match fs::read_dir(process.path().join("task")) {
            Ok(tasks) => tasks,
            Err(error) if directory_vanished(&process.path(), &error)? => continue,
            Err(error) => return Err(error.into()),
        };
        for task in tasks {
            budget.entry()?;
            let task = task?;
            if numeric(&task.file_name()) {
                before_task(&task.path())?;
                require_task_unused(&task.path(), uid)?;
                budget.check()?;
            }
        }
    }
    budget.check()
}

fn single_namespace_pid(status: &[u8], expected: u32) -> Result<(), RuntimeError> {
    let values = status
        .split(|byte| *byte == b'\n')
        .filter_map(|line| line.strip_prefix(b"NSpid:"))
        .collect::<Vec<_>>();
    if values.len() != 1 {
        return Err(error("missing or duplicate proc PID namespace identity"));
    }
    let fields = values[0]
        .split(u8::is_ascii_whitespace)
        .filter(|field| !field.is_empty())
        .collect::<Vec<_>>();
    if fields.len() != 1 || fields[0] != expected.to_string().as_bytes() {
        return Err(error("restricted proc PID namespace view"));
    }
    Ok(())
}

fn unrestricted_proc_mount(bytes: &[u8]) -> Result<(), RuntimeError> {
    let mut matches = 0;
    for line in bytes.split(|byte| *byte == b'\n') {
        let fields = line
            .split(u8::is_ascii_whitespace)
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        if fields.is_empty() {
            continue;
        }
        if fields.len() < 7 {
            return Err(error("malformed proc mount metadata"));
        }
        if fields[4] != b"/proc" {
            continue;
        }
        matches += 1;
        let split = fields
            .iter()
            .position(|field| *field == b"-")
            .ok_or(error("proc mount metadata"))?;
        if split < 6 || fields.len() != split + 4 || fields[split + 1] != b"proc" {
            return Err(error("unexpected proc filesystem"));
        }
        for options in [fields[5], fields[split + 3]] {
            for option in options.split(|byte| *byte == b',') {
                if option.starts_with(b"hidepid=")
                    && option != b"hidepid=0"
                    && option != b"hidepid=off"
                {
                    return Err(error("filtered proc process view"));
                }
                if option.starts_with(b"subset=") {
                    return Err(error("restricted proc subset"));
                }
            }
        }
    }
    if matches != 1 {
        return Err(error("missing or ambiguous host proc mount"));
    }
    Ok(())
}

/// These checks reject observable restrictions; the protected host deployment
/// must additionally supply the original host PID namespace. A proc mount
/// newly made inside a private PID namespace cannot certify the outer host.
pub(super) fn require_host_uid_unheld(uid: u32) -> Result<(), RuntimeError> {
    let mut budget = Budget {
        deadline: Instant::now() + Duration::from_secs(2),
        entries: 0,
        limit: 262_144,
    };
    unrestricted_proc_mount(&read_metadata(
        Path::new("/proc/self/mountinfo"),
        MAX_MOUNT_BYTES,
    )?)?;
    single_namespace_pid(
        &read_metadata(Path::new("/proc/self/status"), MAX_STATUS_BYTES)?,
        std::process::id(),
    )?;
    single_namespace_pid(
        &read_metadata(Path::new("/proc/1/status"), MAX_STATUS_BYTES)?,
        1,
    )?;
    let own = fs::metadata("/proc/self/ns/pid")?;
    let initial = fs::metadata("/proc/1/ns/pid")?;
    if own.dev() != initial.dev() || own.ino() != initial.ino() {
        return Err(error("proc PID namespace differs from current process"));
    }
    budget.check()?;
    require_uid_unheld(Path::new("/proc"), uid, &mut budget)
}

#[cfg(test)]
#[path = "uid_retirement_tests.rs"]
mod tests;
