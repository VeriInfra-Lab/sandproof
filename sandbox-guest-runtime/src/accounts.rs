//! Bounded local passwd/group lookup, matching the fixed envd's CGO-disabled
//! build. No NSS, shell, network lookup or daemon-environment fallback. Blocking
//! filesystem work belongs in an owned, separately bounded blocking operation.
use std::fs::OpenOptions;
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use crate::Credentials;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountError {
    InvalidArgument,
    NotFound,
    ResourceExhausted,
    Unavailable,
}

#[derive(Clone, Copy, Debug)]
pub struct AccountLimits {
    /// Per source file; not a bound on filesystem/kernel allocations or latency.
    pub file_bytes: usize,
    pub groups: usize,
}

impl AccountLimits {
    /// # Errors
    /// Refuses zero/overflowing byte limits and unrepresentable group counts.
    pub fn validate(self) -> Result<Self, AccountError> {
        if self.file_bytes == 0
            || self.file_bytes >= isize::MAX as usize
            || self.groups < 2
            || self.groups > 65_536
        {
            return Err(AccountError::InvalidArgument);
        }
        Ok(self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub name: String,
    pub home: String,
    pub credentials: Credentials,
}

/// Resolve one named local account, never an implicit numeric uid selector.
/// # Errors
/// Missing/malformed accounts, unreadable passwd and exhausted budgets fail
/// closed. Unreadable group files retain only the resolved primary gid, as in
/// the fixed endpoint; they never inherit the daemon's supplementary groups.
pub fn lookup_local_user(name: &str, limits: AccountLimits) -> Result<Account, AccountError> {
    lookup_from_paths(
        name,
        Path::new("/etc/passwd"),
        Path::new("/etc/group"),
        limits,
    )
}

fn lookup_from_paths(
    name: &str,
    passwd: &Path,
    group: &Path,
    limits: AccountLimits,
) -> Result<Account, AccountError> {
    let limits = limits.validate()?;
    let passwd = read_regular(passwd, limits.file_bytes)?;
    let mut account = find_account(name, &passwd, limits)?;
    match read_regular(group, limits.file_bytes) {
        Ok(group) => add_groups(&mut account, &group, limits)?,
        Err(AccountError::Unavailable) => {}
        Err(error) => return Err(error),
    }
    Ok(account)
}

fn read_regular(path: &Path, limit: usize) -> Result<Vec<u8>, AccountError> {
    // O_NONBLOCK prevents a redirected FIFO open from occupying a worker
    // indefinitely. A regular file is required after opening that exact inode;
    // symlinks to regular files retain normal guest path semantics.
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(rustix::fs::OFlags::NONBLOCK.bits().cast_signed())
        .open(path)
        .map_err(|_| AccountError::Unavailable)?;
    let metadata = file.metadata().map_err(|_| AccountError::Unavailable)?;
    if !metadata.is_file() {
        return Err(AccountError::Unavailable);
    }
    if metadata.len() > limit as u64 {
        return Err(AccountError::ResourceExhausted);
    }
    let mut bytes = Vec::new();
    file.take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| AccountError::Unavailable)?;
    if bytes.len() > limit {
        return Err(AccountError::ResourceExhausted);
    }
    Ok(bytes)
}

/// The same parser used for real files; byte slices allow shared deterministic
/// account fixtures without editing host /etc or maintaining another resolver.
/// # Errors
/// Applies the same account and payload budgets as `lookup_local_user`.
pub fn account_from_files(
    name: &str,
    passwd: &[u8],
    group: &[u8],
    limits: AccountLimits,
) -> Result<Account, AccountError> {
    let mut account = find_account(name, passwd, limits)?;
    add_groups(&mut account, group, limits)?;
    Ok(account)
}

fn trim_line(line: &[u8]) -> &[u8] {
    std::str::from_utf8(line).map_or_else(|_| line.trim_ascii(), |text| text.trim().as_bytes())
}

fn find_account(name: &str, passwd: &[u8], limits: AccountLimits) -> Result<Account, AccountError> {
    let limits = limits.validate()?;
    if name.is_empty() || name.contains(['\0', '\n', '\r', ':']) {
        return Err(AccountError::NotFound);
    }
    if passwd.len() > limits.file_bytes {
        return Err(AccountError::ResourceExhausted);
    }
    let mut account = None;
    for line in passwd.split(|byte| *byte == b'\n').map(trim_line) {
        if line.is_empty() || matches!(line[0], b'#' | b'+' | b'-') {
            continue;
        }
        let fields: Vec<_> = line.splitn(7, |byte| *byte == b':').collect();
        if fields.len() < 7 || fields[0] != name.as_bytes() {
            continue;
        }
        // Go's user.Lookup skips non-integer records. Once a record is found,
        // envd's uint32 conversion fails instead of trying a later duplicate.
        let number = |value| {
            std::str::from_utf8(value)
                .ok()
                .and_then(|s| s.parse::<i64>().ok())
        };
        if number(fields[2]).is_none() || number(fields[3]).is_none() {
            continue;
        }
        let uid = identity(fields[2]).ok_or(AccountError::Unavailable)?;
        let gid = identity(fields[3]).ok_or(AccountError::Unavailable)?;
        let home = std::str::from_utf8(fields[5]).map_err(|_| AccountError::Unavailable)?;
        if home.contains('\0') {
            return Err(AccountError::Unavailable);
        }
        account = Some(Account {
            name: name.into(),
            home: home.into(),
            // Fixed envd starts with primary gid, then appends GroupIds(),
            // which also starts with primary gid. Preserve duplicate entries.
            credentials: Credentials {
                uid,
                gid,
                groups: vec![gid],
            },
        });
        break;
    }
    account.ok_or(AccountError::NotFound)
}

fn add_groups(
    account: &mut Account,
    group: &[u8],
    limits: AccountLimits,
) -> Result<(), AccountError> {
    if group.len() > limits.file_bytes {
        return Err(AccountError::ResourceExhausted);
    }
    // The fixed pure-Go GroupIds scanner has a 64 KiB token buffer. A scan
    // error discards the entire supplementary result, retaining primary only.
    if group
        .split(|byte| *byte == b'\n')
        .any(|line| line.len() >= 65_536)
    {
        return Ok(());
    }
    account.credentials.groups.push(account.credentials.gid);
    for line in group.split(|byte| *byte == b'\n').map(trim_line) {
        if line.is_empty() || matches!(line[0], b'#' | b'+' | b'-') {
            continue;
        }
        let fields: Vec<_> = line.splitn(5, |byte| *byte == b':').collect();
        if fields.len() != 4
            || fields[0].is_empty()
            || !fields[3]
                .split(|byte| *byte == b',')
                .any(|member| member == account.name.as_bytes())
        {
            continue;
        }
        let Some(gid) = unsigned(fields[2]) else {
            continue;
        };
        // ParseUint accepts this value in the fixed handler, but the kernel
        // cannot install it. Do not turn that failure into a successful launch
        // with a silently shortened group list.
        if gid == u32::MAX {
            return Err(AccountError::Unavailable);
        }
        if gid == account.credentials.gid {
            continue;
        }
        if account.credentials.groups.len() == limits.groups {
            return Err(AccountError::ResourceExhausted);
        }
        account.credentials.groups.push(gid);
    }
    Ok(())
}

fn identity(value: &[u8]) -> Option<u32> {
    unsigned(value).filter(|id| *id != u32::MAX)
}

fn unsigned(value: &[u8]) -> Option<u32> {
    if value.is_empty() || !value.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(value).ok()?.parse().ok()
}

/// Display ownership as fixed pure-Go LookupId/LookupGroupId do. A missing or
/// unreadable source falls back to numeric text; budget exhaustion is explicit.
/// These names never authorize I/O or select execution credentials.
/// # Errors
/// Invalid limits, exhausted source budget or unrepresentable matched names.
pub fn lookup_local_ownership(
    uid: u32,
    gid: u32,
    file_bytes: usize,
) -> Result<(String, String), AccountError> {
    if file_bytes == 0 || file_bytes >= isize::MAX as usize {
        return Err(AccountError::InvalidArgument);
    }
    let source = |path: &str| match read_regular(Path::new(path), file_bytes) {
        Ok(bytes) => Ok(bytes),
        Err(AccountError::Unavailable) => Ok(Vec::new()),
        Err(error) => Err(error),
    };
    ownership_from_files(
        uid,
        gid,
        &source("/etc/passwd")?,
        &source("/etc/group")?,
        file_bytes,
    )
}

/// Same display parser for deterministic fixtures, without editing host /etc.
/// # Errors
/// Source budget exhaustion or a matched name that is not valid UTF-8.
pub fn ownership_from_files(
    uid: u32,
    gid: u32,
    passwd: &[u8],
    group: &[u8],
    file_bytes: usize,
) -> Result<(String, String), AccountError> {
    if file_bytes == 0 || file_bytes >= isize::MAX as usize {
        return Err(AccountError::InvalidArgument);
    }
    if passwd.len() > file_bytes || group.len() > file_bytes {
        return Err(AccountError::ResourceExhausted);
    }
    let find = |id: u32, bytes: &[u8], count: usize| -> Result<String, AccountError> {
        let id = id.to_string();
        for line in bytes.split(|b| *b == b'\n').map(trim_line) {
            let fields: Vec<_> = line.splitn(count, |b| *b == b':').collect();
            if fields.len() != count
                || fields[0].is_empty()
                || matches!(fields[0][0], b'#' | b'+' | b'-')
                || fields[2] != id.as_bytes()
            {
                continue;
            }
            let numeric = |v: &[u8]| {
                std::str::from_utf8(v)
                    .ok()
                    .and_then(|s| s.parse::<i64>().ok())
                    .is_some()
            };
            if !numeric(fields[2]) || (count == 7 && !numeric(fields[3])) {
                continue;
            }
            return std::str::from_utf8(fields[0])
                .map(str::to_owned)
                .map_err(|_| AccountError::Unavailable);
        }
        Ok(id)
    };
    Ok((find(uid, passwd, 7)?, find(gid, group, 4)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> AccountLimits {
        AccountLimits {
            file_bytes: 4096,
            groups: 8,
        }
    }

    #[test]
    fn named_accounts_and_exact_memberships_preserve_numeric_identity() {
        let passwd = b"# ignored\n+u:x:0:0::/:/bin/sh\nu:x:notnum:4::/bad:/bin/sh\nu:x:1001:1002::/home/u:/bin/sh\nu:x:0:0::/bad:/bin/sh\n";
        let group = b"primary:x:1002:u\na:x:9:u,other\nb:x:10:prefixu\nc:x:11:other,u\nd:x:12:other,u,last\nbad:x:no:u\nnis:x:13:+u\n";
        let user = account_from_files("u", passwd, group, limits()).unwrap();
        assert_eq!(user.home, "/home/u");
        assert_eq!(
            user.credentials,
            Credentials {
                uid: 1001,
                gid: 1002,
                groups: vec![1002, 1002, 9, 11, 12]
            }
        );
        assert_eq!(
            account_from_files("1001", passwd, group, limits()),
            Err(AccountError::NotFound)
        );
    }

    #[test]
    fn invalid_identity_never_falls_back_to_a_later_root_record() {
        for id in ["-1", "+1", "4294967295", "4294967296"] {
            let passwd = format!("u:x:{id}:1::/home/u:/bin/sh\nu:x:0:0::/:/bin/sh\n");
            assert_eq!(
                account_from_files("u", passwd.as_bytes(), b"", limits()),
                Err(AccountError::Unavailable)
            );
        }
        assert_eq!(
            account_from_files("u", b"u:x:1:1::/home/u\n", b"", limits()),
            Err(AccountError::NotFound)
        );
    }

    #[test]
    fn account_and_group_budgets_reject_without_truncating_membership() {
        let passwd = b"u:x:1:2::/:/bin/sh\n";
        let limit = AccountLimits {
            file_bytes: passwd.len(),
            groups: 2,
        };
        assert!(account_from_files("u", passwd, b"", limit).is_ok());
        assert_eq!(
            account_from_files("u", passwd, b"x:x:3:u\n", limit),
            Err(AccountError::ResourceExhausted)
        );
        assert_eq!(
            account_from_files(
                "u",
                passwd,
                b"",
                AccountLimits {
                    file_bytes: passwd.len() - 1,
                    ..limit
                }
            ),
            Err(AccountError::ResourceExhausted)
        );
        for limit in [
            AccountLimits {
                file_bytes: 0,
                groups: 2,
            },
            AccountLimits {
                file_bytes: 1,
                groups: 1,
            },
            AccountLimits {
                file_bytes: 1,
                groups: 65_537,
            },
        ] {
            assert_eq!(limit.validate().unwrap_err(), AccountError::InvalidArgument);
        }
    }

    #[test]
    fn actual_files_require_regular_inodes_and_enforce_growth_budget() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("passwd");
        std::fs::write(&file, b"1234").unwrap();
        assert_eq!(read_regular(&file, 4).unwrap(), b"1234");
        assert_eq!(read_regular(&file, 3), Err(AccountError::ResourceExhausted));
        assert_eq!(
            read_regular(directory.path(), 4096),
            Err(AccountError::Unavailable)
        );
        let link = directory.path().join("link");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert_eq!(read_regular(&link, 4).unwrap(), b"1234");
        let fifo = directory.path().join("fifo");
        rustix::fs::mkfifoat(
            rustix::fs::CWD,
            &fifo,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
        )
        .unwrap();
        assert_eq!(read_regular(&fifo, 4), Err(AccountError::Unavailable));
    }

    #[test]
    fn sentinel_supplementary_gid_is_rejected_instead_of_omitted() {
        let passwd = b"u:x:1:2::/:/bin/sh\n";
        assert_eq!(
            account_from_files("u", passwd, b"group:x:4294967295:u\n", limits()),
            Err(AccountError::Unavailable)
        );
        assert_eq!(
            account_from_files("u", passwd, b"group:x:4294967296:u\n", limits())
                .unwrap()
                .credentials
                .groups,
            vec![2, 2]
        );
    }

    #[test]
    fn group_source_failure_keeps_only_primary_never_daemon_groups() {
        let directory = tempfile::tempdir().unwrap();
        let passwd = directory.path().join("passwd");
        let group = directory.path().join("group");
        std::fs::write(&passwd, b"u:x:1001:1002::/home/u:/bin/sh\n").unwrap();
        let user = lookup_from_paths("u", &passwd, &group, limits()).unwrap();
        assert_eq!(user.credentials.groups, vec![1002]);
        std::fs::write(&group, b"other:x:9:u\n").unwrap();
        assert_eq!(
            lookup_from_paths("u", &passwd, &group, limits())
                .unwrap()
                .credentials
                .groups,
            vec![1002, 1002, 9]
        );
        let large = vec![b'x'; 65_536];
        let user = account_from_files(
            "u",
            &std::fs::read(&passwd).unwrap(),
            &large,
            AccountLimits {
                file_bytes: 65_536,
                groups: 8,
            },
        )
        .unwrap();
        assert_eq!(user.credentials.groups, vec![1002]);
        assert_eq!(
            account_from_files(
                "u",
                b"bad:x:9:9:\xff:/:sh\nu:x:1:2::/:sh\n",
                b"g\xff:x:3:u\n",
                limits()
            )
            .unwrap()
            .credentials
            .groups,
            vec![2, 2, 3]
        );
    }
}
