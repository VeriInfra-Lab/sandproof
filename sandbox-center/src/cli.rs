use std::path::PathBuf;

const USAGE: &str =
    "usage: sandproof-center initialize|serve|audit --config ABSOLUTE_CONFIG, or contract-versions";

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Versions,
    Initialize(PathBuf),
    Serve(PathBuf),
    Audit(PathBuf),
}

impl Command {
    pub fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Self, &'static str> {
        let arguments: Vec<_> = arguments.into_iter().take(4).collect();
        if arguments.len() == 1 && arguments[0] == "contract-versions" {
            return Ok(Self::Versions);
        }
        if arguments.len() != 3 || arguments[1] != "--config" {
            return Err(USAGE);
        }
        let path = PathBuf::from(&arguments[2]);
        if !path.is_absolute() {
            return Err(USAGE);
        }
        match arguments[0].as_str() {
            "initialize" => Ok(Self::Initialize(path)),
            "serve" => Ok(Self::Serve(path)),
            "audit" => Ok(Self::Audit(path)),
            _ => Err(USAGE),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(arguments: &[&str]) -> Result<Command, &'static str> {
        Command::parse(arguments.iter().map(|s| (*s).to_owned()))
    }

    #[test]
    fn one_explicit_configuration_grammar_has_no_legacy_mode_or_implicit_initialize() {
        assert_eq!(parse(&["contract-versions"]), Ok(Command::Versions));
        assert_eq!(
            parse(&["serve", "--config", "/protected/center.json"]),
            Ok(Command::Serve("/protected/center.json".into()))
        );
        assert_eq!(
            parse(&["initialize", "--config", "/protected/center.json"]),
            Ok(Command::Initialize("/protected/center.json".into()))
        );
        assert_eq!(
            parse(&["audit", "--config", "/protected/center.json"]),
            Ok(Command::Audit("/protected/center.json".into()))
        );
        for invalid in [
            vec![],
            vec!["serve"],
            vec!["serve", "/protected/center.json"],
            vec!["serve", "--config", "relative.json"],
            vec!["serve", "--config", "/protected/center.json", "--legacy"],
            vec!["contract-versions", "--config", "/protected/center.json"],
            vec!["initialize-and-serve", "--config", "/protected/center.json"],
        ] {
            assert!(parse(&invalid).is_err());
        }
    }
}
