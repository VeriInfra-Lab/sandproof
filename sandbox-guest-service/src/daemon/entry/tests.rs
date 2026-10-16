use super::*;
use std::io::{Seek, SeekFrom, Write};

#[test]
fn only_explicit_actions_and_one_absolute_config_are_accepted() {
    for (word, action) in [
        ("check", Action::Check),
        ("initialize", Action::Initialize),
        ("serve", Action::Serve),
    ] {
        assert_eq!(
            arguments(
                [word, "--config", "/trusted/config.json"]
                    .into_iter()
                    .map(OsString::from)
            ),
            Ok((action, PathBuf::from("/trusted/config.json")))
        );
    }
    for args in [
        vec![],
        vec!["start"],
        vec!["serve"],
        vec!["serve", "config", "/cfg"],
        vec!["serve", "--config", "relative"],
        vec!["serve", "--config", "/cfg", "extra"],
        vec!["--config", "/cfg", "serve"],
    ] {
        assert_eq!(
            arguments(args.into_iter().map(OsString::from)),
            Err(Error::Arguments)
        );
    }
}

#[test]
fn bounded_same_descriptor_read_rejects_large_files_without_parsing() {
    let mut file = tempfile::tempfile().unwrap();
    file.write_all(b"configuration").unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    assert_eq!(read(file).unwrap(), b"configuration");
    let file = tempfile::tempfile().unwrap();
    file.set_len(CONFIG_BYTES as u64 + 1).unwrap();
    assert_eq!(read(file), Err(Error::Input));
}
