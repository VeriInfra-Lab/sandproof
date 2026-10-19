// Header behavior adapted from Go 1.26.8 compress/gzip/gunzip.go.
// Copyright 2009 The Go Authors. BSD license: ../GO-LICENSE.
// Metadata is checked and discarded through a fixed stack buffer, never stored.
use super::Error;
use crc32fast::Hasher;
use std::io::{BufRead, Read};

struct Header<'a, R> {
    reader: &'a mut R,
    remaining: usize,
    crc: Hasher,
}
impl<R: Read> Header<'_, R> {
    fn bytes(&mut self, output: &mut [u8]) -> Result<(), Error> {
        if output.len() > self.remaining {
            return Err(Error::HeaderLimit);
        }
        self.reader.read_exact(output)?;
        self.remaining -= output.len();
        self.crc.update(output);
        Ok(())
    }
    fn string(&mut self) -> Result<(), Error> {
        // Go's Reader has a 512-byte buffer, including the NUL terminator.
        for _ in 0..512 {
            let mut byte = [0];
            self.bytes(&mut byte)?;
            if byte[0] == 0 {
                return Ok(());
            }
        }
        Err(Error::Header)
    }
}

pub(super) fn read<R: BufRead>(reader: &mut R, limit: usize) -> Result<(), Error> {
    let mut header = Header {
        reader,
        remaining: limit,
        crc: Hasher::new(),
    };
    let mut fixed = [0; 10];
    header.bytes(&mut fixed)?;
    if fixed[..3] != [0x1f, 0x8b, 8] {
        return Err(Error::Header);
    }
    // The fixed Go reader ignores FTEXT, XFL, OS and reserved flag bits.
    let flags = fixed[3];
    if flags & 4 != 0 {
        let mut length = [0; 2];
        header.bytes(&mut length)?;
        let mut remaining = usize::from(u16::from_le_bytes(length));
        let mut scratch = [0; 512];
        while remaining != 0 {
            let count = remaining.min(scratch.len());
            header.bytes(&mut scratch[..count])?;
            remaining -= count;
        }
    }
    if flags & 8 != 0 {
        header.string()?;
    }
    if flags & 16 != 0 {
        header.string()?;
    }
    if flags & 2 != 0 {
        let expected = header.crc.clone().finalize().to_le_bytes();
        let mut actual = [0; 2];
        header.bytes(&mut actual)?;
        if actual != expected[..2] {
            return Err(Error::Header);
        }
    }
    Ok(())
}
