//! Original request fields and the original worker's selected launch rejection.
use super::reply;
use sandproof_guest_runtime::{file_download::LinuxErrno, protocol::StartRequest};

#[derive(Clone, Copy)]
pub(crate) enum LaunchFailureKind {
    Native(i32),
    EnvironmentNul,
    ArgumentNul,
}

pub(crate) struct LaunchFailure {
    program: String,
    args: Vec<String>,
    cwd: String,
    pty: Option<(u32, u32)>,
    pub(super) kind: LaunchFailureKind,
}
impl LaunchFailure {
    pub(crate) fn native(request: StartRequest, cwd: String, errno: i32) -> Self {
        Self::new(request, cwd, LaunchFailureKind::Native(errno))
    }

    pub(crate) fn new(request: StartRequest, cwd: String, kind: LaunchFailureKind) -> Self {
        let (program, args) = request.process.map(|p| (p.cmd, p.args)).unwrap_or_default();
        let pty = request
            .pty
            .map(|p| p.size.map_or((0, 0), |s| (s.cols, s.rows)));
        Self {
            program,
            args,
            cwd,
            pty,
            kind,
        }
    }

    pub(super) fn payload(&self, limit: usize) -> Result<Vec<u8>, reply::Error> {
        crate::go_json::bounded(limit as u64, |put| {
            put(br#"{"error":{"code":""#)?;
            put(
                if matches!(self.kind, LaunchFailureKind::Native(11 | 12 | 23 | 24 | 28)) {
                    b"resource_exhausted"
                } else {
                    b"invalid_argument"
                },
            )?;
            put(br#"","message":""#)?;
            let mut message = |bytes: &[u8]| crate::go_json::string_content(bytes, put);
            message(if self.pty.is_some() {
                b"error starting pty with command '"
            } else {
                b"error starting process '"
            })?;
            message(self.program.as_bytes())?;
            for arg in &self.args {
                message(b" ")?;
                message(arg.as_bytes())?;
            }
            message(b"'")?;
            if let Some((cols, rows)) = self.pty {
                message(b" in dir '")?;
                message(self.cwd.as_bytes())?;
                message(b"' with '")?;
                message(cols.to_string().as_bytes())?;
                message(b"' cols and '")?;
                message(rows.to_string().as_bytes())?;
                message(b"' rows")?;
            }
            match self.kind {
                LaunchFailureKind::Native(errno) => {
                    message(b": fork/exec /bin/sh: ")?;
                    message(LinuxErrno(errno).message().as_bytes())?;
                }
                LaunchFailureKind::EnvironmentNul => {
                    message(b": exec: environment variable contains NUL")?;
                }
                LaunchFailureKind::ArgumentNul => {
                    message(b": fork/exec /bin/sh: invalid argument")?;
                }
            }
            put(br#""}}"#)
        })
        .map_err(|_| reply::Error::ResponseLimit)
    }
}
