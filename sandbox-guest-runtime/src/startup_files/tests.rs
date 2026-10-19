use super::*;
use std::{
    fs,
    io::Read,
    os::unix::fs::{PermissionsExt, symlink},
};

struct Fixture(tempfile::TempDir);
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
        Self(root)
    }
    fn mode(&self, path: &str, mode: u32) {
        fs::set_permissions(self.0.path().join(path), fs::Permissions::from_mode(mode)).unwrap();
    }
    fn file(&self, path: &str, mode: u32) {
        fs::write(self.0.path().join(path), b"original").unwrap();
        self.mode(path, mode);
    }
    fn directory(&self, path: &str, mode: u32) {
        fs::create_dir(self.0.path().join(path)).unwrap();
        self.mode(path, mode);
    }
    fn open(&self, path: &str, kind: Kind) -> Result<File, Error> {
        // Exercise the production descriptor walker below a private owned
        // anchor. Ambient /tmp and its writable parents are intentionally not
        // asserted to be trusted installation ancestry.
        let path = Path::new(path);
        let mut parts = path.components().peekable();
        assert_eq!(parts.next(), Some(Component::RootDir));
        walk(
            File::open(self.0.path()).unwrap(),
            parts,
            current_owner(),
            kind,
        )
    }
}

#[test]
fn private_inputs_and_executable_are_read_from_original_descriptors() {
    let f = Fixture::new();
    f.directory("install", 0o755);
    f.directory("install/state", 0o700);
    f.file("install/config", 0o600);
    f.file("install/worker", 0o755);
    assert!(f.open("/install/state", Kind::Directory).is_ok());
    assert!(f.open("/install/worker", Kind::Worker).is_ok());
    let mut config = f.open("/install/config", Kind::Configuration).unwrap();
    fs::rename(f.0.path().join("install/config"), f.0.path().join("old")).unwrap();
    fs::write(f.0.path().join("install/config"), b"replacement").unwrap();
    let mut contents = String::new();
    config.read_to_string(&mut contents).unwrap();
    assert_eq!(contents, "original");
}

#[test]
#[ignore = "explicit root-owned installation; see docs/testing-plan.md"]
fn root_owned_installation_is_opened_from_os_root() {
    let path = std::env::var_os("SANDPROOF_GUEST_ROOT_WORKER")
        .expect("set SANDPROOF_GUEST_ROOT_WORKER to a protected root-owned executable");
    let path = Path::new(&path);
    // Exercise the actual OS root and production opener. No process executes;
    // this requires file ownership, not a privileged test process.
    let executable = worker(path, 0, 4096).expect("root installation must satisfy startup checks");
    assert_eq!(executable.metadata().unwrap().uid(), 0);
    assert_eq!(worker(path, 1, 4096).err(), Some(Error::Untrusted));
}

#[test]
fn unsafe_ancestry_links_modes_and_types_are_refused() {
    let f = Fixture::new();
    f.directory("install", 0o755);
    f.file("install/config", 0o600);
    f.file("install/worker", 0o755);
    f.directory("install/state", 0o700);
    for mode in [0o777, 0o775, 0o757] {
        f.mode("install", mode);
        assert_eq!(
            f.open("/install/config", Kind::Configuration).err(),
            Some(Error::Untrusted)
        );
    }
    f.mode("install", 0o755);
    f.mode("install/config", 0o640);
    assert_eq!(
        f.open("/install/config", Kind::Configuration).err(),
        Some(Error::Untrusted)
    );
    f.mode("install/config", 0o600);
    f.mode("install/state", 0o750);
    assert_eq!(
        f.open("/install/state", Kind::Directory).err(),
        Some(Error::Untrusted)
    );
    for mode in [0o644, 0o775, 0o757] {
        f.mode("install/worker", mode);
        assert_eq!(
            f.open("/install/worker", Kind::Worker).err(),
            Some(Error::Untrusted)
        );
    }
    f.mode("install/worker", 0o755);
    for (path, kind) in [
        ("install/config", Kind::Configuration),
        ("install/worker", Kind::Worker),
    ] {
        fs::hard_link(f.0.path().join(path), f.0.path().join("alias")).unwrap();
        assert_eq!(
            f.open(&format!("/{path}"), kind).err(),
            Some(Error::Untrusted)
        );
        fs::remove_file(f.0.path().join("alias")).unwrap();
    }
    symlink("install", f.0.path().join("link")).unwrap();
    assert!(f.open("/link/config", Kind::Configuration).is_err());
    symlink("install/config", f.0.path().join("link-file")).unwrap();
    assert!(f.open("/link-file", Kind::Configuration).is_err());
    assert!(f.open("/install/config", Kind::Directory).is_err());
    assert_eq!(
        f.open("/install", Kind::Configuration).err(),
        Some(Error::Untrusted)
    );
    let mut parts = Path::new("/install/config").components().peekable();
    parts.next();
    assert_eq!(
        walk(
            File::open(f.0.path()).unwrap(),
            parts,
            current_owner() + 1,
            Kind::Configuration
        )
        .err(),
        Some(Error::Untrusted)
    );
}

#[test]
fn fifo_is_opened_nonblocking_then_refused_before_reading() {
    let f = Fixture::new();
    assert!(
        std::process::Command::new("mkfifo")
            .arg(f.0.path().join("fifo"))
            .status()
            .unwrap()
            .success()
    );
    f.mode("fifo", 0o600);
    assert_eq!(
        f.open("/fifo", Kind::Configuration).err(),
        Some(Error::Untrusted)
    );
}

#[test]
fn invalid_paths_and_budgets_are_refused_without_creation() {
    for path in ["", "/", "relative", "/missing/../path", "/nul\0path"] {
        assert_eq!(
            configuration(Path::new(path), current_owner(), 4096).err(),
            Some(Error::InvalidPath)
        );
    }
    for maximum in [0, 3, usize::MAX] {
        assert_eq!(
            configuration(Path::new("/some/path"), current_owner(), maximum).err(),
            Some(Error::InvalidPath)
        );
    }
    assert_eq!(
        configuration(Path::new("/some/path"), u32::MAX, 4096).err(),
        Some(Error::InvalidPath)
    );
}
