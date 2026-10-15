//! Fixed runtime f455fd58 handler.New shell/priority wrapper. User argv is
//! passed as positional arguments, never interpolated into the shell source.
use std::io;

fn script(
    ionice: Option<&str>,
    nice: Option<&str>,
    delta: i32,
    limit: usize,
) -> io::Result<String> {
    let mut output = String::new();
    let mut put = |text: &str| {
        if text.len() > limit.saturating_sub(output.len()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "launch wrapper budget",
            ));
        }
        output.push_str(text);
        Ok(())
    };
    put("echo 100 > /proc/$$/oom_score_adj && exec ")?;
    if let Some(path) = ionice {
        put(path)?;
        put(" -c 2 -n 4 ")?;
    }
    if let Some(path) = nice {
        put(path)?;
        put(" -n ")?;
        put(&delta.to_string())?;
        put(" ")?;
    }
    put("\"${@}\"")?;
    Ok(output)
}

// filepath.Join performs lexical Clean before the fixed Go LookPath Stat.
// Preserve relative leading '..'; a found relative helper is then rejected
// under the pinned default execerrdot policy rather than trying a later path.
fn candidate(directory: &str, name: &str) -> String {
    let rooted = directory.starts_with('/');
    let mut parts = Vec::new();
    for part in directory.split('/').chain([name]) {
        match part {
            "" | "." => (),
            ".." if parts.last().is_some_and(|last| *last != "..") => {
                parts.pop();
            }
            ".." if rooted => (),
            _ => parts.push(part),
        }
    }
    format!("{}{}", if rooted { "/" } else { "" }, parts.join("/"))
}

#[cfg(target_os = "linux")]
fn helper(path: &str, name: &str) -> Option<String> {
    use rustix::fs::{Access, AtFlags, CWD, accessat};
    use rustix::io::Errno;
    use std::os::unix::fs::PermissionsExt;
    if path.is_empty() {
        return None;
    }
    for directory in path.split(':') {
        let candidate = candidate(directory, name);
        let Ok(metadata) = std::fs::metadata(&candidate) else {
            continue;
        };
        if metadata.is_dir() {
            continue;
        }
        let executable = match accessat(CWD, &candidate, Access::EXEC_OK, AtFlags::EACCESS) {
            Ok(()) => true,
            Err(Errno::NOSYS | Errno::PERM) => metadata.permissions().mode() & 0o111 != 0,
            Err(_) => false,
        };
        if executable {
            return candidate.starts_with('/').then_some(candidate);
        }
    }
    None
}

#[cfg(target_os = "linux")]
pub(crate) fn prepare(spec: &mut crate::LaunchSpec, maximum: usize) -> io::Result<()> {
    let Some(path) = spec.envd_wrapper_path.take() else {
        return Ok(());
    };
    let nice = rustix::process::getpriority_process(None).unwrap_or(0);
    let delta = nice
        .checked_neg()
        .ok_or_else(|| io::Error::other("invalid nice value"))?;
    let shell = script(
        helper(&path, "ionice").as_deref(),
        helper(&path, "nice").as_deref(),
        delta,
        maximum,
    )?;
    let mut arguments = Vec::new();
    arguments.try_reserve(
        spec.args
            .len()
            .checked_add(4)
            .ok_or_else(|| io::Error::other("launch arguments"))?,
    )?;
    arguments.extend([
        "-c".into(),
        shell,
        "--".into(),
        std::mem::replace(&mut spec.program, "/bin/sh".into()),
    ]);
    arguments.append(&mut spec.args);
    spec.args = arguments;
    spec.validate()?;
    // Wrapped context gets the same encoded launch budget, before READY.
    drop(crate::process::encode(spec, maximum)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[path = "reference.rs"]
    mod reference;

    #[test]
    fn fixed_shell_prefix_keeps_optional_helpers_and_exact_budget() {
        for (io, nice, delta, prefix) in [
            (None, None, 0, ""),
            (Some("/bin/ionice"), None, 0, "/bin/ionice -c 2 -n 4 "),
            (None, Some("/bin/nice"), -19, "/bin/nice -n -19 "),
            (
                Some("/sbin/ionice"),
                Some("/bin/nice"),
                20,
                "/sbin/ionice -c 2 -n 4 /bin/nice -n 20 ",
            ),
        ] {
            let expected = format!("echo 100 > /proc/$$/oom_score_adj && exec {prefix}\"${{@}}\"");
            assert_eq!(script(io, nice, delta, expected.len()).unwrap(), expected);
            assert!(script(io, nice, delta, expected.len() - 1).is_err());
        }
    }

    #[test]
    fn helper_candidates_keep_pinned_lexical_and_relative_rules() {
        for (directory, expected) in [
            ("", "nice"),
            (".", "nice"),
            ("/", "/nice"),
            ("/bin/../sbin//", "/sbin/nice"),
            ("bin/../", "nice"),
            ("../bin/..", "../nice"),
            ("../../", "../../nice"),
            ("/../../bin", "/bin/nice"),
        ] {
            assert_eq!(candidate(directory, "nice"), expected);
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn helper_lookup_omits_missing_nonexecutable_directory_and_first_relative_hit() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let path = root.path().to_str().unwrap();
        assert_eq!(helper("", "nice"), None);
        assert_eq!(helper(path, "nice"), None);
        std::fs::create_dir(root.path().join("nice")).unwrap();
        assert_eq!(helper(path, "nice"), None);
        std::fs::remove_dir(root.path().join("nice")).unwrap();
        std::fs::write(root.path().join("nice"), "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(
            root.path().join("nice"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        assert_eq!(helper(path, "nice"), None);
        std::fs::set_permissions(
            root.path().join("nice"),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        assert_eq!(helper(path, "nice"), Some(format!("{path}/nice")));
        let relative = root.path().file_name().unwrap().to_str().unwrap();
        assert_eq!(helper(&format!("{relative}:{path}"), "nice"), None);
        assert_eq!(
            helper(&format!("{path}/missing:{path}"), "nice"),
            Some(format!("{path}/nice"))
        );
    }
}
