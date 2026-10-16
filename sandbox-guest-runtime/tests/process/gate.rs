use super::*;

#[tokio::test]
async fn connected_gate_survives_path_retirement_and_failed_directory_removal_keeps_fallback() {
    for obstruct in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let mut process = prepare(
            directory.path(),
            &shell(directory.path(), "printf ran >> effect", false),
            limits(),
        )
        .await;
        let gate = std::fs::read_dir(directory.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert!(gate.join("launch").exists());
        if obstruct {
            // Force real ENOTEMPTY for rmdir, using only this fixture's file.
            // The original TempDir must still own its later cleanup.
            std::fs::write(gate.join("held"), b"fixture").unwrap();
        }
        assert_eq!(
            process.release().await.unwrap(),
            LaunchObservation::NoExecErrorObserved
        );
        assert!(!gate.join("launch").exists());
        assert_eq!(gate.exists(), obstruct);
        if !obstruct {
            // A different directory may later occupy the retired pathname.
            // Neither another retirement attempt nor old-owner Drop owns it.
            std::fs::create_dir(&gate).unwrap();
            std::fs::write(gate.join("launch"), b"replacement").unwrap();
            process.retire_gate_path();
            assert_eq!(std::fs::read(gate.join("launch")).unwrap(), b"replacement");
        }
        assert!(process.release().await.is_err());
        assert_eq!(collect(&mut process).await, (vec![], vec![], 0));
        assert_eq!(
            std::fs::read(directory.path().join("effect")).unwrap(),
            b"ran"
        );
        if obstruct {
            assert_eq!(std::fs::read(gate.join("held")).unwrap(), b"fixture");
        }
        drop(process);
        if obstruct {
            assert!(!gate.exists());
        } else {
            assert_eq!(std::fs::read(gate.join("launch")).unwrap(), b"replacement");
        }
    }
}
