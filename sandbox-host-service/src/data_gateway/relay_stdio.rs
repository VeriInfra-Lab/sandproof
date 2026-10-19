//! Cancelable Linux pipe registration for the sole namespace relay entry.
use std::{io, os::fd::OwnedFd};
use tokio::net::unix::pipe::{Receiver, Sender};

pub(super) fn pipes(input: OwnedFd, output: OwnedFd) -> io::Result<(Receiver, Sender)> {
    // Safe Tokio constructors validate pipe type and access before enabling
    // O_NONBLOCK. No regular-file, terminal or blocking-thread fallback.
    Ok((
        Receiver::from_owned_fd(input)?,
        Sender::from_owned_fd(output)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, os::fd::AsFd, sync::mpsc, time::Duration};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn guest_eof_allows_the_real_pipe_relay_runtime_to_exit_while_upload_stays_open() {
        let (input, mut still_open) = io::pipe().unwrap();
        let (client, output) = io::pipe().unwrap();
        still_open.write_all(b"head").unwrap();
        let (finished, result) = mpsc::channel();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async {
                let input_fd = input.as_fd().try_clone_to_owned().unwrap();
                let output_fd = output.as_fd().try_clone_to_owned().unwrap();
                drop(input);
                drop(output);
                let (input, output) = pipes(input_fd, output_fd).unwrap();
                let mut client =
                    Receiver::from_owned_fd(client.as_fd().try_clone_to_owned().unwrap()).unwrap();
                let (stream, mut guest) = tokio::io::duplex(23);
                let (reader, writer) = tokio::io::split(stream);
                let relay = sandproof_data_transport::tunnel::relay(input, output, reader, writer);
                let server = async {
                    let mut head = [0; 4];
                    guest.read_exact(&mut head).await.unwrap();
                    assert_eq!(&head, b"head");
                    guest.shutdown().await.unwrap();
                };
                let client = async {
                    let mut response = Vec::new();
                    client.read_to_end(&mut response).await.unwrap();
                    assert!(response.is_empty());
                };
                tokio::time::timeout(Duration::from_secs(5), async {
                    let (result, (), ()) = tokio::join!(relay, server, client);
                    result.unwrap();
                })
                .await
                .unwrap();
            });
            // No shutdown_timeout: an uninterruptible stdin read must not be
            // hidden by abandoning threads at a new timeout boundary.
            drop(runtime);
            finished.send(()).unwrap();
        });
        result
            .recv_timeout(Duration::from_secs(10))
            .expect("relay runtime kept a blocked input reader");
        drop(still_open);
    }

    #[tokio::test]
    async fn non_pipe_fds_are_refused_without_a_blocking_fallback() {
        let ordinary = tempfile::tempfile().unwrap();
        let (_, output) = io::pipe().unwrap();
        assert_eq!(
            pipes(ordinary.into(), output.into()).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }
}
