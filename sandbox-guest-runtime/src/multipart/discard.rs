//! Original Part.Close consumes the decoded reader and ignores its error.
//! On QP failure, its bounded raw prefetch is already consumed; `NextPart` starts
//! at that position, not at an invented successful end of the raw part.
use super::{
    Decoder, Error, Event, FramingError, PartReadEnd, Phase, QuotedPrintable,
    QuotedPrintableReadError, ReadError, SourceStage,
};

pub struct Discard {
    qp: Option<QuotedPrintable>,
    complete: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiscardStatus {
    NeedInput,
    Complete,
}
impl Discard {
    /// Create after receiving the Part to ignore. Decoded output is discarded
    /// under the same chunk and fixed physical-line budgets as file decoding.
    /// # Errors
    /// Invalid QP chunk budget.
    pub fn new(quoted_printable: bool, chunk_bytes: usize) -> Result<Self, Error> {
        Ok(Self {
            qp: if quoted_printable {
                Some(QuotedPrintable::new(chunk_bytes)?)
            } else {
                None
            },
            complete: false,
        })
    }

    /// Run on a read-buffered decoder until Close stops or more input is needed.
    /// Feed the same decoder only after `NeedInput`. Complete means the ignored
    /// reader was closed, not that the raw part ended successfully. Continue
    /// normal decoder events to observe `NextPart` and its possible framing error.
    /// # Errors
    /// Source/framing/decoder failure or invalid sequencing. QP encoding errors
    /// stop Close without being returned as a file-write diagnostic.
    pub fn advance(&mut self, decoder: &mut Decoder) -> Result<DiscardStatus, ReadError> {
        let result = self.advance_inner(decoder);
        if result.is_err() {
            self.complete = true;
            decoder.phase = Phase::Failed;
        }
        result
    }
    fn advance_inner(&mut self, decoder: &mut Decoder) -> Result<DiscardStatus, ReadError> {
        if self.complete || decoder.phase == Phase::Failed {
            return Err(ReadError::Decode(Error::Poisoned));
        }
        if decoder.read_buffer.is_none() {
            return Err(ReadError::Decode(Error::InvalidConfiguration));
        }
        if decoder.phase != Phase::Body {
            return Err(ReadError::Decode(Error::InputPending));
        }
        loop {
            let capacity = self
                .qp
                .as_ref()
                .map_or(usize::MAX, QuotedPrintable::read_capacity);
            let event = match decoder.next_event_read(capacity) {
                Err(ReadError::Framing(FramingError::BodyTruncated)) => {
                    return Err(ReadError::Framing(FramingError::NextPartEof));
                }
                Err(ReadError::Source(SourceStage::Body)) => {
                    return Err(ReadError::Source(SourceStage::NextPart));
                }
                other => other?,
            };
            match event {
                None => return Ok(DiscardStatus::NeedInput),
                Some(Event::Data(bytes)) => {
                    if let Some(qp) = &mut self.qp {
                        let end = decoder.data_read_end();
                        let output = qp.feed_reader(&bytes, end);
                        match output.error {
                            Some(QuotedPrintableReadError::Encoding(_)) => {
                                decoder.abandon_read()?;
                                self.complete = true;
                                return Ok(DiscardStatus::Complete);
                            }
                            // The next decoder call retains the actual EOF or
                            // source cause, wrapped as a NextPart failure above.
                            Some(QuotedPrintableReadError::Decode(Error::Truncated))
                                if end == PartReadEnd::Failed(Error::Truncated) =>
                            {
                                self.qp = None;
                            }
                            Some(QuotedPrintableReadError::Decode(error)) => {
                                return Err(ReadError::Decode(error));
                            }
                            None if end == PartReadEnd::Eof => self.qp = None,
                            None => {}
                        }
                    }
                }
                Some(Event::PartEnd) => {
                    self.complete = true;
                    return Ok(DiscardStatus::Complete);
                }
                _ => return Err(ReadError::Decode(Error::InputPending)),
            }
        }
    }
}
impl Decoder {
    fn abandon_read(&mut self) -> Result<(), ReadError> {
        if self.phase != Phase::Body || self.read_buffer.is_none() {
            self.phase = Phase::Failed;
            return Err(ReadError::Decode(Error::InputPending));
        }
        // These bytes belong to the same raw Read into QP's fixed prefetch
        // buffer. They were already charged by feed, even if no Data event was
        // requested for their artificial fragments before the encoding error.
        self.offset += self.body_read_remaining;
        self.body_read_remaining = 0;
        self.data_read_end = PartReadEnd::More;
        self.phase = Phase::Boundary {
            after_separator: false,
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::multipart::Limits;
    fn limits() -> Limits {
        Limits {
            chunk_bytes: 128,
            boundary_bytes: 64,
            header_bytes: 1024,
            header_count: 16,
            line_bytes: 4096,
            parts: 16,
            total_bytes: 1 << 24,
        }
    }
    fn feed(decoder: &mut Decoder, bytes: &[u8]) {
        let chunks = bytes.chunks(128);
        let count = chunks.len();
        for (i, chunk) in chunks.enumerate() {
            decoder.feed_read(chunk, i + 1 == count).unwrap();
        }
    }
    fn part(decoder: &mut Decoder) {
        assert!(matches!(
            decoder.next_event().unwrap(),
            Some(Event::Part(_))
        ));
    }
    #[test]
    fn ignored_error_consumes_only_its_existing_raw_prefetch() {
        let head = b"--B\r\n\r\n";
        let mut decoder = Decoder::new_read_buffered("B", limits()).unwrap();
        let input = [head.as_slice(), b"bad\0\nextra\n", b"\r\n--B--\r\n"].concat();
        feed(&mut decoder, &input);
        part(&mut decoder);
        let capacity = decoder.buffer.capacity();
        let mut discard = Discard::new(true, 128).unwrap();
        assert_eq!(discard.advance(&mut decoder), Ok(DiscardStatus::Complete));
        assert!(!decoder.is_complete());
        assert_eq!(decoder.next_event().unwrap(), Some(Event::End));
        assert_eq!(decoder.buffer.capacity(), capacity);
        // A larger body is not drained beyond the current source Read.
        let mut decoder = Decoder::new_read_buffered("B", limits()).unwrap();
        let mut input = [head.as_slice(), b"bad\0\n"].concat();
        input.resize(4096, b'x');
        feed(&mut decoder, &input);
        part(&mut decoder);
        let mut discard = Discard::new(true, 128).unwrap();
        assert_eq!(discard.advance(&mut decoder), Ok(DiscardStatus::Complete));
        assert_eq!(decoder.offset, 4096);
        assert_eq!(decoder.next_event().unwrap(), None);
        feed(&mut decoder, b"remaining\n\r\n--B--\r\n");
        assert_eq!(
            decoder.next_event_detailed(),
            Err(ReadError::Framing(FramingError::Boundary {
                line: b"remaining\n".to_vec(),
                after_separator: false,
            }))
        );
    }
    #[test]
    fn discard_does_not_swallow_source_budget_or_order_failures() {
        for failed in [true, false] {
            let mut decoder = Decoder::new_read_buffered("B", limits()).unwrap();
            feed(&mut decoder, b"--B\r\n\r\nunfinished");
            part(&mut decoder);
            let mut discard = Discard::new(true, 128).unwrap();
            assert_eq!(discard.advance(&mut decoder), Ok(DiscardStatus::NeedInput));
            if failed {
                decoder.fail_source().unwrap();
            } else {
                decoder.end_input();
            }
            assert_eq!(
                discard.advance(&mut decoder),
                Err(if failed {
                    ReadError::Source(SourceStage::NextPart)
                } else {
                    ReadError::Framing(FramingError::NextPartEof)
                })
            );
            assert_eq!(
                discard.advance(&mut decoder),
                Err(ReadError::Decode(Error::Poisoned))
            );
            assert_eq!(decoder.next_event(), Err(Error::Poisoned));
        }
        let mut decoder = Decoder::new_read_buffered("B", limits()).unwrap();
        feed(&mut decoder, b"--B\r\n\r\nnormal\r\n--B--\r\n");
        part(&mut decoder);
        let mut discard = Discard::new(true, 1).unwrap();
        assert_eq!(
            discard.advance(&mut decoder),
            Err(ReadError::Decode(Error::ResourceExhausted))
        );
        assert_eq!(decoder.next_event(), Err(Error::Poisoned));
        for buffered in [false, true] {
            let mut decoder = if buffered {
                Decoder::new_read_buffered("B", limits())
            } else {
                Decoder::new("B", limits())
            }
            .unwrap();
            let mut discard = Discard::new(true, 128).unwrap();
            assert_eq!(
                discard.advance(&mut decoder),
                Err(ReadError::Decode(if buffered {
                    Error::InputPending
                } else {
                    Error::InvalidConfiguration
                }))
            );
            assert_eq!(decoder.feed(b"data"), Err(Error::Poisoned));
        }
    }
    #[test]
    fn large_ignored_qp_stream_has_fixed_parser_and_line_capacity() {
        let mut decoder = Decoder::new_read_buffered("B", limits()).unwrap();
        feed(&mut decoder, b"--B\r\n\r\n");
        part(&mut decoder);
        let mut discard = Discard::new(true, 128).unwrap();
        let capacity = decoder.buffer.capacity();
        let block = b"x\n".repeat(2048);
        for _ in 0..1024 {
            assert_eq!(discard.advance(&mut decoder), Ok(DiscardStatus::NeedInput));
            assert_eq!(decoder.source_read_capacity(), Ok(4096));
            feed(&mut decoder, &block);
            assert_eq!(decoder.buffer.capacity(), capacity);
        }
        assert_eq!(discard.advance(&mut decoder), Ok(DiscardStatus::NeedInput));
        feed(&mut decoder, b"\r\n--B--\r\n");
        assert_eq!(discard.advance(&mut decoder), Ok(DiscardStatus::Complete));
        assert_eq!(decoder.next_event().unwrap(), Some(Event::End));
        assert_eq!(decoder.buffer.capacity(), capacity);
        assert_eq!(
            discard.advance(&mut decoder),
            Err(ReadError::Decode(Error::Poisoned))
        );
    }
}
