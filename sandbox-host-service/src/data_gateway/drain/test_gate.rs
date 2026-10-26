//! One Linux debug acceptance barrier after durable revocation and before the
//! SAME owned relay is killed/reaped. It grants no lifecycle or exit evidence.
use std::{
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Read, Write},
    os::unix::{
        fs::{MetadataExt, OpenOptionsExt},
        net::UnixListener,
    },
    path::Path,
    time::{Duration, Instant},
};

const MAGIC: &str = "sandproof-data-drain-v1";
const WAIT: Duration = Duration::from_secs(10);

fn invalid() -> io::Error {
    io::Error::other("debug data-drain barrier identity or protocol differs")
}

fn identity(meta: &Metadata) -> (u64, u64, u64, i64, i64) {
    (
        meta.dev(),
        meta.ino(),
        meta.len(),
        meta.mtime(),
        meta.mtime_nsec(),
    )
}

fn original(path: &Path, file: &File) -> io::Result<()> {
    if identity(&fs::symlink_metadata(path)?) != identity(&file.metadata()?) {
        return Err(invalid());
    }
    Ok(())
}

fn original_directory(path: &Path, held: &File, owner: u32) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    let original = held.metadata()?;
    if !meta.is_dir()
        || meta.uid() != owner
        || meta.mode() & 0o077 != 0
        || (meta.dev(), meta.ino()) != (original.dev(), original.ino())
    {
        return Err(invalid());
    }
    Ok(())
}

pub(super) fn wait(pid: u32) -> io::Result<()> {
    let Some(directory) = std::env::var_os("SANDPROOF_TEST_DATA_DRAIN_GATE") else {
        return Ok(());
    };
    let directory = Path::new(&directory);
    wait_selected(pid, directory, 0, WAIT)
}

fn wait_selected(pid: u32, directory: &Path, owner: u32, wait: Duration) -> io::Result<()> {
    if !directory.is_absolute() || fs::canonicalize(directory)? != directory {
        return Err(invalid());
    }
    let held = File::open(directory)?;
    let metadata = held.metadata()?;
    if !metadata.is_dir() || metadata.uid() != owner || metadata.mode() & 0o077 != 0 {
        return Err(invalid());
    }
    original_directory(directory, &held, owner)?;
    let arm = directory.join("selected.arm");
    let meta = match fs::symlink_metadata(&arm) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if !meta.is_file()
        || meta.uid() != owner
        || meta.mode() & 0o077 != 0
        || meta.nlink() != 1
        || meta.len() > 512
    {
        return Err(invalid());
    }
    let mut file = File::open(&arm)?;
    if identity(&meta) != identity(&file.metadata()?) {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file).take(513).read_to_end(&mut bytes)?;
    original(&arm, &file)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| invalid())?;
    let fields: Vec<_> = text.lines().collect();
    if fields.len() != 5
        || fields[0] != MAGIC
        || fields[1].len() != 64
        || !fields[1].bytes().all(|b| b.is_ascii_hexdigit())
        || !text.ends_with('\n')
    {
        return Err(invalid());
    }
    if fields[2] != pid.to_string() {
        return Ok(());
    }
    let process_text = fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let (_, values) = process_text.rsplit_once(')').ok_or_else(invalid)?;
    let start = values.split_whitespace().nth(19).ok_or_else(invalid)?;
    let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
    if fields[3] != start || fields[4] != boot.trim() {
        return Err(invalid());
    }
    let socket = directory.join(format!("{pid}.sock"));
    let listener = UnixListener::bind(&socket)?;
    let socket_identity = identity(&fs::symlink_metadata(&socket)?);
    let result = (|| {
        listener.set_nonblocking(true)?;
        let deadline = Instant::now() + wait;
        loop {
            original_directory(directory, &held, owner)?;
            original(&arm, &file)?;
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "debug data-drain barrier",
                ));
            }
            match listener.accept() {
                Ok((mut peer, _)) => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    peer.set_read_timeout(Some(remaining))?;
                    peer.set_write_timeout(Some(remaining))?;
                    peer.write_all(&bytes)?;
                    let mut release = [0];
                    peer.read_exact(&mut release)?;
                    return if release == *b"g" {
                        Ok(())
                    } else {
                        Err(invalid())
                    };
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => return Err(error),
            }
        }
    })();
    drop(listener);
    if fs::symlink_metadata(&socket).is_ok_and(|meta| identity(&meta) == socket_identity) {
        let _ = fs::remove_file(&socket);
    }
    // The caller ALWAYS continues through the original terminate_and_wait,
    // including timeout, disconnect, wrong identity and observer failure.
    result
}

pub(super) fn record_unload_busy<H: sandproof_controller_service::application::ControllerHost>(
    intent: &sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
    app: &sandproof_controller_service::application::ControllerApplication<H>,
) -> io::Result<()> {
    let Some(directory) = std::env::var_os("SANDPROOF_TEST_DATA_DRAIN_GATE") else {
        return Ok(());
    };
    let state = app
        .lifecycle_work_state_for_recorded(intent, &intent.request.binding.node)
        .map_err(|_| invalid())?;
    let Some(local) = state.local else {
        return Ok(());
    };
    let audit = app.internal_audit().map_err(|_| invalid())?;
    if !state.current
        || state.terminal
        || !audit.native.iter().any(|row| {
            row.sandbox == local.sandbox
                && row.phase == sandbox_controller_core::Phase::Pausing
                && !row.terminal
        })
    {
        return Ok(());
    }
    record_busy_at(Path::new(&directory), intent, 0)
}

fn record_busy_at(
    directory: &Path,
    intent: &sandproof_protocol::lifecycle::AcceptedLifecycleIntent,
    owner: u32,
) -> io::Result<()> {
    if !directory.is_absolute() || fs::canonicalize(directory)? != directory {
        return Err(invalid());
    }
    let held = File::open(directory)?;
    original_directory(directory, &held, owner)?;
    let arm = directory.join("selected.arm");
    let meta = match fs::symlink_metadata(&arm) {
        Ok(meta) => meta,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if !meta.is_file()
        || meta.uid() != owner
        || meta.mode() & 0o077 != 0
        || meta.nlink() != 1
        || meta.len() > 512
    {
        return Err(invalid());
    }
    let mut file = File::open(&arm)?;
    if identity(&meta) != identity(&file.metadata()?) {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file).take(513).read_to_end(&mut bytes)?;
    original(&arm, &file)?;
    let encoded =
        sandproof_controller_service::template_registry::evidence::encode_bounded(intent, 8192)
            .map_err(|_| invalid())?;
    bytes.extend(encoded);
    let path = directory.join("unload-busy");
    if fs::symlink_metadata(&path).is_ok() {
        return Ok(());
    }
    let stage = directory.join(".unload-busy.stage");
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&stage)
    {
        Ok(mut output) => {
            output.write_all(&bytes)?;
            output.sync_all()?;
            original_directory(directory, &held, owner)?;
            fs::rename(stage, path)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        os::unix::{fs::PermissionsExt, net::UnixStream},
        sync::mpsc,
    };

    fn connect_selected_socket(
        socket: &Path,
        deadline: Instant,
        mut pending: impl FnMut(io::ErrorKind),
    ) -> io::Result<UnixStream> {
        loop {
            match UnixStream::connect(socket) {
                Ok(peer) => return Ok(peer),
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                    ) && Instant::now() < deadline =>
                {
                    pending(error.kind());
                    std::thread::yield_now();
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn private_directory() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(directory.path().metadata().unwrap().mode() & 0o7777, 0o700);
        directory
    }

    fn arm(directory: &Path, start_override: Option<&str>) -> Vec<u8> {
        let pid = std::process::id();
        let process_text = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        let start = process_text
            .rsplit_once(')')
            .unwrap()
            .1
            .split_whitespace()
            .nth(19)
            .unwrap();
        let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap();
        let bytes = format!(
            "{MAGIC}\n{}\n{pid}\n{}\n{}\n",
            "a".repeat(64),
            start_override.unwrap_or(start),
            boot.trim()
        )
        .into_bytes();
        let path = directory.join("selected.arm");
        fs::write(&path, &bytes).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        bytes
    }

    #[test]
    fn exact_selected_process_waits_for_release_without_claiming_exit() {
        let directory = private_directory();
        let owner = directory.path().metadata().unwrap().uid();
        let bytes = arm(directory.path(), None);
        let path = directory.path().to_owned();
        let worker = std::thread::spawn(move || {
            wait_selected(std::process::id(), &path, owner, Duration::from_secs(2))
        });
        let socket = directory
            .path()
            .join(format!("{}.sock", std::process::id()));
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut peer = connect_selected_socket(&socket, deadline, |_| {})
            .expect("original selected socket");
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut echo = vec![0; bytes.len()];
        peer.read_exact(&mut echo).unwrap();
        assert_eq!(echo, bytes);
        assert!(!worker.is_finished());
        peer.write_all(b"g").unwrap();
        worker.join().unwrap().unwrap();
        assert!(!socket.exists());
        assert!(Path::new(&format!("/proc/{}/stat", std::process::id())).exists());
    }

    #[tokio::test]
    async fn selected_socket_fixture_waits_for_listen_after_path_is_bound() {
        let directory = private_directory();
        let path = directory.path().join("bound-before-listen.sock");
        // These are the real bind/listen phases, using the existing Tokio
        // dependency. The path exists while no listener accepts connections.
        let bound = tokio::net::UnixSocket::new_stream().unwrap();
        bound.bind(&path).unwrap();
        assert!(path.exists());
        assert_eq!(
            UnixStream::connect(&path).unwrap_err().kind(),
            io::ErrorKind::ConnectionRefused
        );
        let (pending_tx, pending_rx) = mpsc::sync_channel(1);
        let (listening_tx, listening_rx) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            let mut first = Some(pending_tx);
            connect_selected_socket(&path, Instant::now() + Duration::from_secs(2), |kind| {
                if let Some(sender) = first.take() {
                    sender.send(kind).unwrap();
                    listening_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                }
            })
        });
        assert_eq!(
            pending_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            io::ErrorKind::ConnectionRefused
        );
        let listener = bound.listen(1).unwrap();
        listening_tx.send(()).unwrap();
        let connected = worker.join().unwrap().unwrap();
        let (accepted, _) = listener.accept().await.unwrap();
        drop((connected, accepted));
    }

    #[test]
    fn selected_socket_fixture_rejects_refusal_after_its_deadline() {
        let directory = private_directory();
        let path = directory.path().join("never-listening.sock");
        let bound = tokio::net::UnixSocket::new_stream().unwrap();
        bound.bind(&path).unwrap();
        let result = connect_selected_socket(&path, Instant::now(), |_| {
            panic!("expired fixture must not wait for listener readiness");
        });
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::ConnectionRefused);
    }

    #[test]
    fn timeout_stale_process_and_foreign_owner_never_claim_exit() {
        let directory = private_directory();
        let owner = directory.path().metadata().unwrap().uid();
        arm(directory.path(), None);
        let result = wait_selected(
            std::process::id(),
            directory.path(),
            owner,
            Duration::from_millis(1),
        );
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
        assert!(
            !directory
                .path()
                .join(format!("{}.sock", std::process::id()))
                .exists()
        );
        arm(directory.path(), Some("0"));
        assert!(wait_selected(std::process::id(), directory.path(), owner, WAIT).is_err());
        assert!(
            wait_selected(
                std::process::id(),
                directory.path(),
                owner.saturating_add(1),
                WAIT
            )
            .is_err()
        );
    }

    #[test]
    fn busy_progress_is_once_and_binds_the_complete_original_request_and_relay() {
        let directory = private_directory();
        let owner = directory.path().metadata().unwrap().uid();
        let bytes = arm(directory.path(), None);
        let mut intent = crate::data_gateway::port_tests::lifecycle::request();
        intent.accepted_version = 2;
        intent.request.operation = sandproof_protocol::lifecycle::LifecycleOperation::Unload {
            source_incarnation: 101.try_into().unwrap(),
            capture: 1.try_into().unwrap(),
        };
        record_busy_at(directory.path(), &intent, owner).unwrap();
        let receipt = fs::read(directory.path().join("unload-busy")).unwrap();
        assert!(receipt.starts_with(&bytes));
        let saved: sandproof_protocol::lifecycle::AcceptedLifecycleIntent =
            serde_json::from_slice(&receipt[bytes.len()..]).unwrap();
        assert_eq!(saved, intent);
        intent.accepted_version += 1;
        record_busy_at(directory.path(), &intent, owner).unwrap();
        assert_eq!(
            fs::read(directory.path().join("unload-busy")).unwrap(),
            receipt
        );
        assert!(!directory.path().join(".unload-busy.stage").exists());
    }
}
