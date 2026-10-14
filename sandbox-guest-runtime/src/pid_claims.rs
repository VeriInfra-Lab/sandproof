//! Durable, VM-bound refusal to reuse a published process PID.
//!
//! All filesystem calls are blocking. The service must own capacity and the
//! gated worker through the real end of these calls, including cancellation.
//! A claim is burned before user execution and is never removed on process
//! exit, lost replies or failed launches. This port does not publish selectors,
//! authenticate requests, restore active processes or release the worker.
//!
//! The directory descriptor must name this VM's trusted, persistent private
//! directory. Its provisioning/ancestry and binding are caller obligations.
//! Guest root, same-owner modification, rollback of a valid old image and
//! storage that lies about fsync are outside this port's guarantees.
use rustix::fs::{Mode, OFlags, openat};
use sandproof_guest_core::CommandIdentity;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    os::unix::fs::{FileExt, MetadataExt},
};

#[cfg(test)]
mod tests;

const NAME: &str = "pid-claims.bin";
const MAGIC: &[u8; 8] = b"SPPID\0\x01\0";
const HEADER: usize = 44;
const DIGEST: usize = 32;
/// Representation ceiling for the Linux `x86_64` component, not a daemon default.
/// Configuration selects a nonzero inclusive bound no larger than this value.
pub const MAX_PID: u32 = 4_194_304;

/// Digest of the trusted VM creation/boot identity, never an access token.
/// Token rotation and service restart must provide the same binding. A changed
/// identity cannot initialize over the old file or silently start an empty set.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Binding([u8; 32]);
impl Binding {
    /// # Errors
    /// An absent/all-zero identity digest is invalid.
    pub fn new(digest: [u8; 32]) -> Result<Self, Error> {
        if digest == [0; 32] {
            Err(Error::Configuration)
        } else {
            Ok(Self(digest))
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Configuration,
    Untrusted,
    Missing,
    AlreadyExists,
    Busy,
    Unavailable,
    Corrupt,
    BindingMismatch,
    RangeMismatch,
    InvalidIdentity,
    OutOfRange,
    Reused,
    WriteUnknown,
    Poisoned,
}

/// Unforgeable through this public API, single-use durable claim for the exact
/// gated child. A registry must check its binding and identity before dispatch.
/// The claim is deliberately neither Clone nor Copy. Dropping it never unclaims.
pub struct Claim {
    binding: Binding,
    identity: CommandIdentity,
}
impl Claim {
    #[must_use]
    pub const fn identity(&self) -> CommandIdentity {
        self.identity
    }
    #[must_use]
    pub fn belongs_to(&self, binding: Binding) -> bool {
        self.binding == binding
    }
}

/// One synchronous writer, locked for the lifetime of this object. Mutating
/// calls require exclusive access; the async service must not wait on this
/// filesystem writer while holding an unbounded request queue.
pub struct PidClaims {
    directory: File,
    file: File,
    owner: u32,
    binding: Binding,
    max_pid: u32,
    image: Vec<u8>,
    poisoned: bool,
}

impl PidClaims {
    #[must_use]
    pub fn belongs_to(&self, binding: Binding) -> bool {
        self.binding == binding
    }

    /// Provision only during creation of a known-new VM, never during normal
    /// daemon startup/recovery. Existing files, including incomplete ones, are
    /// never replaced or removed. The caller supplies an fsync-capable directory
    /// descriptor with private mode and the explicitly trusted owner UID.
    /// # Errors
    /// Invalid configuration, unsafe directory/file, existing name, lock or I/O
    /// failure. An incomplete file is intentionally left in place on failure.
    pub fn create(
        directory: File,
        owner: u32,
        binding: Binding,
        max_pid: u32,
    ) -> Result<Self, Error> {
        let length = image_length(max_pid)?;
        check_directory(&directory, owner)?;
        let fd = openat(
            &directory,
            NAME,
            flags() | OFlags::CREATE | OFlags::EXCL,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(|error| {
            if error == rustix::io::Errno::EXIST {
                Error::AlreadyExists
            } else {
                Error::Unavailable
            }
        })?;
        let file = File::from(fd);
        check_file(&file, owner)?;
        file.try_lock().map_err(|_| Error::Busy)?;
        let mut image = vec![0; length];
        image[..8].copy_from_slice(MAGIC);
        image[8..40].copy_from_slice(&binding.0);
        image[40..HEADER].copy_from_slice(&max_pid.to_le_bytes());
        // PID 0 can never be a real SDK process identity.
        image[HEADER] = 1;
        seal(&mut image);
        let claims = Self {
            directory,
            file,
            owner,
            binding,
            max_pid,
            image,
            poisoned: false,
        };
        claims
            .file
            .write_all_at(&claims.image, 0)
            .map_err(|_| Error::WriteUnknown)?;
        claims.file.sync_all().map_err(|_| Error::WriteUnknown)?;
        claims
            .directory
            .sync_all()
            .map_err(|_| Error::WriteUnknown)?;
        claims.verify()?;
        Ok(claims)
    }

    /// Open the existing set for the same VM and exact configured PID range.
    /// Missing/truncated/corrupt data is fatal: no empty fallback, repair or
    /// migration. Reopening validates the complete image and reconfirms fsync
    /// before any new claim; it never restores old active command records.
    /// # Errors
    /// Missing/unsafe/corrupt/mismatched storage, another writer or I/O failure.
    pub fn open(
        directory: File,
        owner: u32,
        binding: Binding,
        max_pid: u32,
    ) -> Result<Self, Error> {
        let length = image_length(max_pid)?;
        check_directory(&directory, owner)?;
        let fd = openat(&directory, NAME, flags(), Mode::empty()).map_err(|error| {
            if error == rustix::io::Errno::NOENT {
                Error::Missing
            } else {
                Error::Unavailable
            }
        })?;
        let file = File::from(fd);
        check_file(&file, owner)?;
        file.try_lock().map_err(|_| Error::Busy)?;
        if file.metadata().map_err(|_| Error::Unavailable)?.len() != length as u64 {
            return Err(Error::Corrupt);
        }
        let mut image = vec![0; length];
        file.read_exact_at(&mut image, 0)
            .map_err(|_| Error::Unavailable)?;
        validate(&image, binding, max_pid)?;
        let claims = Self {
            directory,
            file,
            owner,
            binding,
            max_pid,
            image,
            poisoned: false,
        };
        claims.file.sync_all().map_err(|_| Error::WriteUnknown)?;
        claims
            .directory
            .sync_all()
            .map_err(|_| Error::WriteUnknown)?;
        claims.verify()?;
        Ok(claims)
    }

    /// Burn this PID durably before releasing the exact prepared worker. A
    /// reused PID is refused without retrying a different worker. Invalid or
    /// out-of-range input makes no write. There is no eviction or reset API.
    /// # Errors
    /// Reused/out-of-range identity, changed/corrupt storage, uncertain write or
    /// an already poisoned writer. Any storage failure poisons this object;
    /// future calls cannot turn an uncertain result into a permission to run.
    pub fn claim(&mut self, identity: CommandIdentity) -> Result<Claim, Error> {
        self.claim_with(identity, |_| Ok(()))
    }

    fn claim_with(
        &mut self,
        identity: CommandIdentity,
        mut checkpoint: impl FnMut(Stage) -> Result<(), Error>,
    ) -> Result<Claim, Error> {
        if self.poisoned {
            return Err(Error::Poisoned);
        }
        if identity.session == 0 || identity.command == 0 || identity.pid == 0 {
            return Err(Error::InvalidIdentity);
        }
        if identity.pid > self.max_pid {
            return Err(Error::OutOfRange);
        }
        let offset = HEADER + identity.pid as usize / 8;
        let mask = 1 << (identity.pid % 8);
        if self.image[offset] & mask != 0 {
            return Err(Error::Reused);
        }
        // Poison first: unwinding or any failure after this point cannot leave
        // a usable writer. Only a fully checked durable success clears it.
        self.poisoned = true;
        self.verify()?;
        self.image[offset] |= mask;
        seal(&mut self.image);
        checkpoint(Stage::BeforeWrite)?;
        self.file
            .write_all_at(&self.image[offset..=offset], offset as u64)
            .map_err(|_| Error::WriteUnknown)?;
        checkpoint(Stage::BitWritten)?;
        let tail = self.image.len() - DIGEST;
        self.file
            .write_all_at(&self.image[tail..], tail as u64)
            .map_err(|_| Error::WriteUnknown)?;
        checkpoint(Stage::DigestWritten)?;
        self.file.sync_all().map_err(|_| Error::WriteUnknown)?;
        checkpoint(Stage::Synced)?;
        self.verify()?;
        self.poisoned = false;
        Ok(Claim {
            binding: self.binding,
            identity,
        })
    }

    fn verify(&self) -> Result<(), Error> {
        check_directory(&self.directory, self.owner)?;
        check_file(&self.file, self.owner)?;
        let named = File::from(
            openat(&self.directory, NAME, flags(), Mode::empty())
                .map_err(|_| Error::Unavailable)?,
        );
        check_file(&named, self.owner)?;
        let actual = self.file.metadata().map_err(|_| Error::Unavailable)?;
        let current = named.metadata().map_err(|_| Error::Unavailable)?;
        if actual.dev() != current.dev() || actual.ino() != current.ino() {
            return Err(Error::Untrusted);
        }
        if actual.len() != self.image.len() as u64 {
            return Err(Error::Corrupt);
        }
        let mut read = vec![0; self.image.len()];
        self.file
            .read_exact_at(&mut read, 0)
            .map_err(|_| Error::Unavailable)?;
        if read != self.image {
            return Err(Error::Corrupt);
        }
        Ok(())
    }
}

impl Drop for PidClaims {
    fn drop(&mut self) {
        // Forked pre-exec descriptors must not extend a retired writer lease.
        let _ = self.file.unlock();
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    BeforeWrite,
    BitWritten,
    DigestWritten,
    Synced,
}

fn flags() -> OFlags {
    OFlags::RDWR | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC
}
fn image_length(max_pid: u32) -> Result<usize, Error> {
    if !(1..=MAX_PID).contains(&max_pid) {
        return Err(Error::Configuration);
    }
    Ok(HEADER + max_pid as usize / 8 + 1 + DIGEST)
}
fn check_directory(directory: &File, owner: u32) -> Result<(), Error> {
    let m = directory.metadata().map_err(|_| Error::Unavailable)?;
    if !m.is_dir() || m.uid() != owner || m.mode() & 0o077 != 0 || m.nlink() == 0 {
        return Err(Error::Untrusted);
    }
    Ok(())
}
fn check_file(file: &File, owner: u32) -> Result<(), Error> {
    let m = file.metadata().map_err(|_| Error::Unavailable)?;
    if !m.is_file() || m.uid() != owner || m.mode() & 0o777 != 0o600 || m.nlink() != 1 {
        return Err(Error::Untrusted);
    }
    Ok(())
}
fn seal(image: &mut [u8]) {
    let tail = image.len() - DIGEST;
    let digest = Sha256::digest(&image[..tail]);
    image[tail..].copy_from_slice(&digest);
}
fn validate(image: &[u8], binding: Binding, max_pid: u32) -> Result<(), Error> {
    let tail = image.len() - DIGEST;
    let digest: [u8; DIGEST] = Sha256::digest(&image[..tail]).into();
    if image[..8] != *MAGIC || digest[..] != image[tail..] {
        return Err(Error::Corrupt);
    }
    if image[8..40] != binding.0 {
        return Err(Error::BindingMismatch);
    }
    if image[40..HEADER] != max_pid.to_le_bytes() {
        return Err(Error::RangeMismatch);
    }
    let allowed = u8::MAX >> (7 - max_pid % 8);
    if image[HEADER] & 1 == 0 || image[tail - 1] & !allowed != 0 {
        return Err(Error::Corrupt);
    }
    Ok(())
}
