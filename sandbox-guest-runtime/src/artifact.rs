//! Bounded fingerprint observation on an already opened trusted regular file.
//! This is not an execution handle, signature, source attestation or protection
//! against the trusted owner changing an installation after this observation.
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
};

#[cfg(test)]
mod tests;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fingerprint {
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Configuration,
    Input,
    Mismatch,
    Cancelled,
}

/// Validated expected bytes, supplied by the trusted installation descriptor.
#[derive(Clone, Copy)]
pub struct Expected {
    bytes: u64,
    digest: [u8; 32],
}

impl Fingerprint {
    /// Validate exact positive length and canonical lowercase SHA-256 syntax.
    /// # Errors
    /// Missing/invalid length or digest. No file is opened or executed.
    pub fn validate(&self) -> Result<Expected, Error> {
        if self.bytes == 0 || self.bytes > isize::MAX as u64 || self.sha256.len() != 64 {
            return Err(Error::Configuration);
        }
        let mut digest = [0; 32];
        for (slot, pair) in digest
            .iter_mut()
            .zip(self.sha256.as_bytes().chunks_exact(2))
        {
            let digit = |b| match b {
                b'0'..=b'9' => Ok(b - b'0'),
                b'a'..=b'f' => Ok(b - b'a' + 10),
                _ => Err(Error::Configuration),
            };
            *slot = digit(pair[0])? * 16 + digit(pair[1])?;
        }
        Ok(Expected {
            bytes: self.bytes,
            digest,
        })
    }
}

impl Expected {
    /// Read from offset zero with an 8 KiB scratch buffer and a one-byte excess
    /// probe. The caller first opens/checks trusted ancestry, owner and mode.
    /// `continue_reading` checks the original deadline/cancellation before I/O
    /// and after hashing. It cannot interrupt a filesystem call already entered.
    /// # Errors
    /// Wrong type, I/O, size/digest mismatch or cancellation. No execution,
    /// mutation, retry, repair or pathname reopen occurs here.
    pub fn verify(
        self,
        mut file: File,
        mut continue_reading: impl FnMut() -> bool,
    ) -> Result<(), Error> {
        let checkpoint = |proceed: bool| {
            if proceed {
                Ok(())
            } else {
                Err(Error::Cancelled)
            }
        };
        checkpoint(continue_reading())?;
        self.metadata(&file)?;
        file.seek(SeekFrom::Start(0)).map_err(|_| Error::Input)?;
        let mut remaining = self.bytes;
        let mut digest = Sha256::new();
        let mut buffer = [0; 8192];
        loop {
            checkpoint(continue_reading())?;
            let maximum = usize::try_from((remaining + 1).min(buffer.len() as u64))
                .map_err(|_| Error::Configuration)?;
            let count = match file.read(&mut buffer[..maximum]) {
                Ok(n) => n,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => return Err(Error::Input),
            };
            if count == 0 {
                break;
            }
            remaining = remaining.checked_sub(count as u64).ok_or(Error::Mismatch)?;
            digest.update(&buffer[..count]);
        }
        checkpoint(continue_reading())?;
        self.metadata(&file)?;
        if remaining != 0 || digest.finalize()[..] != self.digest {
            return Err(Error::Mismatch);
        }
        checkpoint(continue_reading())
    }

    fn metadata(self, file: &File) -> Result<(), Error> {
        let metadata = file.metadata().map_err(|_| Error::Input)?;
        if !metadata.is_file() {
            return Err(Error::Input);
        }
        if metadata.len() != self.bytes {
            return Err(Error::Mismatch);
        }
        Ok(())
    }
}
