use sandproof_guest_runtime::protocol::{End, Event, ProtocolError, encode_event};

use super::CommandExit;

impl CommandExit {
    /// Encode an observed Linux terminal result using the fixed envd/Go status
    /// spelling. This does not turn a launch/stream failure into a process End.
    /// # Errors
    /// Refuses an insufficient per-message encoding budget.
    pub fn encode_end(self, limit: usize) -> Result<Vec<u8>, ProtocolError> {
        let mut status = self.signal.map_or_else(
            || format!("exit status {}", self.code),
            |signal| {
                let name = usize::try_from(signal)
                    .ok()
                    .and_then(|index| SIGNALS.get(index))
                    .filter(|name| !name.is_empty());
                name.map_or_else(
                    || format!("signal: signal {signal}"),
                    |name| format!("signal: {name}"),
                )
            },
        );
        if self.core_dumped {
            status.push_str(" (core dumped)");
        }
        encode_event(
            Event::End(End {
                exit_code: self.code,
                exited: self.signal.is_none(),
                status: &status,
                error: (self.code != 0 || self.signal.is_some()).then_some(status.as_str()),
            }),
            limit,
        )
    }
}

// Go 1.26.8 syscall/zerrors_linux_{amd64,arm64}.go. This is independent of the
// host locale; libc strerror/strsignal and Rust ExitStatus Display differ.
const SIGNALS: [&str; 32] = [
    "",
    "hangup",
    "interrupt",
    "quit",
    "illegal instruction",
    "trace/breakpoint trap",
    "aborted",
    "bus error",
    "floating point exception",
    "killed",
    "user defined signal 1",
    "segmentation fault",
    "user defined signal 2",
    "broken pipe",
    "alarm clock",
    "terminated",
    "stack fault",
    "child exited",
    "continued",
    "stopped (signal)",
    "stopped",
    "stopped (tty input)",
    "stopped (tty output)",
    "urgent I/O condition",
    "CPU time limit exceeded",
    "file size limit exceeded",
    "virtual timer expired",
    "profiling timer expired",
    "window changed",
    "I/O possible",
    "power failure",
    "bad system call",
];
