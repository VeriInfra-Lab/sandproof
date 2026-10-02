//! One bounded, stateless request-window reduction. No clock or persistence.
use sandbox_controller_core::maintenance::windows::{
    WindowDecision, WindowInput, WindowReject, WindowState, reduce_windows,
};
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};
use std::process::ExitCode;

const VERSION: u32 = 1;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    version: u32,
    state: WindowState,
    input: WindowInput,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum Outcome {
    Ok { decision: WindowDecision },
    Rejected { reason: WindowReject },
    Error { code: &'static str },
}

#[derive(Serialize)]
struct Response {
    version: u32,
    #[serde(flatten)]
    outcome: Outcome,
}

const ERROR_CODES: [&str; 5] = [
    "invalid_request",
    "unsupported_version",
    "input_too_large",
    "output_too_large",
    "input_io",
];

fn error(code: &'static str) -> Response {
    Response {
        version: VERSION,
        outcome: Outcome::Error { code },
    }
}

/// A budgeted serialization sink. No partial success reaches stdout.
struct BoundedBytes {
    bytes: Vec<u8>,
    maximum: usize,
}

impl Write for BoundedBytes {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.maximum - self.bytes.len() {
            return Err(io::Error::other("output budget"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn encode(response: &Response, maximum: usize) -> Result<Vec<u8>, serde_json::Error> {
    let mut output = BoundedBytes {
        bytes: Vec::new(),
        maximum,
    };
    serde_json::to_writer(&mut output, response)?;
    Ok(output.bytes)
}

fn output_minimum() -> usize {
    ERROR_CODES
        .iter()
        .map(|code| {
            serde_json::to_vec(&error(code))
                .expect("fixed protocol error")
                .len()
        })
        .max()
        .expect("protocol has errors")
}

fn bounds(args: impl Iterator<Item = String>) -> Option<(usize, usize)> {
    let args: Vec<_> = args.collect();
    if args.len() != 4 || args[0] != "--max-input-bytes" || args[2] != "--max-output-bytes" {
        return None;
    }
    // Canonical positive decimal limits; no implicit deployment defaults.
    let parse = |text: &str| {
        if text.starts_with('0') || !text.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        text.parse::<usize>().ok()
    };
    let input = parse(&args[1])?;
    let output = parse(&args[3])?;
    if input == 0 || input.checked_add(1).is_none() || output < output_minimum() {
        return None;
    }
    Some((input, output))
}

fn reduce(bytes: &[u8]) -> Response {
    // Existing validated WindowState/WindowInput serde types also reject unknown
    // or duplicate fields. from_slice rejects truncated and trailing JSON.
    let Ok(request) = serde_json::from_slice::<Request>(bytes) else {
        return error("invalid_request");
    };
    if request.version != VERSION {
        return error("unsupported_version");
    }
    let outcome = match reduce_windows(&request.state, request.input) {
        Ok(decision) => Outcome::Ok { decision },
        Err(reason) => Outcome::Rejected { reason },
    };
    Response {
        version: VERSION,
        outcome,
    }
}

fn run(input: impl Read, input_maximum: usize, output_maximum: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    // Read at most one byte past the cap to distinguish exact-boundary input.
    let response = match input
        .take((input_maximum + 1) as u64)
        .read_to_end(&mut bytes)
    {
        Err(_) => error("input_io"),
        Ok(_) if bytes.len() > input_maximum => error("input_too_large"),
        Ok(_) => reduce(&bytes),
    };
    encode(&response, output_maximum).unwrap_or_else(|_| {
        encode(&error("output_too_large"), output_maximum).expect("validated protocol error budget")
    })
}

fn main() -> ExitCode {
    let Some((input_maximum, output_maximum)) = bounds(std::env::args().skip(1)) else {
        // Invalid CLI has no trustworthy output budget. Emit nothing.
        return ExitCode::from(2);
    };
    let output = run(io::stdin().lock(), input_maximum, output_maximum);
    let mut stdout = io::stdout().lock();
    if stdout
        .write_all(&output)
        .and_then(|()| stdout.flush())
        .is_err()
    {
        return ExitCode::from(3);
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandbox_controller_core::{TenantId, maintenance::windows::WindowLimits};

    fn request() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "version": VERSION,
            "state": WindowState::new(WindowLimits::new(1, 10).unwrap()),
            "input": WindowInput::Issue { tenant: TenantId(1), now_ms: 0, duration_ms: 10 },
        }))
        .unwrap()
    }

    #[test]
    fn bounded_output_is_all_or_error_for_every_budget() {
        let input = request();
        let full = run(input.as_slice(), input.len(), 4096);
        for maximum in output_minimum()..=full.len() {
            let encoded = run(input.as_slice(), input.len(), maximum);
            assert!(encoded.len() <= maximum);
            let decoded: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
            if maximum < full.len() {
                assert_eq!(decoded["code"], "output_too_large");
                assert!(decoded.get("decision").is_none());
            } else {
                assert_eq!(encoded, full);
            }
        }
    }

    #[test]
    fn input_is_read_only_to_cap_plus_one() {
        let mut input = io::Cursor::new(vec![b' '; 100]);
        let response = run(&mut input, 20, output_minimum());
        assert_eq!(input.position(), 21);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&response).unwrap()["code"],
            "input_too_large"
        );
    }

    #[test]
    fn cli_requires_explicit_valid_budgets() {
        assert!(bounds(Vec::new().into_iter()).is_none());
        for value in ["0", "-1", "+1", "01", "x", "18446744073709551615"] {
            assert!(
                bounds(
                    ["--max-input-bytes", value, "--max-output-bytes", "4096"]
                        .map(str::to_string)
                        .into_iter()
                )
                .is_none()
            );
        }
        assert!(
            bounds(
                ["--max-input-bytes", "4096", "--max-output-bytes", "1"]
                    .map(str::to_string)
                    .into_iter()
            )
            .is_none()
        );
    }
}
