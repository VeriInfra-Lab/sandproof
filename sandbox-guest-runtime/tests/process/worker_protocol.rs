use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixListener,
};

#[tokio::test]
async fn worker_version_needs_no_gate_and_refuses_extra_arguments() {
    for args in [vec!["--version"], vec!["--version", "extra"]] {
        let child = tokio::process::Command::new(WORKER)
            .args(&args)
            .kill_on_drop(true)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let output = tokio::time::timeout(Duration::from_secs(5), child.wait_with_output())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(output.status.success(), args.len() == 1);
        assert!(output.stderr.is_empty());
        if args.len() == 1 {
            let info: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(info, sandproof_guest_runtime::build_info::worker());
        } else {
            assert!(output.stdout.is_empty());
        }
    }
}

#[tokio::test]
async fn malformed_release_is_private_protocol_fault_never_native_exec_failure() {
    for release in [&[2][..], &[1, 9], &[]] {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("gate");
        let listener = UnixListener::bind(&socket).unwrap();
        let mut child = tokio::process::Command::new(WORKER)
            .args(["--gate", socket.to_str().unwrap(), "--limit", "16384"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            let (mut gate, _) = listener.accept().await.unwrap();
            let mut marker = [0; 4];
            gate.read_exact(&mut marker).await.unwrap();
            assert_eq!(&marker, b"SG11");
            let payload =
                serde_json::to_vec(&shell(directory.path(), "printf forbidden > effect", false))
                    .unwrap();
            gate.write_all(&u32::try_from(payload.len()).unwrap().to_be_bytes())
                .await
                .unwrap();
            gate.write_all(&payload).await.unwrap();
            gate.read_exact(&mut marker).await.unwrap();
            assert_eq!(&marker, b"SG12");
            gate.write_all(release).await.unwrap();
            gate.shutdown().await.unwrap();
            gate.read_exact(&mut marker).await.unwrap();
            assert_eq!(&marker, b"SGF1");
            let mut code = [0; 4];
            gate.read_exact(&mut code).await.unwrap();
            assert_eq!(i32::from_be_bytes(code), 22);
            assert_eq!(gate.read(&mut marker).await.unwrap(), 0);
            assert!(!child.wait().await.unwrap().success());
        })
        .await
        .unwrap();
        assert!(!directory.path().join("effect").exists());
        drop(listener);
        std::fs::remove_file(socket).unwrap();
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}
