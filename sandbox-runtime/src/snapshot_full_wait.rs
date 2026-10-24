//! The original Full owner waits on one connection; no request redispatch.
use super::RuntimeProcessIdentity;
use std::{io::{self, Read, Write}, os::unix::net::UnixStream};

pub(super) struct OriginalFullStream<'a> {
    pub stream: UnixStream,
    pub source: &'a RuntimeProcessIdentity,
}
impl OriginalFullStream<'_> {
    fn retry(&self, error: &io::Error) -> io::Result<bool> {
        if !matches!(error.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) {
            return Ok(false);
        }
        self.source.is_alive().map_err(io::Error::other)
    }
}
impl Read for OriginalFullStream<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        loop {
            match self.stream.read(bytes) {
                Err(error) if self.retry(&error)? => {},
                result => return result,
            }
        }
    }
}
impl Write for OriginalFullStream<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        loop {
            match self.stream.write(bytes) {
                Err(error) if self.retry(&error)? => {},
                result => return result,
            }
        }
    }
    fn flush(&mut self) -> io::Result<()> { self.stream.flush() }
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::{thread, time::Duration};
    #[test]
    fn same_parser_keeps_partial_response_across_socket_timeouts() {
        let (client, mut server) = UnixStream::pair().unwrap();
        client.set_read_timeout(Some(Duration::from_millis(10))).unwrap();
        server.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
        let writer = thread::spawn(move || {
            server.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 3\r\n\r\nb").unwrap();
            // The controlled peer receives no additional request while it holds
            // the remaining response, forcing the original reader through polls.
            let mut byte = [0];
            assert!(matches!(server.read(&mut byte), Err(ref e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut)));
            server.write_all(b"ad").unwrap();
        });
        let source = RuntimeProcessIdentity::capture(std::process::id()).unwrap();
        let mut original = OriginalFullStream { stream: client, source: &source };
        assert_eq!(super::super::super::api_response::read_reply(&mut original).unwrap(), (400, b"bad".to_vec()));
        writer.join().unwrap();
    }
    #[test]
    fn eof_stays_unknown_instead_of_becoming_api_end() {
        let (client, server) = UnixStream::pair().unwrap();
        drop(server);
        let source = RuntimeProcessIdentity::capture(std::process::id()).unwrap();
        let mut original = OriginalFullStream { stream: client, source: &source };
        assert!(super::super::super::api_response::read_reply(&mut original).is_err());
    }
}
