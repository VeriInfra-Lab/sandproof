//! Source-read window accounting shared with the existing MIME parser.
//! The parser keeps header/diagnostic bytes under its original budgets; these
//! cursors separately model bytes consumed by Go's fixed bufio.Reader.
use super::{Decoder, Error, PartReadEnd, Phase, Source};
const READ_BYTES: usize = 4096;
#[derive(Default)]
pub(super) struct ReadBuffer {
    cursor: usize,
    end: usize,
    pub(super) remaining: Option<usize>,
}
impl Decoder {
    /// Size of the next actual source read. Only call after `next_event` returned
    /// None. A source may return fewer bytes, but fragments of one read must be
    /// supplied without running the parser between them.
    /// # Errors
    /// Legacy decoder, misuse, failed/ended input or an unrepresentable lookahead.
    pub fn source_read_capacity(&self) -> Result<usize, Error> {
        let read = self
            .read_buffer
            .as_ref()
            .ok_or(Error::InvalidConfiguration)?;
        if self.phase == Phase::Failed {
            return Err(Error::Poisoned);
        }
        if self.source != Source::Open || !self.wants_input || read.remaining.is_some() {
            return Err(Error::InputPending);
        }
        READ_BYTES
            .checked_sub(read.end - read.cursor)
            .filter(|n| *n != 0)
            .ok_or(Error::ResourceExhausted)
    }
    /// Feed a fragment of one actual source read, of at most `chunk_bytes`.
    /// `complete` marks the last fragment of that read, not transport EOF. The
    /// total must not exceed `source_read_capacity`; no event may be consumed
    /// until the read is complete. A failure poisons this decoder.
    /// # Errors
    /// Invalid sequencing, input after EOF, or original input/chunk budgets.
    pub fn feed_read(&mut self, bytes: &[u8], complete: bool) -> Result<(), Error> {
        let result = self.feed_read_inner(bytes, complete);
        if result.is_err() {
            self.phase = Phase::Failed;
        }
        result
    }
    fn feed_read_inner(&mut self, bytes: &[u8], complete: bool) -> Result<(), Error> {
        let read = self
            .read_buffer
            .as_ref()
            .ok_or(Error::InvalidConfiguration)?;
        if self.phase == Phase::Failed {
            return Err(Error::Poisoned);
        }
        if self.source != Source::Open || !self.wants_input {
            return Err(Error::InputPending);
        }
        if bytes.is_empty()
            || bytes.len() > self.limits.chunk_bytes
            || bytes.len() as u64 > self.limits.total_bytes - self.total
        {
            return Err(Error::ResourceExhausted);
        }
        let new_read = read.remaining.is_none();
        let remaining = if let Some(remaining) = read.remaining {
            remaining
        } else {
            self.source_read_capacity()?
        };
        if bytes.len() > remaining {
            return Err(Error::ResourceExhausted);
        }
        let read = self.read_buffer.as_mut().unwrap();
        if new_read {
            self.buffer.drain(..self.offset);
            read.cursor -= self.offset;
            read.end -= self.offset;
            self.offset = 0;
        }
        // capacity is fixed by create; staging never grows it or bypasses the
        // separate header/line budgets, checked before semantic consumption.
        if bytes.len() > self.limits.capacity()? + READ_BYTES - self.buffer.len() {
            return Err(Error::ResourceExhausted);
        }
        self.total += bytes.len() as u64;
        if self.phase != Phase::End {
            self.buffer.extend_from_slice(bytes);
        }
        read.remaining = if complete {
            None
        } else {
            Some(remaining - bytes.len())
        };
        if complete {
            read.end = self.buffer.len();
            self.wants_input = false;
        }
        Ok(())
    }
    /// Valid immediately after a Data event: whether its final raw bytes carried
    /// a part EOF/failure in that read. Artificial event fragments do not move
    /// the terminal observation earlier.
    #[must_use]
    pub fn data_read_end(&self) -> PartReadEnd {
        self.data_read_end
    }

    pub(super) fn prepare_read(&mut self) {
        let Some(read) = &mut self.read_buffer else {
            return;
        };
        match self.phase {
            Phase::Headers => {
                read.cursor = read.cursor.max(self.offset);
                // ReadLine consumes physical lines, including continuation
                // lines, before the complete semantic Part is available.
                if let Some(n) = self.buffer[self.offset..].iter().rposition(|b| *b == b'\n') {
                    let line_start = self.offset + n + 1;
                    // textproto.skipSpace consumes continuation indentation
                    // before its next ReadLine, including across read windows.
                    let spaces = self.buffer[line_start..]
                        .iter()
                        .take_while(|b| matches!(b, b' ' | b'\t'))
                        .count();
                    read.cursor = read.cursor.max(line_start + spaces);
                }
                // A full ReadLine fragment is consumed to make room. bufio's
                // special trailing CR is unread until the following fragment.
                if read.end - read.cursor == READ_BYTES {
                    read.cursor = read.end - usize::from(self.buffer.last() == Some(&b'\r'));
                }
            }
            Phase::End => read.cursor = read.end,
            _ => read.cursor = self.offset,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::multipart::{Event, FramingError, Limits, ReadError};
    fn limits() -> Limits {
        Limits {
            chunk_bytes: 7,
            boundary_bytes: 64,
            header_bytes: 16384,
            header_count: 32,
            line_bytes: 4096,
            parts: 32,
            total_bytes: 1 << 20,
        }
    }
    fn feed_read(decoder: &mut Decoder, bytes: &[u8]) {
        let chunks = bytes.chunks(7);
        let length = chunks.len();
        for (index, chunk) in chunks.enumerate() {
            decoder.feed_read(chunk, index + 1 == length).unwrap();
        }
    }
    #[test]
    fn staged_source_reads_enforce_order_and_every_input_budget() {
        let mut decoder = Decoder::new_read_buffered("B", limits()).unwrap();
        assert_eq!(decoder.source_read_capacity(), Ok(4096));
        decoder.feed_read(b"--", false).unwrap();
        assert_eq!(decoder.source_read_capacity(), Err(Error::InputPending));
        assert_eq!(
            decoder.next_event_detailed(),
            Err(ReadError::Decode(Error::InputPending))
        );
        assert_eq!(decoder.feed_read(b"B\r\n", true), Err(Error::Poisoned));
        assert_eq!(
            decoder.next_event_detailed(),
            Err(ReadError::Decode(Error::Poisoned))
        );
        let mut small = limits();
        small.total_bytes = 3;
        let mut decoder = Decoder::new_read_buffered("B", small).unwrap();
        decoder.feed_read(b"abc", false).unwrap();
        assert_eq!(decoder.feed_read(b"d", true), Err(Error::ResourceExhausted));
        let mut decoder = Decoder::new_read_buffered("B", limits()).unwrap();
        assert_eq!(
            decoder.feed_read(b"oversized", true),
            Err(Error::ResourceExhausted)
        );
        let mut decoder = Decoder::new_read_buffered("B", limits()).unwrap();
        for chunk in vec![b'x'; 4096].chunks(7) {
            decoder.feed_read(chunk, false).unwrap();
        }
        assert_eq!(decoder.feed_read(b"x", true), Err(Error::ResourceExhausted));
    }
    #[test]
    fn full_physical_header_fragment_retains_cr_for_the_next_read() {
        let mut decoder = Decoder::new_read_buffered("B", limits()).unwrap();
        let prefix = [b"--B\r\nX:".as_slice(), &vec![b'x'; 4093], b"\r"].concat();
        feed_read(&mut decoder, &prefix[..4096]);
        assert_eq!(decoder.next_event().unwrap(), None);
        assert_eq!(decoder.source_read_capacity(), Ok(5));
        feed_read(&mut decoder, &prefix[4096..]);
        assert_eq!(decoder.next_event().unwrap(), None);
        assert_eq!(decoder.source_read_capacity(), Ok(4095));
        feed_read(
            &mut decoder,
            b"\nContent-Disposition: form-data; name=\"file\"\r\n\r\nabc\r\n--B--\r\n",
        );
        assert!(matches!(
            decoder.next_event().unwrap(),
            Some(Event::Part(_))
        ));
        assert_eq!(
            decoder.next_event().unwrap(),
            Some(Event::Data(b"abc".to_vec()))
        );
        assert_eq!(decoder.data_read_end(), PartReadEnd::Eof);
        assert_eq!(decoder.next_event().unwrap(), Some(Event::PartEnd));
        assert_eq!(decoder.next_event().unwrap(), Some(Event::End));
    }
    #[test]
    fn full_next_part_line_errors_without_reading_again_and_end_is_not_early() {
        let mut decoder = Decoder::new_read_buffered("B", limits()).unwrap();
        let capacity = decoder.buffer.capacity();
        feed_read(&mut decoder, &vec![b'x'; 4096]);
        assert_eq!(
            decoder.next_event_detailed(),
            Err(ReadError::Framing(FramingError::ReadBufferFull))
        );
        assert_eq!(decoder.buffer.capacity(), capacity);
        let mut decoder = Decoder::new_read_buffered("B", limits()).unwrap();
        feed_read(&mut decoder, b"--B\r\n\r\nabcdefgh\r\n--B--\r\n");
        assert!(matches!(
            decoder.next_event().unwrap(),
            Some(Event::Part(_))
        ));
        assert_eq!(
            decoder.next_event().unwrap(),
            Some(Event::Data(b"abcdefg".to_vec()))
        );
        assert_eq!(decoder.data_read_end(), PartReadEnd::More);
        assert_eq!(
            decoder.next_event().unwrap(),
            Some(Event::Data(b"h".to_vec()))
        );
        assert_eq!(decoder.data_read_end(), PartReadEnd::Eof);
        assert_eq!(decoder.next_event().unwrap(), Some(Event::PartEnd));
        assert_eq!(decoder.next_event().unwrap(), Some(Event::End));
        assert!(decoder.next_event().unwrap().is_none());
        assert_eq!(decoder.source_read_capacity(), Ok(4096));
        feed_read(&mut decoder, b"tail");
        decoder.end_input();
        assert!(decoder.next_event().unwrap().is_none());
    }

    #[test]
    fn source_failure_drains_ambiguous_bytes_before_reporting_the_actual_stage() {
        use crate::multipart::SourceStage;
        for (tail, expected, complete, stage) in [
            (
                b"\r\n--B".as_slice(),
                b"".as_slice(),
                true,
                SourceStage::NextPart,
            ),
            (b"\r\n--B-", b"\r\n--B-", false, SourceStage::Body),
            (b"\r\n--", b"", false, SourceStage::Body),
            (b"--B", b"", true, SourceStage::NextPart),
            (b"--B-", b"--B-", false, SourceStage::Body),
        ] {
            let mut decoder = Decoder::new_read_buffered("B", limits()).unwrap();
            feed_read(&mut decoder, &[b"--B\r\n\r\n".as_slice(), tail].concat());
            assert!(matches!(
                decoder.next_event().unwrap(),
                Some(Event::Part(_))
            ));
            assert_eq!(decoder.next_event().unwrap(), None);
            decoder.fail_source().unwrap();
            assert_eq!(decoder.source_read_capacity(), Err(Error::InputPending));
            let mut output = Vec::new();
            let mut finished = false;
            loop {
                match decoder.next_event_detailed() {
                    Ok(Some(Event::Data(bytes))) => {
                        output.extend(bytes);
                        assert_eq!(
                            decoder.data_read_end(),
                            if output.len() == expected.len() {
                                PartReadEnd::Failed(Error::Truncated)
                            } else {
                                PartReadEnd::More
                            }
                        );
                    }
                    Ok(Some(Event::PartEnd)) => finished = true,
                    Err(error) => {
                        assert_eq!(error, ReadError::Source(stage));
                        break;
                    }
                    other => panic!("unexpected {other:?}"),
                }
            }
            assert_eq!(output, expected);
            assert_eq!(finished, complete);
            assert_eq!(decoder.next_event(), Err(Error::Poisoned));
            assert_eq!(decoder.feed(b"tail"), Err(Error::Poisoned));
        }
    }

    #[test]
    fn source_failure_is_not_eof_and_rejects_invalid_order() {
        use crate::multipart::SourceStage;
        for (bytes, stage) in [
            (b"--B--".as_slice(), SourceStage::NextPart),
            (b"--B\r\nX: valid", SourceStage::Headers),
            (b"--B--\r\n", SourceStage::Complete),
        ] {
            let mut decoder = Decoder::new_read_buffered("B", limits()).unwrap();
            feed_read(&mut decoder, bytes);
            while decoder.next_event().unwrap().is_some() {}
            decoder.fail_source().unwrap();
            decoder.end_input(); // Must not overwrite the recorded failure.
            assert_eq!(decoder.next_event_detailed(), Err(ReadError::Source(stage)));
        }
        for staged in [false, true] {
            let mut decoder = Decoder::new_read_buffered("B", limits()).unwrap();
            decoder.feed_read(b"--", !staged).unwrap();
            assert_eq!(decoder.fail_source(), Err(Error::InputPending));
            assert_eq!(decoder.fail_source(), Err(Error::Poisoned));
        }
        let mut decoder = Decoder::new_read_buffered("B", limits()).unwrap();
        assert_eq!(decoder.next_event(), Ok(None));
        decoder.fail_source().unwrap();
        assert_eq!(decoder.feed(b"tail"), Err(Error::InputPending));
        assert_eq!(decoder.next_event(), Err(Error::Poisoned));
    }
}
