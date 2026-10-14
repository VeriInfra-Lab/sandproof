//! Stable terminal descriptors. Subsequent input/output/resize use no path or
//! numeric process lookup, and need no blocking helper thread.
use std::io;
use std::os::fd::OwnedFd;
use std::sync::Arc;

use rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};
use rustix::pty::{OpenptFlags, grantpt, ioctl_tiocgptpeer, openpt, unlockpt};
use rustix::termios::{Winsize, tcsetwinsize};
use tokio::io::Interest;
use tokio::io::unix::AsyncFd;

use crate::PtySize;

pub(crate) struct PtyMaster(AsyncFd<OwnedFd>);

impl PtyMaster {
    pub(crate) fn open(size: PtySize) -> io::Result<(Arc<Self>, OwnedFd)> {
        let flags = OpenptFlags::RDWR | OpenptFlags::NOCTTY | OpenptFlags::CLOEXEC;
        let master = openpt(flags)?;
        grantpt(&master)?;
        unlockpt(&master)?;
        // Open the exact peer from the master, not a replaceable /dev/pts path.
        let slave = ioctl_tiocgptpeer(&master, flags)?;
        fcntl_setfl(&master, fcntl_getfl(&master)? | OFlags::NONBLOCK)?;
        let master = Arc::new(Self(AsyncFd::new(master)?));
        master.resize(size)?;
        Ok((master, slave))
    }

    pub(crate) fn resize(&self, size: PtySize) -> io::Result<()> {
        tcsetwinsize(
            self.0.get_ref(),
            Winsize {
                ws_col: size.cols,
                ws_row: size.rows,
                ws_xpixel: 0,
                ws_ypixel: 0,
            },
        )?;
        Ok(())
    }

    pub(crate) async fn read(&self, bytes: &mut [u8]) -> io::Result<usize> {
        self.0
            .async_io(Interest::READABLE, |fd| {
                loop {
                    match rustix::io::read(fd, &mut *bytes) {
                        Err(rustix::io::Errno::INTR) => {}
                        // Linux master EIO is the terminal end-of-stream observation.
                        // Actual child wait remains separate; do not normalize pipe errors.
                        Err(rustix::io::Errno::IO) => return Ok(0),
                        result => return result.map_err(Into::into),
                    }
                }
            })
            .await
    }

    pub(crate) async fn write_all(&self, mut bytes: &[u8]) -> io::Result<()> {
        while !bytes.is_empty() {
            let mut ready = self.0.writable().await?;
            // A hung-up Linux master can stay poll-ready while write returns
            // EAGAIN for a full line discipline buffer. Retrying async_io in
            // that state spins and starves the independent wait/expiry owner.
            if ready.ready().is_write_closed() || ready.ready().is_error() {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "terminal hung up",
                ));
            }
            let count = match ready.try_io(|fd| {
                loop {
                    match rustix::io::write(fd.get_ref(), bytes) {
                        Err(rustix::io::Errno::INTR) => {}
                        result => return result.map_err(io::Error::from),
                    }
                }
            }) {
                Ok(result) => result?,
                Err(_) => continue,
            };
            if count == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "terminal write stopped",
                ));
            }
            bytes = &bytes[count..];
        }
        Ok(())
    }
}
