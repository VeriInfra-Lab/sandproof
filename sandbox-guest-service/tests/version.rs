#![cfg(target_os = "linux")]
use std::{process::Stdio, time::Duration};

#[tokio::test]
async fn version_is_available_without_configuration_and_has_no_startup_side_effects() {
    let directory = tempfile::tempdir().unwrap();
    for args in [
        vec!["--version"],
        vec!["--version", "--config", "/absent"],
        vec!["--version", "extra"],
    ] {
        let child = tokio::process::Command::new(env!("CARGO_BIN_EXE_sandproof-guest-service"))
            .args(&args)
            .current_dir(directory.path())
            .env_clear()
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let output = tokio::time::timeout(Duration::from_secs(5), child.wait_with_output())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(output.status.success(), args.len() == 1);
        if args.len() == 1 {
            let info: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(info, sandproof_guest_service::build_info::service());
            assert!(output.stderr.is_empty());
        } else {
            assert!(output.stdout.is_empty());
            assert!(
                String::from_utf8(output.stderr)
                    .unwrap()
                    .contains("Arguments")
            );
        }
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}
