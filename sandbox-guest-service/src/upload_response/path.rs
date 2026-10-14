//! Fixed permissions.ExpandAndResolve rejection of ~other paths.
//! runtime f455fd58fa3f289e78ce1de690c35968d8d29322, Apache-2.0.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathExpansion {
    username: String,
}
impl PathExpansion {
    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn new(username: &str) -> Self {
        Self {
            username: username.into(),
        }
    }

    pub(crate) fn upload_parts(&self) -> [&[u8]; 3] {
        [
            b"error resolving path: failed to expand path '' for user '",
            self.username.as_bytes(),
            b"': cannot expand user-specific home dir",
        ]
    }

    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn download_parts<'a>(&'a self, original: &'a str) -> [&'a [u8]; 5] {
        // Go assigns expand's empty error result to its local path. The outer
        // GET wrapper still reports the original query (empty for defaults).
        [
            b"error expanding and resolving path '",
            original.as_bytes(),
            b"': failed to expand path '' for user '",
            self.username.as_bytes(),
            b"': cannot expand user-specific home dir",
        ]
    }
}

#[cfg(test)]
mod tests;
