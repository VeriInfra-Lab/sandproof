//! Own an unreaped synchronous helper across every early-return path.
use std::{
    io::{self, Read},
    ops::{Deref, DerefMut},
    process::{Child, Output},
    thread,
};

pub(super) struct OwnedChild(Option<Child>);
impl OwnedChild {
    pub(super) fn new(child: Child) -> Self {
        Self(Some(child))
    }
    pub(super) fn into_child(mut self) -> Child {
        self.0.take().expect("owned child")
    }
    fn stop_and_reap(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    pub(super) fn wait_output(&mut self) -> io::Result<Output> {
        drop(self.stdin.take());
        let stdout = self.stdout.take();
        let stderr = self.stderr.take();
        thread::scope(|scope| {
            // Fallible thread admission ensures an OS thread-limit error
            // stops the child before the scope joins a still-blocked reader.
            let stdout = match thread::Builder::new().spawn_scoped(scope, move || read_pipe(stdout))
            {
                Ok(reader) => reader,
                Err(error) => {
                    self.stop_and_reap();
                    return Err(error);
                }
            };
            let stderr = match thread::Builder::new().spawn_scoped(scope, move || read_pipe(stderr))
            {
                Ok(reader) => reader,
                Err(error) => {
                    self.stop_and_reap();
                    return Err(error);
                }
            };
            let status = self.wait();
            if status.is_err() {
                self.stop_and_reap();
            }
            let stdout = stdout
                .join()
                .map_err(|_| io::Error::other("stdout helper reader panicked"))??;
            let stderr = stderr
                .join()
                .map_err(|_| io::Error::other("stderr helper reader panicked"))??;
            Ok(Output {
                status: status?,
                stdout,
                stderr,
            })
        })
    }
}
impl Deref for OwnedChild {
    type Target = Child;
    fn deref(&self) -> &Child {
        self.0.as_ref().expect("owned child")
    }
}
impl DerefMut for OwnedChild {
    fn deref_mut(&mut self) -> &mut Child {
        self.0.as_mut().expect("owned child")
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        self.stop_and_reap();
    }
}
fn read_pipe(pipe: Option<impl Read>) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    if let Some(mut pipe) = pipe {
        pipe.read_to_end(&mut bytes)?;
    }
    Ok(bytes)
}
