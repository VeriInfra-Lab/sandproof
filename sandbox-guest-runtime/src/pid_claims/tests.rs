use super::*;
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    panic::{AssertUnwindSafe, catch_unwind},
};

struct Fixture {
    directory: tempfile::TempDir,
    owner: u32,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let owner = fs::metadata(directory.path()).unwrap().uid();
        Self { directory, owner }
    }
    fn fd(&self) -> File {
        File::open(self.directory.path()).unwrap()
    }
    fn create(&self, max: u32) -> Result<PidClaims, Error> {
        PidClaims::create(self.fd(), self.owner, binding(), max)
    }
    fn open(&self, max: u32) -> Result<PidClaims, Error> {
        PidClaims::open(self.fd(), self.owner, binding(), max)
    }
    fn bytes(&self) -> Vec<u8> {
        fs::read(self.directory.path().join(NAME)).unwrap()
    }
    fn replace_bytes(&self, bytes: &[u8]) {
        fs::write(self.directory.path().join(NAME), bytes).unwrap();
    }
}
fn binding() -> Binding {
    Binding::new([0x3a; 32]).unwrap()
}
fn identity(pid: u32) -> CommandIdentity {
    CommandIdentity {
        session: 7,
        command: u64::from(pid),
        pid,
        start_ticks: 42,
    }
}
fn error<T>(result: Result<T, Error>) -> Error {
    result.err().expect("unexpected success")
}

#[test]
fn claims_are_durable_across_reopen_and_never_return_after_lost_receipt_or_new_session() {
    let f = Fixture::new();
    let mut claims = f.create(31).unwrap();
    for pid in [1, 2, 7, 8, 16, 31] {
        let receipt = claims.claim(identity(pid)).unwrap();
        assert_eq!(receipt.identity(), identity(pid));
        assert!(receipt.belongs_to(binding()));
        assert!(!receipt.belongs_to(Binding::new([1; 32]).unwrap()));
        // Discarding a lost Start reply or never-executed launch still burns it.
    }
    let before = f.bytes();
    drop(claims);
    let mut reopened = f.open(31).unwrap();
    for pid in [1, 2, 7, 8, 16, 31] {
        let newer = CommandIdentity {
            session: 999,
            command: 1000,
            start_ticks: 99999,
            ..identity(pid)
        };
        assert_eq!(error(reopened.claim(newer)), Error::Reused);
    }
    assert_eq!(before, f.bytes());
    reopened.claim(identity(30)).unwrap();
    drop(reopened);
    assert_eq!(error(f.create(31)), Error::AlreadyExists);
    assert_eq!(
        error(f.open(31).unwrap().claim(identity(30))),
        Error::Reused
    );
}

#[test]
fn invalid_identity_and_exhaustion_cannot_grow_or_reset_the_finite_set() {
    assert!(matches!(Binding::new([0; 32]), Err(Error::Configuration)));
    for maximum in [0, MAX_PID + 1, u32::MAX] {
        let f = Fixture::new();
        assert_eq!(error(f.create(maximum)), Error::Configuration);
        assert!(!f.directory.path().join(NAME).exists());
    }
    let f = Fixture::new();
    let mut claims = f.create(9).unwrap();
    let before = f.bytes();
    for invalid in [
        identity(0),
        CommandIdentity {
            session: 0,
            ..identity(1)
        },
        CommandIdentity {
            command: 0,
            ..identity(1)
        },
    ] {
        assert_eq!(error(claims.claim(invalid)), Error::InvalidIdentity);
    }
    assert_eq!(error(claims.claim(identity(10))), Error::OutOfRange);
    assert_eq!(before, f.bytes());
    for pid in 1..=9 {
        claims.claim(identity(pid)).unwrap();
    }
    for pid in 1..=9 {
        assert_eq!(error(claims.claim(identity(pid))), Error::Reused);
    }
    assert_eq!(f.bytes().len(), HEADER + 2 + DIGEST);
    drop(claims);
    for pid in 1..=9 {
        assert_eq!(
            error(f.open(9).unwrap().claim(identity(pid))),
            Error::Reused
        );
    }
    let largest = Fixture::new();
    let mut claims = largest.create(MAX_PID).unwrap();
    claims.claim(identity(MAX_PID)).unwrap();
    assert_eq!(largest.bytes().len(), HEADER + 524_289 + DIGEST);
}

#[test]
fn recovery_requires_existing_complete_image_same_vm_range_and_canonical_bits() {
    let missing = Fixture::new();
    assert_eq!(error(missing.open(16)), Error::Missing);
    assert!(!missing.directory.path().join(NAME).exists());
    let f = Fixture::new();
    drop(f.create(10).unwrap());
    let original = f.bytes();
    assert_eq!(
        error(PidClaims::open(
            f.fd(),
            f.owner,
            Binding::new([1; 32]).unwrap(),
            10
        )),
        Error::BindingMismatch
    );
    assert_eq!(error(f.open(11)), Error::RangeMismatch);
    for mode in 0..8 {
        let mut altered = original.clone();
        match mode {
            0 => {
                altered.pop();
            }
            1 => altered.push(0),
            2 => altered[HEADER] ^= 2,
            3 => altered[0] ^= 1,
            4 => {
                altered[HEADER] &= !1;
                seal(&mut altered);
            }
            5 => {
                altered[HEADER + 1] |= 128;
                seal(&mut altered);
            }
            6 => altered.clear(),
            7 => altered[HEADER + 2] ^= 1,
            _ => unreachable!(),
        }
        f.replace_bytes(&altered);
        assert_eq!(error(f.open(10)), Error::Corrupt, "case {mode}");
        assert_eq!(altered, f.bytes(), "recovery must not repair case {mode}");
        assert_eq!(error(f.create(10)), Error::AlreadyExists);
    }
    f.replace_bytes(&original);
    assert!(f.open(10).is_ok());
}

#[test]
fn unsafe_storage_and_multiple_writers_are_refused_without_replacement() {
    let f = Fixture::new();
    assert_eq!(
        error(PidClaims::create(f.fd(), f.owner + 1, binding(), 31)),
        Error::Untrusted
    );
    fs::set_permissions(f.directory.path(), fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(error(f.create(31)), Error::Untrusted);
    fs::set_permissions(f.directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let claims = f.create(31).unwrap();
    assert_eq!(error(f.open(31)), Error::Busy);
    let inherited = claims.file.try_clone().unwrap();
    drop(claims);
    drop(f.open(31).unwrap()); // Explicit unlock also releases the inherited FD.
    drop(inherited);
    let path = f.directory.path().join(NAME);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    assert_eq!(error(f.open(31)), Error::Untrusted);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let alias = f.directory.path().join("hardlink");
    fs::hard_link(&path, &alias).unwrap();
    assert_eq!(error(f.open(31)), Error::Untrusted);
    fs::remove_file(&alias).unwrap();
    fs::rename(&path, &alias).unwrap();
    symlink(&alias, &path).unwrap();
    assert_eq!(error(f.open(31)), Error::Unavailable);
    assert_eq!(error(f.create(31)), Error::AlreadyExists);
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(f.open(31).is_err());
}

#[test]
fn changed_record_or_name_poison_original_writer_before_any_new_receipt() {
    for replace_name in [false, true] {
        let f = Fixture::new();
        let mut claims = f.create(31).unwrap();
        claims.claim(identity(1)).unwrap();
        let old = f.bytes();
        if replace_name {
            let path = f.directory.path().join(NAME);
            fs::rename(&path, f.directory.path().join("retired")).unwrap();
            fs::write(&path, &old).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        } else {
            let mut corrupted = old.clone();
            corrupted[HEADER] ^= 2;
            seal(&mut corrupted); // Even a validly sealed in-process rollback differs.
            f.replace_bytes(&corrupted);
        }
        assert_eq!(
            error(claims.claim(identity(2))),
            if replace_name {
                Error::Untrusted
            } else {
                Error::Corrupt
            }
        );
        assert_eq!(error(claims.claim(identity(3))), Error::Poisoned);
        assert_eq!(
            claims.image, old,
            "failed validation must precede bitmap mutation"
        );
    }
}

#[test]
fn partial_writes_unknown_sync_and_panic_never_grant_a_reusable_receipt() {
    for fault in [
        Stage::BeforeWrite,
        Stage::BitWritten,
        Stage::DigestWritten,
        Stage::Synced,
    ] {
        let f = Fixture::new();
        let mut claims = f.create(31).unwrap();
        claims.claim(identity(1)).unwrap();
        assert_eq!(
            error(claims.claim_with(identity(2), |stage| if stage == fault {
                Err(Error::WriteUnknown)
            } else {
                Ok(())
            })),
            Error::WriteUnknown
        );
        assert_eq!(error(claims.claim(identity(3))), Error::Poisoned);
        drop(claims);
        if fault == Stage::BitWritten {
            assert_eq!(error(f.open(31)), Error::Corrupt);
        } else {
            // Reopen never repairs: it accepts only a complete image and
            // confirms durability again. PID 1 must remain burned throughout.
            let mut reopened = f.open(31).unwrap();
            assert_eq!(error(reopened.claim(identity(1))), Error::Reused);
            if fault == Stage::BeforeWrite {
                reopened.claim(identity(2)).unwrap();
            } else {
                assert_eq!(error(reopened.claim(identity(2))), Error::Reused);
            }
        }
    }
    let f = Fixture::new();
    let mut claims = f.create(31).unwrap();
    let unwound = catch_unwind(AssertUnwindSafe(|| {
        claims.claim_with(identity(1), |_| panic!("controlled writer interruption"))
    }));
    assert!(unwound.is_err());
    assert_eq!(error(claims.claim(identity(2))), Error::Poisoned);
}
