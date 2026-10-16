use super::*;

#[test]
fn sessions_survive_restart_and_lost_return_without_resetting_the_pid_set() {
    let f = Fixture::new();
    let mut claims = f.create(31).unwrap();
    let size = f.bytes().len();
    claims.claim(identity(7)).unwrap();
    assert_eq!(claims.allocate_session().unwrap(), 1);
    // A lost startup reply or dropping all service state must burn this number.
    assert_eq!(claims.allocate_session().unwrap(), 2);
    drop(claims);
    for expected in 3..=5 {
        let mut reopened = f.open(31).unwrap();
        assert_eq!(reopened.allocate_session().unwrap(), expected);
        assert_eq!(error(reopened.claim(identity(7))), Error::Reused);
        reopened
            .claim(identity(u32::try_from(expected).unwrap()))
            .unwrap();
        assert_eq!(f.bytes().len(), size);
    }
    let mut reopened = f.open(31).unwrap();
    for pid in [3, 4, 5, 7] {
        assert_eq!(error(reopened.claim(identity(pid))), Error::Reused);
    }
    assert_eq!(reopened.allocate_session().unwrap(), 6);
}

#[test]
fn final_session_can_be_burned_but_exhaustion_never_wraps_or_writes() {
    let f = Fixture::new();
    drop(f.create(31).unwrap());
    let mut image = f.bytes();
    image[SESSION].copy_from_slice(&(u64::MAX - 1).to_le_bytes());
    seal(&mut image);
    f.replace_bytes(&image);
    let mut claims = f.open(31).unwrap();
    assert_eq!(claims.allocate_session().unwrap(), u64::MAX);
    let exhausted = f.bytes();
    assert_eq!(error(claims.allocate_session()), Error::SessionExhausted);
    assert_eq!(f.bytes(), exhausted);
    drop(claims);
    let mut reopened = f.open(31).unwrap();
    assert_eq!(error(reopened.allocate_session()), Error::SessionExhausted);
    assert_eq!(f.bytes(), exhausted);
    // The counter's capacity error does not mutate or poison the PID bitmap.
    reopened.claim(identity(1)).unwrap();
}

#[test]
fn uncertain_session_writes_poison_both_operations_and_only_complete_images_reopen() {
    for fault in [
        Stage::BeforeWrite,
        Stage::SessionWritten,
        Stage::DigestWritten,
        Stage::Synced,
    ] {
        let f = Fixture::new();
        let mut claims = f.create(31).unwrap();
        claims.claim(identity(7)).unwrap();
        assert_eq!(claims.allocate_session().unwrap(), 1);
        assert_eq!(
            error(claims.allocate_session_with(|stage| {
                if stage == fault {
                    Err(Error::WriteUnknown)
                } else {
                    Ok(())
                }
            })),
            Error::WriteUnknown
        );
        assert_eq!(error(claims.allocate_session()), Error::Poisoned);
        assert_eq!(error(claims.claim(identity(8))), Error::Poisoned);
        let interrupted = f.bytes();
        drop(claims);
        if fault == Stage::SessionWritten {
            assert_eq!(error(f.open(31)), Error::Corrupt);
        } else {
            let mut reopened = f.open(31).unwrap();
            assert_eq!(f.bytes(), interrupted, "open must never repair");
            assert_eq!(error(reopened.claim(identity(7))), Error::Reused);
            let expected = if fault == Stage::BeforeWrite { 2 } else { 3 };
            assert_eq!(reopened.allocate_session().unwrap(), expected);
        }
    }
}

#[test]
fn changed_storage_and_panics_cannot_leave_a_usable_session_writer() {
    for panic in [false, true] {
        let f = Fixture::new();
        let mut claims = f.create(31).unwrap();
        claims.allocate_session().unwrap();
        if panic {
            assert!(
                catch_unwind(AssertUnwindSafe(|| {
                    claims.allocate_session_with(|_| panic!("controlled session interruption"))
                }))
                .is_err()
            );
        } else {
            let mut image = f.bytes();
            image[HEADER] ^= 2;
            f.replace_bytes(&image);
            assert_eq!(error(claims.allocate_session()), Error::Corrupt);
            assert_eq!(f.bytes(), image);
        }
        assert_eq!(error(claims.allocate_session()), Error::Poisoned);
        assert_eq!(error(claims.claim(identity(1))), Error::Poisoned);
    }
}

#[test]
fn previous_format_is_refused_in_place_without_migration_or_empty_fallback() {
    let f = Fixture::new();
    let mut claims = f.create(31).unwrap();
    claims.claim(identity(7)).unwrap();
    drop(claims);
    let mut legacy = f.bytes();
    legacy.drain(SESSION);
    legacy[..8].copy_from_slice(b"SPPID\0\x01\0");
    seal(&mut legacy);
    for pad in [false, true] {
        let mut image = legacy.clone();
        if pad {
            image.splice(44..44, [0; 8]);
            seal(&mut image);
        }
        f.replace_bytes(&image);
        assert_eq!(error(f.open(31)), Error::Corrupt);
        assert_eq!(error(f.create(31)), Error::AlreadyExists);
        assert_eq!(f.bytes(), image);
    }
}
