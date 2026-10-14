use super::*;
use std::os::unix::fs::PermissionsExt;

#[tokio::test]
async fn wrapper_waits_for_release_uses_daemon_helpers_and_preserves_pid_and_literal_arguments() {
    let directory = tempfile::tempdir().unwrap();
    let helpers = directory.path().join("helpers");
    std::fs::create_dir(&helpers).unwrap();
    for (name, script) in [
        (
            "ionice",
            "#!/bin/sh\nprintf 'io:%s:%s:%s:%s\\n' \"$1\" \"$2\" \"$3\" \"$4\" >> helper_trace\nshift 4\nexec \"$@\"\n",
        ),
        (
            "nice",
            "#!/bin/sh\nprintf 'nice:%s:%s\\n' \"$1\" \"$2\" >> helper_trace\nshift 2\nexec \"$@\"\n",
        ),
    ] {
        let path = helpers.join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let mut spec = shell(
        directory.path(),
        r#"printf once >> effect; printf '%s\n' "$$"; /bin/cat /proc/$$/oom_score_adj; printf '<%s>\n' "$@""#,
        false,
    );
    spec.envd_wrapper_path = Some(helpers.to_str().unwrap().into());
    spec.env
        .insert("PATH".into(), "/not-the-daemon-path".into());
    spec.args
        .extend(["--", "a b", "$(touch injected)", "", "*"].map(str::to_owned));
    let delta = -rustix::process::getpriority_process(None).unwrap_or(0);
    let mut process = prepare(directory.path(), &spec, limits()).await;
    assert!(!directory.path().join("helper_trace").exists());
    assert!(!directory.path().join("effect").exists());
    let pid = process.identity().pid;
    assert_eq!(
        process.release().await.unwrap(),
        LaunchObservation::NoExecErrorObserved
    );
    let (stdout, stderr, code) = collect(&mut process).await;
    assert_eq!(code, 0);
    assert_eq!(stderr, b"");
    assert_eq!(
        String::from_utf8(stdout).unwrap(),
        format!("{pid}\n100\n<a b>\n<$(touch injected)>\n<>\n<*>\n")
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("helper_trace")).unwrap(),
        format!("io:-c:2:-n:4\nnice:-n:{delta}\n")
    );
    assert_eq!(
        std::fs::read(directory.path().join("effect")).unwrap(),
        b"once"
    );
    assert!(!directory.path().join("injected").exists());
}

#[tokio::test]
async fn wrapper_missing_user_program_is_a_real_exit_after_shell_exec() {
    let directory = tempfile::tempdir().unwrap();
    let mut spec = shell(directory.path(), "", false);
    spec.envd_wrapper_path = Some(String::new()); // Missing optional helpers are normal.
    spec.program = directory
        .path()
        .join("missing-program")
        .to_str()
        .unwrap()
        .into();
    spec.args.clear();
    let mut process = prepare(directory.path(), &spec, limits()).await;
    assert_eq!(
        process.release().await.unwrap(),
        LaunchObservation::NoExecErrorObserved
    );
    let (stdout, stderr, code) = collect(&mut process).await;
    assert_eq!(stdout, b"");
    assert!(!stderr.is_empty());
    assert_eq!(code, 127);
}

#[tokio::test]
async fn wrapped_launch_budget_refuses_before_ready_and_reaps_original_worker() {
    let directory = tempfile::tempdir().unwrap();
    let mut spec = shell(directory.path(), "printf forbidden > effect", false);
    spec.envd_wrapper_path = Some(String::new());
    let mut limits = limits();
    limits.launch_bytes = serde_json::to_vec(&spec).unwrap().len();
    let mut preparation = sandproof_guest_runtime::CommandPreparation::spawn(
        Path::new(WORKER),
        directory.path(),
        1,
        1,
        &spec,
        limits,
    )
    .unwrap();
    assert!(preparation.finish().await.is_err());
    let status = preparation.abort().await.unwrap();
    assert!(!status.success());
    drop(preparation);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}
