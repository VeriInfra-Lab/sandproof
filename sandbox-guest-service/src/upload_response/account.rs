//! Observed missing account, shared by fixed GET/HEAD/POST /files errors.
//! runtime f455fd58fa3f289e78ce1de690c35968d8d29322 api/{download,upload}.go;
//! Go os/user.UnknownUserError. No arbitrary source diagnostic is exposed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownUser {
    name: String,
}
impl UnknownUser {
    // Construct only after the account name budget and a missing-account result.
    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn new(name: &str) -> Self {
        Self { name: name.into() }
    }

    pub(crate) fn parts(&self) -> [&[u8]; 4] {
        [
            b"error looking up user '",
            self.name.as_bytes(),
            b"': user: unknown user ",
            self.name.as_bytes(),
        ]
    }
}

#[cfg(test)]
mod tests;
