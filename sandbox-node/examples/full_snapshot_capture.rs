//! Explicit root-only component probe, not a product lifecycle entrypoint.
//! Capture mode deliberately leaves the original VM paused, including on error.
//! Cleanup mode needs its exact persisted process identity and is test discard.

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("full_snapshot_capture requires Linux/KVM");
    std::process::exit(1);
}

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    linux::run()
}

#[cfg(target_os = "linux")]
mod linux {
    use sandbox_runtime::{
        FullCaptureLimits, Runtime, RuntimeConfig, RuntimeError, RuntimeProcessIdentity,
        SnapshotLayout, VmState,
    };
    use sandproof_node::full_snapshot::{
        ExpectedFullCapture, SealedFullSnapshot, capture_directory_key, seal_full_capture,
    };
    use sandproof_protocol::lifecycle::{
        CaptureBinding, CaptureId, RequestSequence, SnapshotFileKind, VmCompatibility,
    };
    use serde::Deserialize;
    use serde_json::json;
    use sha2::{Digest, Sha256};
    use std::{
        error::Error,
        fmt::Write as _,
        fs::{self, File, OpenOptions},
        io::{Read, Write},
        os::unix::fs::OpenOptionsExt,
        path::{Path, PathBuf},
        time::{Duration, Instant},
    };

    type ProbeResult<T> = Result<T, Box<dyn Error>>;

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Fixture {
        runtime: RuntimeConfig,
        layout_parent: PathBuf,
        layout_binding: [u8; 32],
        resource_id: String,
        store: PathBuf,
        binding: CaptureBinding,
        compatibility: VmCompatibility,
        expected_source: Option<RuntimeProcessIdentity>,
    }

    fn record(report: &mut File, value: &serde_json::Value) -> ProbeResult<()> {
        serde_json::to_writer(&mut *report, value)?;
        report.write_all(b"\n")?;
        report.sync_all()?;
        Ok(())
    }

    fn hash_file(path: &Path) -> ProbeResult<[u8; 32]> {
        let mut file = File::open(path)?;
        let mut hash = Sha256::new();
        let mut buffer = [0_u8; 8192];
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hash.update(&buffer[..count]);
        }
        Ok(hash.finalize().into())
    }

    fn console_ready(fixture: &Fixture) -> ProbeResult<()> {
        let path = fixture
            .runtime
            .chroot_base
            .join(".claims")
            .join(&fixture.resource_id)
            .join("console.log");
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let text = fs::read_to_string(&path)?;
            if text.contains("Ubuntu 24.04") && text.contains("login:") {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err("original guest boot marker timed out".into());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn verify_published_files(
        fixture: &Fixture,
        sealed: &SealedFullSnapshot,
        directory_key: [u8; 32],
    ) -> ProbeResult<()> {
        let mut bundle_name = "capture-".to_owned();
        for byte in directory_key {
            write!(&mut bundle_name, "{byte:02x}")?;
        }
        let bundle = fixture.store.join(bundle_name);
        for file in &sealed.manifest().files {
            let path = bundle.join(&file.relative_path);
            if path.metadata()?.len() != file.length
                || hash_file(&path)? != file.digest.into_bytes()
            {
                return Err("published actual file differs from manifest measurement".into());
            }
            if matches!(file.kind, SnapshotFileKind::Disk { .. }) {
                let original_disk = fixture
                    .runtime
                    .chroot_base
                    .join(
                        fixture
                            .runtime
                            .firecracker
                            .file_name()
                            .ok_or("binary basename")?,
                    )
                    .join(&fixture.resource_id)
                    .join("root/rootfs.ext4");
                if hash_file(&original_disk)? != file.digest.into_bytes() {
                    return Err("frozen private disk differs from sealed disk".into());
                }
            }
        }
        if hash_file(&bundle.join("manifest.json"))? != *sealed.physical().files().manifest_sha256()
        {
            return Err("published manifest bytes differ".into());
        }
        Ok(())
    }

    fn capture(
        fixture: &Fixture,
        runtime: &Runtime,
        layout: &SnapshotLayout,
        report: &mut File,
    ) -> ProbeResult<()> {
        let launched = runtime.launch(&fixture.resource_id)?;
        let source = RuntimeProcessIdentity::capture(launched.pid)?;
        record(
            report,
            &json!({"phase":"launched","resource_id":fixture.resource_id,"source":source,"observation":launched}),
        )?;
        console_ready(fixture)?;
        let original = runtime.pause_original_vm(&fixture.resource_id, &source)?;
        record(report, &json!({"phase":"original_paused","source":source}))?;
        let disk_bytes = fixture.runtime.rootfs.metadata()?.len();
        let mut failed_binding = fixture.binding.clone();
        failed_binding.capture = CaptureId::try_from(1)?;
        failed_binding.operation.key.sequence = RequestSequence::try_from(1)?;
        let failed_key = capture_directory_key(&failed_binding)?;
        let directory_key = capture_directory_key(&fixture.binding)?;
        if failed_key == directory_key {
            return Err("fixture requires two different original capture keys".into());
        }
        let failed = runtime.capture_full_into(
            &original,
            layout,
            failed_key,
            FullCaptureLimits {
                state_bytes: 1,
                disk_bytes,
                api_timeout: Duration::from_secs(30),
            },
        );
        if !matches!(
            failed,
            Err(RuntimeError::IdentityMismatch(
                "snapshot source ownership, type or length"
            ))
        ) {
            return Err("expected real state-length rejection after Full capture".into());
        }
        runtime.confirm_original_paused(&original)?;
        if !source.is_alive()? {
            return Err("failed capture lost original VMM".into());
        }
        record(
            report,
            &json!({"phase":"failed_capture_preserved_original","source":source,"directory_key":failed_key}),
        )?;
        let captured = runtime.capture_full_into(
            &original,
            layout,
            directory_key,
            FullCaptureLimits {
                state_bytes: 16 * 1024 * 1024,
                disk_bytes,
                api_timeout: Duration::from_secs(30),
            },
        )?;
        let sealed = seal_full_capture(
            captured,
            &ExpectedFullCapture {
                resource_id: &fixture.resource_id,
                source: &source,
                binding: &fixture.binding,
                compatibility: &fixture.compatibility,
            },
        )?;
        runtime.confirm_original_paused(&original)?;
        verify_published_files(fixture, &sealed, directory_key)?;
        record(
            report,
            &json!({"phase":"physical_bundle_sealed_original_still_paused","source":source,
            "directory_key":sealed.physical().files().directory_key(),"manifest":sealed.manifest(),"manifest_sha256":sealed.physical().files().manifest_sha256(),
            "scope":"physical component only; no SQLite handoff, restore, RAM nonce or GC claim"}),
        )?;
        // No automatic stop/cleanup: the caller next checks that this manager's
        // exit does not kill the original VMM, then explicitly discards the test.
        Ok(())
    }

    fn cleanup(fixture: &Fixture, runtime: &Runtime, report: &mut File) -> ProbeResult<()> {
        let expected = fixture
            .expected_source
            .as_ref()
            .ok_or("exact persisted test source required")?;
        let observed = runtime.observe(&fixture.resource_id)?;
        if !expected.is_alive()?
            || expected.pid != observed.pid
            || expected.start_time_ticks != observed.start_time_ticks
        {
            return Err("cleanup original source binding rejected".into());
        }
        record(
            report,
            &json!({"phase":"source_survived_manager_exit","source":expected}),
        )?;
        let stopped = runtime.stop(&fixture.resource_id)?;
        if stopped.state != VmState::Stopped {
            return Err("test original VM did not stop".into());
        }
        runtime.cleanup(&fixture.resource_id)?;
        record(
            report,
            &json!({"phase":"test_discard_stopped_and_cleaned_original","source":expected}),
        )?;
        Ok(())
    }

    pub(super) fn run() -> ProbeResult<()> {
        let args: Vec<_> = std::env::args().collect();
        if args.len() != 4
            || !matches!(
                args[1].as_str(),
                "initialize-layout" | "check-layout" | "capture" | "cleanup-test-original"
            )
        {
            return Err("usage: full_snapshot_capture <initialize-layout|check-layout|capture|cleanup-test-original> <private-input.json> <new-report.jsonl>".into());
        }
        let fixture: Fixture = serde_json::from_slice(&fs::read(&args[2])?)?;
        let mut report = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&args[3])?;
        if args[1] == "initialize-layout" {
            let layout =
                SnapshotLayout::initialize(&fixture.layout_parent, fixture.layout_binding)?;
            record(
                &mut report,
                &json!({"phase":"layout_initialized","binding":fixture.layout_binding}),
            )?;
            if layout.runtime_root() != fixture.runtime.chroot_base
                || layout.store_root() != fixture.store
            {
                return Err("fixture layout paths mismatch".into());
            }
            return Ok(());
        }
        let layout = SnapshotLayout::reopen(&fixture.layout_parent, fixture.layout_binding)?;
        if layout.runtime_root() != fixture.runtime.chroot_base
            || layout.store_root() != fixture.store
        {
            return Err("fixture original layout paths mismatch".into());
        }
        if args[1] == "check-layout" {
            record(
                &mut report,
                &json!({"phase":"original_layout_reopened","binding":fixture.layout_binding}),
            )?;
            return Ok(());
        }
        let runtime = Runtime::new(fixture.runtime.clone())?;
        match args[1].as_str() {
            "capture" => capture(&fixture, &runtime, &layout, &mut report),
            "cleanup-test-original" => cleanup(&fixture, &runtime, &mut report),
            _ => unreachable!(),
        }
    }
}
