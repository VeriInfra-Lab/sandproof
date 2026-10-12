//! One owned Python child exercising the original relay against real Rust TLS.
use super::{Certificates, NodeIdentity, SocketAddr};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    net::TcpListener,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

pub(super) struct PythonRelay {
    child: Child,
    _files: tempfile::TempDir,
    pub address: SocketAddr,
}

impl PythonRelay {
    pub fn start(certs: &Certificates, backend: SocketAddr, node: &NodeIdentity) -> Self {
        let files = tempfile::tempdir().unwrap();
        // macOS /var is a symlink; the same original private_path contract
        // requires canonical file paths on both supported development OSes.
        let root = files.path().canonicalize().unwrap();
        let operator = root.join("operator");
        let control = root.join("control");
        for directory in [&operator, &control] {
            fs::create_dir(directory).unwrap();
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let receipt = material_files(certs, &operator);
        let port = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = port.local_addr().unwrap();
        drop(port);
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let child = Command::new("bash")
            .arg(repo.join("scripts/python-entry.sh"))
            .arg(repo.join("tests/system/_internal_relay_tls_peer.py"))
            .current_dir(repo)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let mut peer = Self {
            child,
            _files: files,
            address,
        };
        let value = serde_json::json!({
            "operator": operator, "control": control, "receipt": receipt,
            "listen_port": address.port(), "backend_port": backend.port(), "node": node,
        });
        let input = peer.child.stdin.as_mut().unwrap();
        serde_json::to_writer(&mut *input, &value).unwrap();
        input.write_all(b"\n").unwrap();
        input.flush().unwrap();
        let mut stdout = peer.child.stdout.take().unwrap();
        let (send, receive) = mpsc::channel();
        let reader = thread::spawn(move || {
            let mut ready = [0; 6];
            let result = stdout.read_exact(&mut ready).map(|()| ready);
            let _ = send.send(result);
        });
        let ready = receive
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap();
        reader.join().unwrap();
        assert_eq!(&ready, b"READY\n");
        peer
    }

    pub fn stop(&mut self) {
        self.child
            .stdin
            .take()
            .unwrap()
            .write_all(b"STOP\n")
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "owned Python relay failed");
                return;
            }
            assert!(
                Instant::now() < deadline,
                "owned Python relay shutdown expired"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for PythonRelay {
    fn drop(&mut self) {
        // Panic/cancellation still stops only our exact Child. No global PID
        // matching or deployed service/material cleanup is performed.
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn material_files(
    certs: &Certificates,
    operator: &Path,
) -> serde_json::Map<String, serde_json::Value> {
    let payloads = [
        ("ca.der", certs.ca.der().to_vec()),
        ("center.der", certs.center.certificate.der().to_vec()),
        ("center-key.der", certs.center.key.serialize_der()),
        ("host-runtime.der", certs.host.certificate.der().to_vec()),
        ("host-runtime-key.der", certs.host.key.serialize_der()),
    ];
    let mut receipt = serde_json::Map::new();
    for (name, data) in payloads {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(operator.join(name))
            .unwrap();
        file.write_all(&data).unwrap();
        file.sync_all().unwrap();
        receipt.insert(name.into(), format!("{:x}", Sha256::digest(&data)).into());
    }
    receipt
}
