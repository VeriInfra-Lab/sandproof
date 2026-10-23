//! One production role; no legacy public server alias or implicit mode.
use std::{ffi::OsString, path::PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Command {
    Initialize(PathBuf),
    ProvisionBootstrap(PathBuf),
    Serve(PathBuf),
    Audit(PathBuf),
    ObserveCenter(PathBuf),
    Relay,
    ContractVersions,
    CaptureWorker(String),
}

pub(crate) fn parse(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<Command, &'static str> {
    let mut arguments = arguments.into_iter();
    let command = arguments.next().ok_or("a host command is required")?;
    let result = match command.to_str() {
        Some("relay") => Command::Relay,
        Some("contract-versions") => Command::ContractVersions,
        Some("capture-worker") => {
            let digest = arguments.next().ok_or("capture job digest missing")?;
            let digest = digest.into_string().map_err(|_| "capture job digest is not ASCII")?;
            if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
                return Err("capture job digest must contain 64 lowercase hex digits");
            }
            Command::CaptureWorker(digest)
        }
        Some("initialize" | "provision-bootstrap" | "serve" | "audit" | "observe-center") => {
            if arguments.next().as_deref() != Some(std::ffi::OsStr::new("--config")) {
                return Err("host command requires --config /absolute/protected/config.json");
            }
            let path = PathBuf::from(arguments.next().ok_or("configuration path missing")?);
            if !path.is_absolute() {
                return Err("configuration path must be absolute");
            }
            match command.to_str() {
                Some("initialize") => Command::Initialize(path),
                Some("provision-bootstrap") => Command::ProvisionBootstrap(path),
                Some("serve") => Command::Serve(path),
                Some("observe-center") => Command::ObserveCenter(path),
                _ => Command::Audit(path),
            }
        }
        _ => return Err("unsupported host command"),
    };
    if arguments.next().is_some() {
        return Err("unexpected host arguments");
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }
    #[test]
    fn only_explicit_protected_host_commands_are_accepted() {
        for command in ["initialize", "provision-bootstrap", "serve", "audit", "observe-center"] {
            assert!(parse(arguments(&[command, "--config", "/private/config"])).is_ok());
        }
        assert_eq!(parse(arguments(&["relay"])).unwrap(), Command::Relay);
        assert_eq!(
            parse(arguments(&["contract-versions"])).unwrap(),
            Command::ContractVersions
        );
        for values in [
            &[][..],
            &["/private/config"][..],
            &["serve", "/private/config"][..],
            &["serve", "--config", "relative"][..],
            &["relay", "extra"][..],
            &["contract-versions", "--config", "/private/config"][..],
            &["serve", "--config", "/private/config", "extra"][..],
        ] {
            assert!(parse(arguments(values)).is_err());
        }
    }

    #[test]
    fn capture_worker_accepts_one_exact_digest_and_no_product_configuration() {
        let digest = "a1".repeat(32);
        assert_eq!(parse(arguments(&["capture-worker", &digest])), Ok(Command::CaptureWorker(digest)));
        for values in [
            vec!["capture-worker"], vec!["capture-worker", "1234"],
            vec!["capture-worker", "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"],
            vec!["capture-worker", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaag"],
            vec!["capture-worker", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "extra"],
            vec!["capture-worker", "--config", "/private/config"],
        ] {
            assert!(parse(arguments(&values)).is_err());
        }
    }
}
