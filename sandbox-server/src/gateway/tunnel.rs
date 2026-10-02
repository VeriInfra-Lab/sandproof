//! Bounded byte transport for the trusted namespace relay.

use std::io;

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

const BUFFER_BYTES: usize = 64 * 1024;

pub(super) async fn relay<I, O, R, W>(
    _bounded: bool,
    input: I,
    mut output: O,
    reader: R,
    mut writer: W,
) -> io::Result<()>
where
    I: AsyncRead + Unpin,
    O: AsyncWrite + Unpin,
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    // copy_buf forwards each available slice without waiting to fill a buffer.
    // Fixed read buffers amortize stdio's blocking-pool work on large streams;
    // they do not buffer a request/response or alter half-close/error handling.
    let mut input = BufReader::with_capacity(BUFFER_BYTES, input);
    let mut reader = BufReader::with_capacity(BUFFER_BYTES, reader);
    let upstream = async {
        tokio::io::copy_buf(&mut input, &mut writer).await?;
        writer.shutdown().await?;
        Ok::<(), io::Error>(())
    };
    let downstream = async {
        tokio::io::copy_buf(&mut reader, &mut output).await?;
        output.flush().await?;
        Ok::<(), io::Error>(())
    };
    // Both host modes have TCP half-close semantics. Upload EOF only closes
    // the guest's write half; it is not a revocation or a cleanup certificate.
    // The supervising gateway explicitly terminates/reaps a tracked child on
    // revocation/disconnect. A copy error still cancels the opposite direction.
    tokio::try_join!(upstream, downstream)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{io, time::Duration};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::relay;

    #[tokio::test]
    async fn forwards_before_upload_eof_and_keeps_response_after_half_close() {
        for bounded in [false, true] {
            half_close_response(bounded).await;
        }
    }

    async fn half_close_response(bounded: bool) {
        // Tiny duplex queues force partial reads/writes in both directions.
        let (mut client_input, input) = tokio::io::duplex(17);
        let (output, mut client_output) = tokio::io::duplex(19);
        let (stream, mut guest) = tokio::io::duplex(23);
        let (reader, writer) = tokio::io::split(stream);
        let request = vec![0x5a; 192 * 1024 + 7];
        let response = vec![0xa5; 256 * 1024 + 11];
        let client = async {
            client_input.write_all(b"head").await.unwrap();
            let mut ack = [0; 3];
            client_output.read_exact(&mut ack).await.unwrap();
            assert_eq!(&ack, b"ack"); // No EOF and far less than one buffer.
            client_input.write_all(&request).await.unwrap();
            client_input.shutdown().await.unwrap();
            let mut received = Vec::new();
            client_output.read_to_end(&mut received).await.unwrap();
            assert_eq!(received, response);
        };
        let server = async {
            let mut head = [0; 4];
            guest.read_exact(&mut head).await.unwrap();
            assert_eq!(&head, b"head");
            guest.write_all(b"ack").await.unwrap();
            let mut received = Vec::new();
            guest.read_to_end(&mut received).await.unwrap();
            assert_eq!(received, request);
            // The client's write EOF must leave its read side usable.
            guest.write_all(&response).await.unwrap();
            guest.shutdown().await.unwrap();
        };
        tokio::time::timeout(Duration::from_secs(5), async {
            let (result, (), ()) =
                tokio::join!(relay(bounded, input, output, reader, writer), client, server);
            result.unwrap();
        })
        .await
        .expect("stream stalled");
    }

    #[tokio::test]
    async fn downstream_error_cancels_an_upload_that_has_no_eof() {
        for bounded in [false, true] {
            downstream_error(bounded).await;
        }
    }

    async fn downstream_error(bounded: bool) {
        let (_open_input, input) = tokio::io::duplex(17);
        let (output, closed_reader) = tokio::io::duplex(19);
        drop(closed_reader);
        let (stream, mut guest) = tokio::io::duplex(23);
        let (reader, writer) = tokio::io::split(stream);
        guest.write_all(b"response").await.unwrap();
        let error =
            tokio::time::timeout(Duration::from_secs(5), relay(bounded, input, output, reader, writer))
                .await
                .expect("failed response waited for upload EOF")
                .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }
}
