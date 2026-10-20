#[cfg(test)]
mod std_mutex_pty_trial {
    use super::*;
    #[tokio::test]
    async fn original_descriptor_refuses_poison_and_close_uses_the_original_cell() {
        let (master, slave) = PtyMaster::open(PtySize { rows: 24, cols: 80 }).unwrap();
        let original = master.descriptor().unwrap();
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = master.descriptor.lock().unwrap();
            panic!("controlled original descriptor unwind");
        }));
        assert!(failed.is_err());
        let error = master.descriptor().err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(error.to_string(), "terminal owner poisoned");
        master.close();
        let recovered = master.descriptor.lock().err().unwrap().into_inner();
        assert!(recovered.is_none());
        drop(recovered);
        assert!(fcntl_getfl(original.get_ref()).is_ok());
        drop(original);
        drop(slave);
    }
}
