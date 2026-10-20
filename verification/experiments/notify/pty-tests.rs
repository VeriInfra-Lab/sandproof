// Appended only to the unchanged production module in a private test copy.
#[cfg(test)]
mod notify_contract_trial {
    use super::*;
    use std::future::{Future, poll_fn};
    use std::task::Poll;

    #[tokio::test]
    async fn close_notifies_every_enabled_original_waiter() {
        let (master, slave) = PtyMaster::open(PtySize::default()).unwrap();
        let other = Arc::clone(&master);
        let mut attributes = rustix::termios::tcgetattr(&slave).unwrap();
        attributes.make_raw();
        rustix::termios::tcsetattr(&slave, rustix::termios::OptionalActions::Now, &attributes)
            .unwrap();
        let mut input = [0u8; 8];
        let mut read = Box::pin(master.read(&mut input));
        assert!(
            poll_fn(|cx| Poll::Ready(read.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        let payload = vec![b'x'; 1024 * 1024];
        let mut write = Box::pin(other.write_all(&payload));
        assert!(
            poll_fn(|cx| Poll::Ready(write.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        master.close();
        let read_result = poll_fn(|cx| Poll::Ready(read.as_mut().poll(cx))).await;
        assert!(
            matches!(read_result, Poll::Ready(Err(ref e)) if e.kind() == io::ErrorKind::BrokenPipe),
            "close left an enabled original read waiter unnotified: {read_result:?}"
        );
        let write_result = poll_fn(|cx| Poll::Ready(write.as_mut().poll(cx))).await;
        assert!(
            matches!(write_result, Poll::Ready(Err(ref e)) if e.kind() == io::ErrorKind::BrokenPipe),
            "close left an enabled original write waiter unnotified: {write_result:?}"
        );
        drop(slave);
    }

    #[tokio::test]
    async fn closed_descriptor_rejects_late_read_and_write() {
        let (master, slave) = PtyMaster::open(PtySize::default()).unwrap();
        master.close();
        master.close();
        let mut input = [0u8; 8];
        assert_eq!(
            master.read(&mut input).await.unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        assert_eq!(
            master.write_all(b"x").await.unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        drop(slave);
    }

    #[tokio::test]
    async fn close_only_notifies_the_original_pty() {
        let (first, first_slave) = PtyMaster::open(PtySize::default()).unwrap();
        let (other, other_slave) = PtyMaster::open(PtySize::default()).unwrap();
        let mut first_input = [0u8; 8];
        let mut other_input = [0u8; 8];
        let mut first_read = Box::pin(first.read(&mut first_input));
        let mut other_read = Box::pin(other.read(&mut other_input));
        assert!(
            poll_fn(|cx| Poll::Ready(first_read.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        assert!(
            poll_fn(|cx| Poll::Ready(other_read.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        first.close();
        assert!(
            matches!(poll_fn(|cx| Poll::Ready(first_read.as_mut().poll(cx))).await,
            Poll::Ready(Err(ref e)) if e.kind() == io::ErrorKind::BrokenPipe)
        );
        assert!(
            poll_fn(|cx| Poll::Ready(other_read.as_mut().poll(cx)))
                .await
                .is_pending()
        );
        other.close();
        assert!(
            matches!(poll_fn(|cx| Poll::Ready(other_read.as_mut().poll(cx))).await,
            Poll::Ready(Err(ref e)) if e.kind() == io::ErrorKind::BrokenPipe)
        );
        drop((first_slave, other_slave));
    }
}
