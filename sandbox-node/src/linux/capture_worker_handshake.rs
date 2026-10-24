//! One job/Ready/Release exchange with the same original worker and pipes.
use super::*;
use std::io::{self, Read, Write};
use std::os::fd::AsFd;

pub(super) const RELEASE: &[u8] = b"release\n";
const READY_BYTES: usize = 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ready {
    job_digest: ContentDigest,
    worker: RuntimeProcessIdentity,
}

pub(super) fn job_frame(mut body: Vec<u8>) -> NodeResult<Vec<u8>> {
    if body.len() > CAPTURE_WORKER_INPUT_BYTES - 4 {
        return Err("Full job frame exceeds original byte budget".into());
    }
    let length = u32::try_from(body.len())?.to_be_bytes();
    body.extend_from_slice(&length);
    body.rotate_right(4);
    Ok(body)
}
pub(super) fn frame_payload(frame: &[u8]) -> NodeResult<&[u8]> {
    let length: [u8; 4] = frame.get(..4).ok_or("short Full job frame")?.try_into()?;
    let body = &frame[4..];
    if usize::try_from(u32::from_be_bytes(length))? != body.len()
        || frame.len() > CAPTURE_WORKER_INPUT_BYTES
    {
        return Err("Full job frame length differs".into());
    }
    Ok(body)
}
fn read_job(input: &mut impl Read, expected: ContentDigest) -> NodeResult<CaptureWorkerJob> {
    let mut length = [0; 4];
    input.read_exact(&mut length)?;
    let length = usize::try_from(u32::from_be_bytes(length))?;
    if length > CAPTURE_WORKER_INPUT_BYTES - 4 {
        return Err("Full job length exceeds original byte budget".into());
    }
    let mut body = vec![0; length];
    input.read_exact(&mut body)?;
    let job = decode_job(expected, &body)?;
    job.validate_job()?;
    Ok(job)
}
pub(super) fn ready_frame(
    job_digest: ContentDigest,
    worker: RuntimeProcessIdentity,
) -> NodeResult<Vec<u8>> {
    let mut bytes = serde_json::to_vec(&Ready { job_digest, worker })?;
    bytes.push(b'\n');
    if bytes.len() > READY_BYTES {
        return Err("Full Ready exceeds original output budget".into());
    }
    Ok(bytes)
}
pub(super) fn check_ready(
    bytes: &[u8],
    expected: ContentDigest,
    process: &RuntimeProcessIdentity,
) -> NodeResult<()> {
    let line = bytes.strip_suffix(b"\n").ok_or("incomplete Full Ready")?;
    if bytes.len() > READY_BYTES {
        return Err("oversize Full Ready".into());
    }
    let ready: Ready = serde_json::from_slice(line)?;
    if ready.job_digest != expected || &ready.worker != process {
        return Err("Full Ready differs from original worker/job".into());
    }
    Ok(())
}

/// A complete release AND pipe EOF is required. The worker itself decides not
/// to send a request on EOF/error/terminal source exit; no parent infers a fact.
fn await_release(
    input: &mut impl Read,
    source: &RuntimeProcessIdentity,
    deadline: Instant,
) -> NodeResult<bool> {
    let mut release = [0; 8];
    let mut received = 0;
    loop {
        if Instant::now() >= deadline {
            return Ok(false);
        }
        let end = if received < RELEASE.len() {
            RELEASE.len()
        } else {
            release.len()
        };
        match input.read(&mut release[received..end]) {
            Ok(0) => return Ok(received == RELEASE.len() && &release[..received] == RELEASE),
            Ok(n) => {
                received += n;
                if received > RELEASE.len() || !RELEASE.starts_with(&release[..received]) {
                    return Ok(false);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if !source.is_alive()? {
                    return Ok(false);
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => return Ok(false),
        }
    }
}

impl CaptureWorkerJob {
    fn request_end_layout(&self) -> NodeResult<SnapshotLayout> {
        let layout = SnapshotLayout::reopen(&self.layout_parent, *self.layout_binding.as_bytes())?
            .with_material_project(self.material_project.clone())?;
        if layout.runtime_root() != self.runtime.chroot_base {
            return Err("Full Ready layout differs".into());
        }
        Ok(layout)
    }
    fn no_request(&self, layout: &SnapshotLayout, expected: ContentDigest) -> NodeResult<()> {
        let source = crate::source_observation::expected_source(&self.source)?;
        layout.publish_full_no_request(
            self.full.pause.source_resource.id.as_str(),
            &source,
            full_snapshot::capture_directory_key(&self.full.pause.binding)?,
            api_context(&self.action, &self.full, expected)?,
        )?;
        Ok(())
    }
}

pub(super) fn run(expected_job_digest: &str) -> NodeResult<()> {
    let started = Instant::now();
    let expected = decode_digest(expected_job_digest)?;
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let job = read_job(&mut input, expected)?;
    let layout = job.request_end_layout()?;
    let source = crate::source_observation::expected_source(&job.source)?;
    sandbox_runtime::FullCaptureRequestEnd::validate_capacity(
        job.full.pause.source_resource.id.as_str(),
        &source,
        full_snapshot::capture_directory_key(&job.full.pause.binding)?,
        api_context(&job.action, &job.full, expected)?,
    )?;
    layout.require_full_request_unstarted(full_snapshot::capture_directory_key(
        &job.full.pause.binding,
    )?)?;
    let deadline = started
        .checked_add(Duration::from_millis(job.full.worker_timeout_ms))
        .ok_or("Full worker release deadline overflow")?;
    let ready = ready_frame(
        expected,
        RuntimeProcessIdentity::capture(std::process::id())?,
    )?;
    let flags = rustix::fs::fcntl_getfl(stdin.as_fd())?;
    rustix::fs::fcntl_setfl(stdin.as_fd(), flags | rustix::fs::OFlags::NONBLOCK)?;
    let mut output = io::stdout().lock();
    if output
        .write_all(&ready)
        .and_then(|()| output.flush())
        .is_err()
    {
        job.no_request(&layout, expected)?;
        return Ok(());
    }
    // A failed gate observation makes this worker decline, never dispatch.
    let released = await_release(&mut input, &source, deadline).unwrap_or(false);
    let outcome = if released {
        match job.execute(expected) {
            Ok(manifest_digest) => CaptureWorkerOutcome::Captured { manifest_digest },
            Err(_) => CaptureWorkerOutcome::Failed,
        }
    } else {
        job.no_request(&layout, expected)?;
        CaptureWorkerOutcome::Failed
    };
    let mut reply = serde_json::to_vec(&CaptureWorkerReply {
        job_digest: expected,
        outcome,
    })?;
    reply.push(b'\n');
    if ready.len() + reply.len() > CAPTURE_WORKER_OUTPUT_BYTES {
        return Err("combined Full output exceeds original budget".into());
    }
    output.write_all(&reply)?;
    output.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn process() -> RuntimeProcessIdentity {
        RuntimeProcessIdentity::capture(std::process::id()).unwrap()
    }
    #[test]
    fn job_frame_uses_one_original_input_budget_and_exact_length() {
        let frame = job_frame(vec![7; CAPTURE_WORKER_INPUT_BYTES - 4]).unwrap();
        assert_eq!(frame.len(), CAPTURE_WORKER_INPUT_BYTES);
        assert_eq!(
            frame_payload(&frame).unwrap().len(),
            CAPTURE_WORKER_INPUT_BYTES - 4
        );
        assert!(job_frame(vec![7; CAPTURE_WORKER_INPUT_BYTES - 3]).is_err());
        for length in 0..4 {
            assert!(frame_payload(&frame[..length]).is_err());
        }
        assert!(frame_payload(&frame[..frame.len() - 1]).is_err());
        let mut corrupt = frame;
        corrupt[3] ^= 1;
        assert!(frame_payload(&corrupt).is_err());
    }
    #[test]
    fn malformed_or_truncated_job_cannot_produce_ready() {
        let expected = digest(b"{}");
        let frame = job_frame(b"{}".to_vec()).unwrap();
        for length in 0..=frame.len() {
            assert!(read_job(&mut &frame[..length], expected).is_err());
        }
        assert!(read_job(&mut &u32::MAX.to_be_bytes()[..], expected).is_err());
    }
    #[test]
    fn ready_matches_exact_original_worker_and_job_and_rejects_noise() {
        let worker = process();
        let expected = digest(b"original");
        let ready = ready_frame(expected, worker.clone()).unwrap();
        check_ready(&ready, expected, &worker).unwrap();
        assert!(check_ready(&ready, digest(b"other"), &worker).is_err());
        let mut other = worker.clone();
        other.start_time_ticks += 1;
        assert!(check_ready(&ready, expected, &other).is_err());
        let mut noisy = ready.clone();
        noisy.push(b'x');
        assert!(check_ready(&noisy, expected, &worker).is_err());
        assert!(check_ready(&ready[..ready.len() - 1], expected, &worker).is_err());
        let reply = serde_json::to_vec(&CaptureWorkerReply {
            job_digest: expected,
            outcome: CaptureWorkerOutcome::Captured {
                manifest_digest: expected,
            },
        })
        .unwrap();
        assert!(ready.len() + reply.len() + 1 <= CAPTURE_WORKER_OUTPUT_BYTES);
    }
    #[test]
    fn only_complete_release_and_eof_can_dispatch_original_request() {
        let source = process();
        let deadline = Instant::now() + Duration::from_secs(1);
        for length in 0..RELEASE.len() {
            assert!(!await_release(&mut &RELEASE[..length], &source, deadline).unwrap());
        }
        let mut complete = RELEASE;
        assert!(await_release(&mut complete, &source, deadline).unwrap());
        assert!(!await_release(&mut &b"release\nextra"[..], &source, deadline).unwrap());
        assert!(!await_release(&mut &b"refuse\n"[..], &source, deadline).unwrap());
        let mut expired = RELEASE;
        assert!(!await_release(&mut expired, &source, Instant::now()).unwrap());
    }
}
