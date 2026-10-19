use super::*;

#[tokio::test]
async fn raw_launch_contexts_still_reject_nul_before_spawning_a_worker() {
    for case in 0..5 {
        let directory = tempfile::tempdir().unwrap();
        let mut spec = shell(directory.path(), "printf forbidden > effect", false);
        match case {
            0 => spec.program.push('\0'),
            1 => spec.args[0].push('\0'),
            2 => spec.cwd.push("bad\0cwd"),
            3 => {
                spec.env.insert("A\0B".into(), "value".into());
            }
            4 => {
                spec.env.insert("A".into(), "bad\0value".into());
            }
            _ => unreachable!(),
        }
        let result = sandproof_guest_runtime::CommandPreparation::spawn(
            Path::new(WORKER),
            directory.path(),
            1,
            1,
            &spec,
            limits(),
        );
        assert!(matches!(result, Err(error) if error.kind() == std::io::ErrorKind::InvalidInput));
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}
