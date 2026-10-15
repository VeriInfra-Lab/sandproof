//! Stable terminal descriptors. Subsequent input/output/resize use no path or
//! numeric process lookup, and need no blocking helper thread.
use std::io;
use std::os::fd::OwnedFd;
use std::sync::{Arc, Mutex, PoisonError};

use rustix::fs::{OFlags, fcntl_getfl, fcntl_setfl};
use rustix::pty::{OpenptFlags, grantpt, ioctl_tiocgptpeer, openpt, unlockpt};
use rustix::termios::{Winsize, tcsetwinsize};
use tokio::io::Interest;
use tokio::io::unix::AsyncFd;
use tokio::sync::Notify;

use crate::PtySize;

pub(crate) struct PtyMaster {
    descriptor: Mutex<Option<Arc<AsyncFd<OwnedFd>>>>,
    closed: Notify,
}

fn closed() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "terminal closed")
}

impl PtyMaster {
    pub(crate) fn open(size: PtySize) -> io::Result<(Arc<Self>, OwnedFd)> {
        let flags = OpenptFlags::RDWR | OpenptFlags::NOCTTY | OpenptFlags::CLOEXEC;
        let master = openpt(flags)?;
        grantpt(&master)?;
        unlockpt(&master)?;
        // Open the exact peer from the master, not a replaceable /dev/pts path.
        let slave = ioctl_tiocgptpeer(&master, flags)?;
        fcntl_setfl(&master, fcntl_getfl(&master)? | OFlags::NONBLOCK)?;
        let master = Arc::new(Self {
            descriptor: Mutex::new(Some(Arc::new(AsyncFd::new(master)?))),
            closed: Notify::new(),
        });
        master.resize(size)?;
        Ok((master, slave))
    }

    pub(crate) fn resize(&self, size: PtySize) -> io::Result<()> {
        let descriptor = self.descriptor()?;
        tcsetwinsize(
            descriptor.get_ref(),
            Winsize {
                ws_col: size.cols,
                ws_row: size.rows,
                ws_xpixel: 0,
                ws_ypixel: 0,
            },
        )?;
        Ok(())
    }

    fn descriptor(&self) -> io::Result<Arc<AsyncFd<OwnedFd>>> {
        self.descriptor
            .lock()
            .map_err(|_| io::Error::other("terminal owner poisoned"))?
            .as_ref()
            .cloned()
            .ok_or_else(closed)
    }

    /// Revoke every clone's access to this exact descriptor, waking pending
    /// reads/writes. An in-flight operation retains its FD until it returns;
    /// close never uses a raw descriptor number that the kernel could recycle.
    pub(crate) fn close(&self) {
        self.descriptor
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        self.closed.notify_waiters();
    }

    pub(crate) async fn read(&self, bytes: &mut [u8]) -> io::Result<usize> {
        let closing = self.closed.notified();
        tokio::pin!(closing);
        closing.as_mut().enable();
        let descriptor = self.descriptor()?;
        tokio::select! {
            biased;
            () = closing => Err(closed()),
            result = descriptor.async_io(Interest::READABLE, |fd| {
                loop {
                    match rustix::io::read(fd, &mut *bytes) {
                        Err(rustix::io::Errno::INTR) => {}
                        // Linux master EIO is the terminal end-of-stream observation.
                        // Actual child wait remains separate; do not normalize pipe errors.
                        Err(rustix::io::Errno::IO) => return Ok(0),
                        result => return result.map_err(Into::into),
                    }
                }
            }) => result,
        }
    }

    pub(crate) async fn write_all(&self, mut bytes: &[u8]) -> io::Result<()> {
        let closing = self.closed.notified();
        tokio::pin!(closing);
        closing.as_mut().enable();
        let descriptor = self.descriptor()?;
        let write = async {
            while !bytes.is_empty() {
                let mut ready = descriptor.writable().await?;
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
        };
        tokio::select! {
            biased;
            () = closing => Err(closed()),
            result = write => result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::{Future, poll_fn};
    use std::task::Poll;
    use std::time::Duration;

    #[tokio::test]
    async fn close_wakes_pending_io_and_revokes_all_clones_without_reusing_fd() {
        let (master, slave) = PtyMaster::open(PtySize::default()).unwrap();
        let clone = Arc::clone(&master);
        let descriptor = Arc::downgrade(&master.descriptor().unwrap());
        let mut attributes = rustix::termios::tcgetattr(&slave).unwrap();
        attributes.make_raw();
        rustix::termios::tcsetattr(&slave, rustix::termios::OptionalActions::Now, &attributes)
            .unwrap();
        let mut bytes = [0; 8];
        let mut read = Box::pin(master.read(&mut bytes));
        assert!(
            poll_fn(|cx| Poll::Ready(read.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        let payload = vec![b'x'; 1024 * 1024];
        let mut write = Box::pin(clone.write_all(&payload));
        assert!(
            poll_fn(|cx| Poll::Ready(write.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        master.close();
        master.close();
        assert!(clone.resize(PtySize::default()).is_err());
        for result in [
            tokio::time::timeout(Duration::from_secs(1), read)
                .await
                .unwrap()
                .map(|_| ()),
            tokio::time::timeout(Duration::from_secs(1), write)
                .await
                .unwrap(),
        ] {
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
        }
        assert!(descriptor.upgrade().is_none());
        assert!(master.read(&mut bytes).await.is_err());
        assert!(master.write_all(b"late").await.is_err());
        drop(slave);
    }
}
