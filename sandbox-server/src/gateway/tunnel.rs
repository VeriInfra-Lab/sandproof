//! Bounded byte transport for the trusted namespace relay.

use std::io;

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

const BUFFER_BYTES: usize = 64 * 1024;

pub(super) async fn relay<I, O, R, W>(
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
    // they never buffer an entire request or response.
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
    // The relay has TCP half-close semantics. Upload EOF only closes
    // the guest's write half; it is not a revocation or a cleanup certificate.
    // A guest may stop accepting an upload before returning its rejection
    // (for example, HTTP 507). Preserve that response even if the request
    // copy failed. Response errors still cancel the upload immediately.
    // Revocation/disconnect remains the supervising gateway's responsibility:
    // it terminates/reaps the exact child rather than treating EOF as cleanup.
    tokio::pin!(upstream, downstream);
    tokio::select! {
        result = &mut upstream => {
            downstream.await?;
            result
        }
        result = &mut downstream => {
            result?;
            upstream.await
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io,
        pin::Pin,
        task::{Context, Poll},
        time::Duration,
    };

    use tokio::{
        io::{AsyncReadExt, AsyncWrite, AsyncWriteExt},
        sync::oneshot,
    };

    use super::relay;

    struct RefusedUpload(Option<oneshot::Sender<()>>);

    impl AsyncWrite for RefusedUpload {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &[u8],
        ) -> Poll<io::Result<usize>> {
            if let Some(refused) = self.0.take() {
                refused.send(()).unwrap();
            }
            Poll::Ready(Err(io::Error::from(io::ErrorKind::BrokenPipe)))
        }

        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn upload_write_failure_preserves_the_guests_rejection_response() {
        tokio::time::timeout(Duration::from_secs(5), async {
            let (mut client_input, input) = tokio::io::duplex(17);
            let (output, mut client_output) = tokio::io::duplex(19);
            let (mut guest, reader) = tokio::io::duplex(23);
            let (refused, rejection_ready) = oneshot::channel();
            let relay = relay(input, output, reader, RefusedUpload(Some(refused)));
            let client = async {
                client_input.write_all(b"upload").await.unwrap();
                let mut response = Vec::new();
                client_output.read_to_end(&mut response).await.unwrap();
                response
            };
            let response = [
                b"HTTP/1.1 507 Insufficient Storage\r\n\r\n".as_slice(),
                &vec![b'X'; 256 * 1024 + 1],
            ]
            .concat();
            let expected = response.clone();
            let server = async move {
                rejection_ready.await.unwrap();
                // The write failure is observed first. The response must
                // still flow, with backpressure and no upload EOF needed.
                guest.write_all(&response).await
            };
            let (result, received, sent) = tokio::join!(relay, client, server);
            assert!(
                received == expected,
                "upload error discarded the guest response ({} of {} bytes)",
                received.len(),
                expected.len()
            );
            sent.unwrap();
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
        })
        .await
        .expect("rejected upload stalled");
    }

    #[tokio::test]
    async fn forwards_before_upload_eof_and_keeps_response_after_half_close() {
        half_close_response().await;
    }

    async fn half_close_response() {
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
                tokio::join!(relay(input, output, reader, writer), client, server);
            result.unwrap();
        })
        .await
        .expect("stream stalled");
    }

    #[tokio::test]
    async fn downstream_error_cancels_an_upload_that_has_no_eof() {
        downstream_error().await;
    }

    async fn downstream_error() {
        let (_open_input, input) = tokio::io::duplex(17);
        let (output, closed_reader) = tokio::io::duplex(19);
        drop(closed_reader);
        let (stream, mut guest) = tokio::io::duplex(23);
        let (reader, writer) = tokio::io::split(stream);
        guest.write_all(b"response").await.unwrap();
        let error =
            tokio::time::timeout(Duration::from_secs(5), relay(input, output, reader, writer))
                .await
                .expect("failed response waited for upload EOF")
                .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }

    #[tokio::test]
    async fn upload_eof_waits_for_explicitly_delayed_response_() {
        tokio::time::timeout(Duration::from_secs(5), async {
            let (mut client_input, input) = tokio::io::duplex(17);
            let (output, mut client_output) = tokio::io::duplex(19);
            let (stream, mut guest) = tokio::io::duplex(23);
            let (reader, writer) = tokio::io::split(stream);
            let task = tokio::spawn(relay(input, output, reader, writer));
            client_input.write_all(b"request").await.unwrap();
            client_input.shutdown().await.unwrap();
            let mut request = Vec::new();
            guest.read_to_end(&mut request).await.unwrap();
            assert_eq!(request, b"request");
            // The guest has observed FIN, but deliberately has not replied.
            // No sleep: observing FIN is the scheduling barrier.
            tokio::task::yield_now().await;
            assert!(!task.is_finished(), "upload EOF cancelled the response");
            let expected = vec![0x42; 256 * 1024 + 3];
            let send = async {
                guest.write_all(&expected).await.unwrap();
                guest.shutdown().await.unwrap();
            };
            let receive = async {
                let mut bytes = Vec::new();
                client_output.read_to_end(&mut bytes).await.unwrap();
                assert_eq!(bytes, expected);
            };
            tokio::join!(send, receive);
            task.await.unwrap().unwrap();
        })
        .await
        .expect("delayed half-close response stalled");
    }
}
