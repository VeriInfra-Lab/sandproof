use sandproof_guest_core::{
    Channel, CommandError, CommandIdentity, CommandOutcome, CommandStream, OutputMode,
    StreamFailure, Transfer, TransferError, TransferOutcome,
};

const ID: CommandIdentity = CommandIdentity {
    session: 7,
    command: 11,
    pid: 4321,
    start_ticks: 19,
};

fn pipes(capacity: u64) -> CommandStream {
    CommandStream::new(ID, OutputMode::Pipes, capacity).unwrap()
}

#[test]
fn exit_and_pipe_eof_in_every_order_still_wait_for_pending_output() {
    let orders = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    for order in orders {
        let mut stream = pipes(8);
        stream.queue_output(ID, Channel::Stdout, 5).unwrap();
        stream.queue_output(ID, Channel::Stderr, 3).unwrap();
        for step in order {
            match step {
                0 => stream.observe_exit(ID, 17).unwrap(),
                1 => stream.observe_eof(ID, Channel::Stdout).unwrap(),
                _ => stream.observe_eof(ID, Channel::Stderr).unwrap(),
            }
            assert_eq!(stream.outcome(), CommandOutcome::Pending);
        }
        stream.sent_output(ID, Channel::Stdout, 5).unwrap();
        stream.sent_output(ID, Channel::Stderr, 2).unwrap();
        assert_eq!(stream.outcome(), CommandOutcome::Pending);
        stream.sent_output(ID, Channel::Stderr, 1).unwrap();
        assert_eq!(stream.outcome(), CommandOutcome::Exited(17));
    }
}

#[test]
fn no_cumulative_output_quota_and_backpressure_is_shared_across_channels() {
    let mut stream = pipes(8);
    for _ in 0..1024 {
        stream.queue_output(ID, Channel::Stdout, 5).unwrap();
        stream.queue_output(ID, Channel::Stderr, 3).unwrap();
        let before = stream.clone();
        assert_eq!(
            stream.queue_output(ID, Channel::Stderr, 1),
            Err(CommandError::BufferFull)
        );
        assert_eq!(stream, before);
        stream.sent_output(ID, Channel::Stdout, 5).unwrap();
        stream.sent_output(ID, Channel::Stderr, 3).unwrap();
    }
    assert_eq!(stream.pending_bytes(), 0);
    assert_eq!(stream.outcome(), CommandOutcome::Pending);
}

#[test]
fn stale_session_command_pid_or_birth_cannot_publish_or_release_output() {
    let stale = [
        CommandIdentity { session: 8, ..ID },
        CommandIdentity { command: 12, ..ID },
        CommandIdentity { pid: 4322, ..ID },
        CommandIdentity {
            start_ticks: 20,
            ..ID
        },
    ];
    for identity in stale {
        let mut stream = pipes(8);
        stream.queue_output(ID, Channel::Stdout, 3).unwrap();
        let before = stream.clone();
        assert_eq!(
            stream.queue_output(identity, Channel::Stdout, 1),
            Err(CommandError::StaleIdentity)
        );
        assert_eq!(
            stream.sent_output(identity, Channel::Stdout, 1),
            Err(CommandError::StaleIdentity)
        );
        assert_eq!(
            stream.observe_eof(identity, Channel::Stdout),
            Err(CommandError::StaleIdentity)
        );
        assert_eq!(
            stream.observe_exit(identity, 0),
            Err(CommandError::StaleIdentity)
        );
        assert_eq!(
            stream.fail(identity, StreamFailure::Cancelled),
            Err(CommandError::StaleIdentity)
        );
        assert_eq!(stream, before);
    }
}

#[test]
fn disconnect_never_becomes_a_process_exit_or_releases_queued_bytes() {
    let mut stream = pipes(8);
    stream.queue_output(ID, Channel::Stdout, 4).unwrap();
    stream.fail(ID, StreamFailure::Transport).unwrap();
    assert_eq!(stream.observed_exit(), None);
    assert_eq!(stream.pending_bytes(), 4);
    assert_eq!(
        stream.outcome(),
        CommandOutcome::Failed(StreamFailure::Transport)
    );
    assert_eq!(
        stream.observe_exit(ID, 0),
        Err(CommandError::TerminalStream)
    );
}

#[test]
fn read_failure_after_exit_is_still_incomplete_output() {
    let mut stream = pipes(8);
    stream.observe_exit(ID, 0).unwrap();
    stream.observe_eof(ID, Channel::Stdout).unwrap();
    stream.fail(ID, StreamFailure::Read).unwrap();
    assert_eq!(stream.observed_exit(), Some(0));
    assert_eq!(
        stream.outcome(),
        CommandOutcome::Failed(StreamFailure::Read)
    );
}

#[test]
fn eof_without_wait_result_is_not_completion() {
    let mut stream = pipes(8);
    stream.observe_eof(ID, Channel::Stdout).unwrap();
    stream.observe_eof(ID, Channel::Stderr).unwrap();
    assert_eq!(stream.outcome(), CommandOutcome::Pending);
    let before = stream.clone();
    assert_eq!(
        stream.queue_output(ID, Channel::Stdout, 1),
        Err(CommandError::ClosedChannel)
    );
    assert_eq!(stream, before);
    stream.observe_exit(ID, 0).unwrap();
    assert_eq!(stream.outcome(), CommandOutcome::Exited(0));
}

#[test]
fn pty_has_one_channel_and_does_not_accept_stdout_or_stderr() {
    let mut stream = CommandStream::new(ID, OutputMode::Pty, 4).unwrap();
    let before = stream.clone();
    for channel in [Channel::Stdout, Channel::Stderr] {
        assert_eq!(
            stream.queue_output(ID, channel, 1),
            Err(CommandError::WrongChannel)
        );
        assert_eq!(
            stream.observe_eof(ID, channel),
            Err(CommandError::WrongChannel)
        );
    }
    assert_eq!(stream, before);
    stream.queue_output(ID, Channel::Pty, 4).unwrap();
    stream.observe_exit(ID, -1).unwrap();
    stream.observe_eof(ID, Channel::Pty).unwrap();
    assert_eq!(stream.outcome(), CommandOutcome::Pending);
    stream.sent_output(ID, Channel::Pty, 4).unwrap();
    assert_eq!(stream.outcome(), CommandOutcome::Exited(-1));
}

#[test]
fn conflicting_exit_and_unobserved_output_preserve_state() {
    let mut stream = pipes(u64::MAX);
    stream.observe_exit(ID, 42).unwrap();
    stream.observe_exit(ID, 42).unwrap();
    stream.queue_output(ID, Channel::Stdout, u64::MAX).unwrap();
    let before = stream.clone();
    assert_eq!(
        stream.observe_exit(ID, 0),
        Err(CommandError::ConflictingExit)
    );
    assert_eq!(
        stream.sent_output(ID, Channel::Stderr, 1),
        Err(CommandError::UnobservedOutput)
    );
    assert_eq!(
        stream.queue_output(ID, Channel::Stderr, 1),
        Err(CommandError::BufferFull)
    );
    assert_eq!(stream, before);
}

#[test]
fn configuration_rejects_missing_identity_but_allows_zero_start_ticks() {
    for id in [
        CommandIdentity { session: 0, ..ID },
        CommandIdentity { command: 0, ..ID },
        CommandIdentity { pid: 0, ..ID },
    ] {
        assert_eq!(
            CommandStream::new(id, OutputMode::Pipes, 4),
            Err(CommandError::InvalidConfiguration)
        );
    }
    assert_eq!(
        CommandStream::new(ID, OutputMode::Pipes, 0),
        Err(CommandError::InvalidConfiguration)
    );
    assert!(
        CommandStream::new(
            CommandIdentity {
                start_ticks: 0,
                ..ID
            },
            OutputMode::Pipes,
            4
        )
        .is_ok()
    );
}

#[test]
fn empty_file_still_requires_an_observed_end() {
    let mut file = Transfer::new(16, 4, Some(0)).unwrap();
    assert_eq!(file.outcome(), TransferOutcome::Pending);
    file.end_input().unwrap();
    assert_eq!(file.outcome(), TransferOutcome::Pending);
    file.finish().unwrap();
    assert_eq!(file.outcome(), TransferOutcome::Complete(0));
}

#[test]
fn known_and_chunked_files_complete_only_after_input_end_and_actual_writes() {
    for expected in [Some(9), None] {
        let mut file = Transfer::new(9, 4, expected).unwrap();
        for _ in 0..2 {
            file.receive(4).unwrap();
            file.record_write(1).unwrap();
            file.record_write(3).unwrap();
        }
        file.receive(1).unwrap();
        file.end_input().unwrap();
        assert_eq!(file.outcome(), TransferOutcome::Pending);
        assert_eq!(file.finish(), Err(TransferError::Incomplete));
        assert_eq!(file.receive(0), Err(TransferError::InputClosed));
        file.record_write(1).unwrap();
        assert_eq!(file.outcome(), TransferOutcome::Pending);
        file.finish().unwrap();
        assert_eq!(file.outcome(), TransferOutcome::Complete(9));
    }
}

#[test]
fn short_body_and_failed_write_preserve_partial_effects() {
    let mut short = Transfer::new(8, 4, Some(8)).unwrap();
    short.receive(4).unwrap();
    short.record_write(3).unwrap();
    short.end_input().unwrap();
    assert_eq!(
        short.outcome(),
        TransferOutcome::Failed {
            reason: StreamFailure::Truncated,
            written: 3
        }
    );
    assert_eq!(short.pending_bytes(), 1);
    let mut failed = Transfer::new(8, 4, None).unwrap();
    failed.receive(4).unwrap();
    failed.record_write(2).unwrap();
    failed.fail(StreamFailure::Write).unwrap();
    assert_eq!(
        failed.outcome(),
        TransferOutcome::Failed {
            reason: StreamFailure::Write,
            written: 2
        }
    );
    assert_eq!(failed.record_write(2), Err(TransferError::TerminalTransfer));
}

#[test]
fn total_advertised_and_resident_limits_are_distinct() {
    assert_eq!(
        Transfer::new(8, 4, Some(9)),
        Err(TransferError::LimitExceeded)
    );
    for expected in [Some(6), None] {
        let mut file = Transfer::new(6, 4, expected).unwrap();
        assert_eq!(file.receive(5), Err(TransferError::BufferFull));
        file.receive(4).unwrap();
        let before = file.clone();
        assert_eq!(file.receive(1), Err(TransferError::BufferFull));
        assert_eq!(file.record_write(5), Err(TransferError::UnobservedWrite));
        assert_eq!(file, before);
        file.record_write(4).unwrap();
        let before = file.clone();
        assert_eq!(file.receive(3), Err(TransferError::LimitExceeded));
        assert_eq!(file, before);
        file.receive(2).unwrap();
        file.record_write(2).unwrap();
        assert_eq!(file.outcome(), TransferOutcome::Pending);
        file.end_input().unwrap();
        file.finish().unwrap();
        assert_eq!(file.outcome(), TransferOutcome::Complete(6));
    }
}

#[test]
fn representational_boundary_does_not_wrap_or_reset_file_budget() {
    let mut file = Transfer::new(u64::MAX, u64::MAX, None).unwrap();
    file.receive(u64::MAX).unwrap();
    file.record_write(u64::MAX).unwrap();
    assert_eq!(file.receive(1), Err(TransferError::LimitExceeded));
    file.end_input().unwrap();
    file.finish().unwrap();
    assert_eq!(file.outcome(), TransferOutcome::Complete(u64::MAX));
}

#[test]
fn close_or_metadata_failure_after_full_write_is_not_success() {
    let mut file = Transfer::new(8, 4, Some(4)).unwrap();
    file.receive(4).unwrap();
    file.record_write(4).unwrap();
    assert_eq!(file.finish(), Err(TransferError::Incomplete));
    file.end_input().unwrap();
    assert_eq!(file.outcome(), TransferOutcome::Pending);
    file.fail(StreamFailure::Write).unwrap();
    assert_eq!(
        file.outcome(),
        TransferOutcome::Failed {
            reason: StreamFailure::Write,
            written: 4
        }
    );
    assert_eq!(file.finish(), Err(TransferError::TerminalTransfer));
}
