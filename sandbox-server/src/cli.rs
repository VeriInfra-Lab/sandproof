//! One production role; no legacy public server alias or implicit mode.
use std::{ffi::OsString, path::PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Command {
    Initialize(PathBuf),
    Serve(PathBuf),
    Audit(PathBuf),
    ObserveCenter(PathBuf),
    Relay,
    ContractVersions,
}

pub(crate) fn parse(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<Command, &'static str> {
    let mut arguments = arguments.into_iter();
    let command = arguments.next().ok_or("a host command is required")?;
    let result = match command.to_str() {
        Some("relay") => Command::Relay,
        Some("contract-versions") => Command::ContractVersions,
        Some("initialize" | "serve" | "audit" | "observe-center") => {
            if arguments.next().as_deref() != Some(std::ffi::OsStr::new("--config")) {
                return Err("host command requires --config /absolute/protected/config.json");
            }
            let path = PathBuf::from(arguments.next().ok_or("configuration path missing")?);
            if !path.is_absolute() {
                return Err("configuration path must be absolute");
            }
            match command.to_str() {
                Some("initialize") => Command::Initialize(path),
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
        for command in ["initialize", "serve", "audit", "observe-center"] {
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
}
