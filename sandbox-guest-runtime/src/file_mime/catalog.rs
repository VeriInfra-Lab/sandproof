// Adapted from Go 1.26.8 mime/type.go and type_unix.go.
// Copyright 2010 The Go Authors. BSD terms: ../multipart/GO-LICENSE.
use super::{Error, builtins, format};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug)]
pub struct CatalogLimits {
    pub input_bytes: usize,
    /// Raw line bytes, including newline. At most 65535 (a conservative
    /// boundary below Go Scanner's 65536-byte buffer ceiling).
    pub line_bytes: usize,
    /// Combined entries in exact-case and lower-case maps, including builtins.
    pub entries: usize,
    /// Combined key/value bytes in both maps, including builtins.
    pub retained_bytes: usize,
    pub value_bytes: usize,
}

/// Immutable snapshot. The caller supplies successful opens in original Unix
/// order; None means open failure. Invalid/oversized material rejects startup
/// instead of publishing a partial or silently builtin-only catalog.
pub struct Catalog {
    exact: BTreeMap<String, String>,
    lower: BTreeMap<String, String>,
    retained: usize,
    limits: CatalogLimits,
}
impl Catalog {
    /// First available globs2 wins (even empty); otherwise all available
    /// mime.types files are applied in order. Source bytes are not retained.
    /// Locations/order are fixed by Go: /usr/local/share/mime/globs2,
    /// /usr/share/mime/globs2; /etc/mime.types, /etc/apache2/mime.types,
    /// /etc/apache/mime.types, /etc/httpd/conf/mime.types.
    /// # Errors
    /// Configuration/aggregate input, line, value, map count/bytes or UTF-8.
    /// This explicit fail-closed boundary replaces Go scanner panic and does
    /// not claim compatibility for malformed non-UTF-8 system databases.
    pub fn from_sources(
        globs: [Option<&[u8]>; 2],
        files: [Option<&[u8]>; 4],
        limits: CatalogLimits,
    ) -> Result<Self, Error> {
        if [
            limits.input_bytes,
            limits.line_bytes,
            limits.entries,
            limits.retained_bytes,
            limits.value_bytes,
        ]
        .iter()
        .any(|n| *n == 0 || *n > isize::MAX as usize / 16)
            || limits.line_bytes > 65535
        {
            return Err(Error::InvalidConfiguration);
        }
        let mut catalog = Self {
            exact: BTreeMap::new(),
            lower: BTreeMap::new(),
            retained: 0,
            limits,
        };
        for (ext, kind) in builtins::TYPES {
            catalog.store(ext, kind)?;
        }
        if let Some(data) = globs.into_iter().flatten().next() {
            catalog.load(data, true)?;
        } else {
            let total = files
                .iter()
                .flatten()
                .try_fold(0_usize, |n, bytes| n.checked_add(bytes.len()));
            if total.is_none_or(|n| n > limits.input_bytes) {
                return Err(Error::InputLimit);
            }
            for data in files.into_iter().flatten() {
                catalog.load(data, false)?;
            }
        }
        Ok(catalog)
    }
    /// Exact-case first, then Go simple Unicode lower-case fallback.
    /// # Errors
    /// An unbounded extension is rejected before normalization allocation.
    pub fn lookup(&self, ext: &str) -> Result<Option<&str>, Error> {
        if ext.len() > self.limits.value_bytes {
            return Err(Error::InputLimit);
        }
        Ok(self
            .exact
            .get(ext)
            .or_else(|| self.lower.get(&lower(ext)))
            .map(String::as_str))
    }
    fn load(&mut self, bytes: &[u8], globs: bool) -> Result<(), Error> {
        if bytes.len() > self.limits.input_bytes {
            return Err(Error::InputLimit);
        }
        let text = std::str::from_utf8(bytes).map_err(|_| Error::UnsupportedText)?;
        for line in text.split_inclusive('\n') {
            if line.len() > self.limits.line_bytes {
                return Err(Error::InputLimit);
            }
            let line = line.strip_suffix('\n').unwrap_or(line);
            let line = line.strip_suffix('\r').unwrap_or(line);
            if globs {
                self.glob(line)?;
            } else {
                self.types(line)?;
            }
        }
        Ok(())
    }
    fn glob(&mut self, line: &str) -> Result<(), Error> {
        let mut fields = line.split(':');
        let weight = fields.next().unwrap_or("");
        let kind = fields.next().unwrap_or("");
        let glob = fields.next().unwrap_or("");
        if weight.is_empty() || weight.starts_with('#') || glob.len() < 3 || !glob.starts_with("*.")
        {
            return Ok(());
        }
        let ext = &glob[1..];
        if ext.contains(['?', '*', '[']) || self.exact.contains_key(ext) {
            return Ok(());
        }
        self.add(ext, kind)
    }
    fn types(&mut self, line: &str) -> Result<(), Error> {
        let mut fields = line.split_whitespace();
        let Some(kind) = fields.next().filter(|s| !s.starts_with('#')) else {
            return Ok(());
        };
        for ext in fields.take_while(|s| !s.starts_with('#')) {
            if ext.len() >= self.limits.value_bytes {
                return Err(Error::InputLimit);
            }
            self.add(&format!(".{ext}"), kind)?;
        }
        Ok(())
    }
    fn add(&mut self, ext: &str, kind: &str) -> Result<(), Error> {
        if ext.len() > self.limits.value_bytes || kind.len() > self.limits.value_bytes {
            return Err(Error::InputLimit);
        }
        // Go lowercases Unicode in the base type before parsing; keep the raw
        // kind for setExtensionType's original case-sensitive text/ behavior.
        let (base, rest) = kind.find(';').map_or((kind, ""), |n| kind.split_at(n));
        let mut normalized = lower(base);
        normalized.push_str(rest);
        let mut params = match crate::multipart::media_type(&normalized) {
            Ok((_, params)) => params,
            Err(crate::multipart::MediaError::Invalid) => return Ok(()),
            Err(crate::multipart::MediaError::NonUtf8) => return Err(Error::UnsupportedText),
        };
        if kind.starts_with("text/") && params.get("charset").is_none_or(String::is_empty) {
            params.insert("charset".into(), "utf-8".into());
            let kind = format::media_type(kind, &params, self.limits.value_bytes)?;
            self.store(ext, &kind)
        } else {
            self.store(ext, kind)
        }
    }
    fn store(&mut self, ext: &str, kind: &str) -> Result<(), Error> {
        if ext.len() > self.limits.value_bytes || kind.len() > self.limits.value_bytes {
            return Err(Error::InputLimit);
        }
        let folded = lower(ext);
        let old_exact = self.exact.get(ext);
        let old_lower = self.lower.get(&folded);
        let count = self.exact.len()
            + self.lower.len()
            + usize::from(old_exact.is_none())
            + usize::from(old_lower.is_none());
        let removed = old_exact.map_or(0, |old| ext.len() + old.len())
            + old_lower.map_or(0, |old| folded.len() + old.len());
        let size = self
            .retained
            .checked_sub(removed)
            .and_then(|n| n.checked_add(ext.len()))
            .and_then(|n| n.checked_add(folded.len()))
            .and_then(|n| n.checked_add(kind.len().checked_mul(2)?))
            .filter(|n| *n <= self.limits.retained_bytes)
            .ok_or(Error::RetainedLimit)?;
        if count > self.limits.entries {
            return Err(Error::RetainedLimit);
        }
        self.exact.insert(ext.into(), kind.into());
        self.lower.insert(folded, kind.into());
        self.retained = size;
        Ok(())
    }
}
pub(crate) fn lower(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c == '\u{130}' {
                'i'
            } else {
                c.to_lowercase().next().unwrap_or(c)
            }
        })
        .collect()
}
