//! Controlled delivery faults around the real Redis engine, never a fake store.
use crate::engine::connect;
use std::{
    io::{BufReader, Read, Write},
    net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};
// Controlled proxy changes only delivery of an actual Redis reply. It neither
// implements nor mocks the authority store. Frames/threads have fixed budgets.
pub(crate) struct ReplyLoss {
    pub(crate) address: SocketAddrV4,
    pub(crate) armed: Arc<AtomicU8>,
    skip_writes: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl ReplyLoss {
    pub(crate) fn new(target: SocketAddrV4) -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, listener.local_addr().unwrap().port());
        let armed = Arc::new(AtomicU8::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let skip_writes = Arc::new(AtomicUsize::new(0));
        let worker_skip = Arc::clone(&skip_writes);
        let worker_arm = Arc::clone(&armed);
        let worker_stop = Arc::clone(&stop);
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
                while let Ok(request) = frame(&mut client) {
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
                    if upstream.get_mut().write_all(&request).is_err() {
                        break;
                    }
                    let Ok(reply) = frame(&mut upstream) else {
                        break;
                    };
                    if fault == 1 {
                        assert_eq!(reply, b":0\r\n");
                        break; // commit acknowledged by real engine, not by caller
                    }
                    if client.get_mut().write_all(&reply).is_err() {
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
            worker: Some(worker),
        }
    }
    pub(crate) fn lose_after(&self, preceding_writes: usize) {
        self.skip_writes.store(preceding_writes, Ordering::SeqCst);
        self.armed.store(1, Ordering::SeqCst);
    }
}
impl Drop for ReplyLoss {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect_timeout(&self.address.into(), Duration::from_secs(1));
        self.worker.take().unwrap().join().unwrap();
    }
}
fn frame(reader: &mut BufReader<TcpStream>) -> std::io::Result<Vec<u8>> {
    fn read(
        reader: &mut BufReader<TcpStream>,
        output: &mut Vec<u8>,
        depth: usize,
    ) -> std::io::Result<()> {
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
