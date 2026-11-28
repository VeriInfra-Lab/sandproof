//! Controlled delivery faults around the real Redis engine, never a fake store.
use crate::engine::connect;
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
// Controlled proxy changes only delivery of an actual Redis reply. It neither
// implements nor mocks the authority store. Frames/threads have fixed budgets.
pub(crate) struct ReplyLoss {
    pub(crate) address: SocketAddrV4,
    pub(crate) armed: Arc<AtomicU8>,
    skip_writes: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    timeout_release: Arc<(Mutex<bool>, Condvar)>,
    worker: Option<thread::JoinHandle<()>>,
}
impl ReplyLoss {
    pub(crate) fn new(target: SocketAddrV4) -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, listener.local_addr().unwrap().port());
        let armed = Arc::new(AtomicU8::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let timeout_release = Arc::new((Mutex::new(false), Condvar::new()));
        let worker_release = Arc::clone(&timeout_release);
        let skip_writes = Arc::new(AtomicUsize::new(0));
        let worker_skip = Arc::clone(&skip_writes);
        let worker_arm = Arc::clone(&armed);
        let worker_stop = Arc::clone(&stop);
        let owner = thread::current().name().unwrap_or("unnamed").to_owned();
        let worker = thread::spawn(move || {
            for incoming in listener.incoming() {
                if worker_stop.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(client) = incoming else {
                    break;
                };
                let upstream =
                    TcpStream::connect_timeout(&target.into(), Duration::from_secs(1)).unwrap();
                for stream in [&client, &upstream] {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    stream
                        .set_write_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                }
                let mut client = BufReader::new(client);
                let mut upstream = BufReader::new(upstream);
                while let Ok(Some(request)) = next_request(&mut client, &worker_stop) {
                    let fault = if request.windows(4).any(|part| part == b"HSET") {
                        if worker_skip
                            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                                value.checked_sub(1)
                            })
                            .is_ok()
                        {
                            0
                        } else {
                            worker_arm.swap(0, Ordering::SeqCst)
                        }
                    } else {
                        0
                    };
                    if fault == 2 {
                        // Real noeviction refusal after the adapter's health
                        // check, not a fabricated Redis error response.
                        let _: () = redis::cmd("CONFIG")
                            .arg("SET")
                            .arg("maxmemory")
                            .arg(1)
                            .query(&mut connect(target))
                            .unwrap();
                    }
                    let started = Instant::now();
                    if let Err(error) = upstream.get_mut().write_all(&request) {
                        report(&owner, "upstream-write", fault, started, &error);
                        break;
                    }
                    let reply = match frame(&mut upstream) {
                        Ok(reply) => reply,
                        Err(error) => {
                            report(&owner, "upstream-reply", fault, started, &error);
                            break;
                        }
                    };
                    if fault == 3 {
                        // The actual Redis commit has replied. Hold its reply
                        // until the caller observes its unchanged read timeout.
                        assert_eq!(reply, b":0\r\n");
                        let (ready, changed) = &*worker_release;
                        let (released, _) = changed
                            .wait_timeout_while(
                                ready.lock().unwrap(),
                                Duration::from_secs(5),
                                |value| !*value,
                            )
                            .unwrap();
                        assert!(*released, "test must release the bounded timeout gate");
                        break;
                    }
                    if fault == 1 {
                        assert_eq!(reply, b":0\r\n");
                        break; // commit acknowledged by real engine, not by caller
                    }
                    if let Err(error) = client.get_mut().write_all(&reply) {
                        report(&owner, "client-reply", fault, started, &error);
                        break;
                    }
                }
            }
        });
        Self {
            address,
            armed,
            skip_writes,
            stop,
            timeout_release,
            worker: Some(worker),
        }
    }
    pub(crate) fn timeout_after(&self, preceding_writes: usize) {
        *self.timeout_release.0.lock().unwrap() = false;
        self.skip_writes.store(preceding_writes, Ordering::SeqCst);
        self.armed.store(3, Ordering::SeqCst);
    }
    pub(crate) fn release_timeout(&self) {
        *self.timeout_release.0.lock().unwrap() = true;
        self.timeout_release.1.notify_all();
    }
    pub(crate) fn lose_after(&self, preceding_writes: usize) {
        self.skip_writes.store(preceding_writes, Ordering::SeqCst);
        self.armed.store(1, Ordering::SeqCst);
    }
}
impl Drop for ReplyLoss {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.release_timeout();
        let _ = TcpStream::connect_timeout(&self.address.into(), Duration::from_secs(1));
        self.worker.take().unwrap().join().unwrap();
    }
}
// Failure metadata only: no requests, responses, credentials or arbitrary error
// strings. Keep the original two-second transport limits and fault behavior.
fn report(owner: &str, phase: &str, fault: u8, started: Instant, error: &std::io::Error) {
    eprintln!(
        "redis_fixture_proxy owner={owner} phase={phase} injected_fault={fault} elapsed_us={} kind={:?}",
        started.elapsed().as_micros(),
        error.kind(),
    );
}

fn next_request<R: BufRead>(reader: &mut R, stop: &AtomicBool) -> std::io::Result<Option<Vec<u8>>> {
    loop {
        if stop.load(Ordering::SeqCst) {
            return Ok(None);
        }
        // The original read timeout bounds a frame in progress, not the idle
        // lifetime of this owned connection while a test prepares a real
        // SQLite certificate. Do not invent a delivery fault before byte one.
        match reader.fill_buf() {
            Ok([]) => return Ok(None),
            Ok(_) => return frame(reader).map(Some),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::Interrupted
                        | std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                ) => {}
            Err(error) => return Err(error),
        }
    }
}

fn frame<R: Read>(reader: &mut R) -> std::io::Result<Vec<u8>> {
    fn read<R: Read>(reader: &mut R, output: &mut Vec<u8>, depth: usize) -> std::io::Result<()> {
        if depth > 8 {
            return Err(std::io::Error::other("fixture frame depth"));
        }
        let mut line = Vec::new();
        loop {
            let mut byte = [0];
            reader.read_exact(&mut byte)?;
            line.push(byte[0]);
            if line.len() > 4096 {
                return Err(std::io::Error::other("fixture header bound"));
            }
            if line.ends_with(b"\r\n") {
                break;
            }
        }
        let number = || -> std::io::Result<usize> {
            std::str::from_utf8(&line[1..line.len() - 2])
                .ok()
                .and_then(|s| s.parse().ok())
                .filter(|value| *value < 32768)
                .ok_or_else(|| std::io::Error::other("fixture frame bound"))
        };
        let prefix = line[0];
        let count = if matches!(prefix, b'$' | b'*') {
            Some(number()?)
        } else {
            None
        };
        output.extend_from_slice(&line);
        match prefix {
            b'$' => {
                let mut bytes = vec![0; count.unwrap() + 2];
                reader.read_exact(&mut bytes)?;
                output.extend(bytes);
            }
            b'*' => {
                for _ in 0..count.unwrap() {
                    read(reader, output, depth + 1)?;
                }
            }
            b'+' | b'-' | b':' => {}
            _ => return Err(std::io::Error::other("fixture RESP2")),
        }
        if output.len() > 65536 {
            return Err(std::io::Error::other("fixture aggregate bound"));
        }
        Ok(())
    }
    let mut output = Vec::new();
    read(reader, &mut output, 0)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::VecDeque, io::Cursor};

    struct Reader {
        bytes: Cursor<Vec<u8>>,
        before: VecDeque<std::io::ErrorKind>,
        end: Option<std::io::ErrorKind>,
    }

    impl Reader {
        fn new(bytes: &[u8]) -> Self {
            Self {
                bytes: Cursor::new(bytes.to_vec()),
                before: VecDeque::new(),
                end: None,
            }
        }
    }

    impl Read for Reader {
        fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
            if let Some(error) = self.before.pop_front() {
                return Err(error.into());
            }
            if self.bytes.position() == self.bytes.get_ref().len() as u64
                && let Some(error) = self.end
            {
                return Err(error.into());
            }
            self.bytes.read(bytes)
        }
    }

    impl BufRead for Reader {
        fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
            if let Some(error) = self.before.pop_front() {
                return Err(error.into());
            }
            self.bytes.fill_buf()
        }

        fn consume(&mut self, count: usize) {
            self.bytes.consume(count);
        }
    }

    #[test]
    fn idle_read_observations_do_not_disconnect_or_consume_the_next_request() {
        let mut reader = Reader::new(b"*1\r\n$4\r\nPING\r\n");
        reader.before.extend([
            std::io::ErrorKind::TimedOut,
            std::io::ErrorKind::WouldBlock,
            std::io::ErrorKind::Interrupted,
        ]);
        let stop = AtomicBool::new(false);
        assert_eq!(
            next_request(&mut reader, &stop).unwrap(),
            Some(b"*1\r\n$4\r\nPING\r\n".to_vec())
        );
        assert!(reader.before.is_empty());
        assert_eq!(next_request(&mut reader, &stop).unwrap(), None);
    }

    #[test]
    fn partial_frame_timeout_and_truncation_remain_real_delivery_failures() {
        for end in [
            None,
            Some(std::io::ErrorKind::TimedOut),
            Some(std::io::ErrorKind::WouldBlock),
        ] {
            let mut reader = Reader::new(b"$3\r\na");
            reader.end = end;
            assert_eq!(
                next_request(&mut reader, &AtomicBool::new(false))
                    .unwrap_err()
                    .kind(),
                end.unwrap_or(std::io::ErrorKind::UnexpectedEof)
            );
        }
    }

    #[test]
    fn connection_reset_is_not_idle_and_owner_stop_does_not_read_again() {
        let mut reader = Reader::new(b"+OK\r\n");
        reader.before.push_back(std::io::ErrorKind::ConnectionReset);
        assert_eq!(
            next_request(&mut reader, &AtomicBool::new(true)).unwrap(),
            None
        );
        assert_eq!(reader.before.len(), 1);
        assert_eq!(
            next_request(&mut reader, &AtomicBool::new(false))
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::ConnectionReset
        );
    }

    #[test]
    fn original_scalar_and_aggregate_frame_limits_are_not_relaxed() {
        assert_eq!(
            frame(&mut Cursor::new(b"$32768\r\n"))
                .unwrap_err()
                .to_string(),
            "fixture frame bound"
        );
        let body = "x".repeat(32767);
        let wire = format!("*3\r\n$32767\r\n{body}\r\n$32767\r\n{body}\r\n$32767\r\n{body}\r\n");
        assert_eq!(
            frame(&mut Cursor::new(wire)).unwrap_err().to_string(),
            "fixture aggregate bound"
        );
    }
}
