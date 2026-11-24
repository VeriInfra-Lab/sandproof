"""Run the original Stage1 native or formal entrance in a private cgroup."""
import argparse
import fcntl
import json
import os
import runpy
from pathlib import Path
import shutil
import subprocess
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from tools import acceptance_receipts, ci_redis, proof_native_suite
from tools.benchmark_ci import remove_private_tree
from tools.ci_retention import private_path, reject_mounts


GIB = 1024**3
MIN_STORAGE_FREE = 256 * GIB
MIN_XFS_FREE = 48 * GIB
MIN_AVAILABLE_MEMORY = 176 * GIB
MEMORY_LIMIT = 160 * GIB
SAMPLE_INTERVAL = 60
ARCHIVE_SHA256 = {
    "SANDPROOF_SQLITE_STATEMENTS_SOURCE_ARCHIVE":
        "c872dffa1831db48d38e7b90649e65ee5e354ef69b73971061c8642a0b6e57c9",
    "SANDPROOF_CHILD_OBSERVATION_SOURCE_ARCHIVE":
        "92ab8a056c5549f62c26e53f70cd386b72460db3a780be5ed004f5b9ff609a57",
    "SANDPROOF_WRITER_SOURCE_ARCHIVE":
        "a96634e94e39f0e3377fa422283f018a4eb84f983a753eaacee2005b98d91a83",
    "SANDPROOF_LIBRARY_SOURCE_ARCHIVE":
        "4ef9f178a2039ad8effd3fc97414d5b464d940fce7e6514a0d8f8e39ef14d687",
    "SANDPROOF_STAGE1_CREATE_SOURCE_ARCHIVE":
        "eecb60f6e9788189e0be71ad7c6907cd73e912f1b8f4d2274c4b54ae207d8fe4",
}


def admit(commit, xfs, mode):
    root = Path.cwd()
    proof_native_suite.trusted_checkout(root)
    if (subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip() != commit
            or subprocess.check_output(["git", "status", "--porcelain"])):
        raise ValueError("the exact clean source checkout is required")
    setting = "SANDPROOF_STAGE1_JOBS" if mode == "formal" else "SANDPROOF_STAGE1_NATIVE_JOBS"
    workers = int(os.environ[setting])
    light_jobs = int(os.environ["SANDPROOF_STAGE1_LIGHT_JOBS"])
    cpus = sorted(os.sched_getaffinity(0))
    worker_limit = 8 if mode == "formal" else 16
    if not 1 <= workers <= worker_limit or not 1 <= light_jobs <= 32 or len(cpus) < max(4*workers, light_jobs):
        raise ValueError("CPU affinity is too small for the requested native and lightweight parallelism")
    memory = Path("/proc/meminfo").read_text().split("MemAvailable:", 1)[1].split()[0]
    if (int(memory) * 1024 < MIN_AVAILABLE_MEMORY or shutil.disk_usage(root).free < MIN_STORAGE_FREE
            or shutil.disk_usage(xfs).free < MIN_XFS_FREE):
        raise ValueError("insufficient available memory or disk for the private native batch")
    redis = Path(os.environ["SANDPROOF_REDIS_SERVER"])
    material = ci_redis.check_material(redis.parent)
    if redis.name != "redis-server" or material["binary_sha256"] != os.environ["SANDPROOF_STAGE1_REDIS_SHA256"]:
        raise ValueError("native Redis differs from the pinned local build")
    if acceptance_receipts.source_archive_inputs() != ARCHIVE_SHA256:
        raise ValueError("the five registered native source archives are required")
    if mode != "formal":
        subprocess.run(["cargo", "metadata", "--offline", "--locked", "--format-version", "1"],
                       stdout=subprocess.DEVNULL, check=True)
    proof_native_suite.stage1_source_admission("stage1-protocol")
    runtime = proof_native_suite.runtime(temporary_parent=xfs) if mode != "formal" else None
    return dict(commit=commit, mode=mode, cpu_affinity=cpus, workers=workers, light_jobs=light_jobs,
                data_disk_free_bytes=shutil.disk_usage(root).free, xfs_free_bytes=shutil.disk_usage(xfs).free,
                redis=material, source_archives=ARCHIVE_SHA256, runtime=runtime)


def cgroup(mode):
    line, = (row for row in Path("/proc/self/cgroup").read_text().splitlines() if row.startswith("0::/"))
    path = Path("/sys/fs/cgroup") / line.removeprefix("0::/")
    if (not path.name.startswith(f"sandproof-stage1-{mode}-")
            or int((path / "memory.max").read_text()) != MEMORY_LIMIT):
        raise ValueError("run native acceptance in its dedicated bounded systemd unit")
    return path


def record_sample(run, xfs, group, started, history, log_errors):
    row = dict(elapsed_seconds=round(time.monotonic() - started, 3),
               dedicated_xfs_used_bytes=shutil.disk_usage(xfs).used,
               shared_data_disk_free_bytes=shutil.disk_usage(run).free,
               memory_current_bytes=int((group / "memory.current").read_text()),
               cgroup_memory_peak_bytes=int((group / "memory.peak").read_text()))
    # Cargo and Lean can remove a file between du's directory read and stat.
    # Its partial total would hide rather than measure a possible peak.
    measurement = subprocess.run(["nice", "-n", "10", "du", "-sxB1", "--", str(run)],
                                 capture_output=True, text=True, check=False)
    try:
        row["private_run_allocated_bytes"] = int(measurement.stdout.split()[0]) if measurement.returncode == 0 else None
    except (IndexError, ValueError):
        row["private_run_allocated_bytes"] = None
    if row["private_run_allocated_bytes"] is None:
        row["private_run_disk_sample_error"] = measurement.stderr.strip()[:1000] or "du returned no usable total"
    history.append(row)
    try:
        with (run / "resource-samples.jsonl").open("a") as samples:
            samples.write(json.dumps(row) + "\n")
    except OSError as error:
        log_errors.append(str(error))


def tidy_ephemeral(native, xfs):
    recorded = native / "temporary-root.json"
    if recorded.is_file():
        info = json.loads(recorded.read_text())
        target = Path(info["path"])
        if (target.parent != xfs or not target.name.startswith("sandproof-native-")
                or info["parent"] != str(xfs) or info["uid"] != os.getuid()
                or info["mode"] != "0o700" or target.is_symlink()):
            raise ValueError("original native scratch identity is uncertain")
        if target.exists():
            reject_mounts(target)
            identity = target.stat()
            remove_private_tree(target, [identity.st_dev, identity.st_ino])


def original_native_approval(commit):
    folder = private_path(Path(os.environ["SANDPROOF_NATIVE_EVIDENCE"]), directory=True)
    seal = private_path(folder / "complete.json")
    seal_sha = acceptance_receipts.sha(seal)
    result = json.loads(seal.read_text())
    snapshot = acceptance_receipts.digest(acceptance_receipts.repository_inputs(Path.cwd()))
    if (result.get("complete") is not True or result.get("exit_code") != 0
            or result.get("scope") != "stage1-protocol" or result.get("repository_sha256") != snapshot):
        raise ValueError("formal Stage1 requires a complete native seal for the exact candidate inputs")
    for manifest in sorted(folder.glob("run-*/build-targets.json")):
        original = private_path(manifest.parent / "native-complete.json")
        record = json.loads(private_path(manifest).read_text())
        if (record.get("kind") == "native-cargo-targets-v1" and record.get("candidate") == commit
                and record.get("completion_sha256") == seal_sha and acceptance_receipts.sha(original) == seal_sha):
            return dict(commit=commit, path=str(seal), sha256=seal_sha, original_manifest=str(manifest),
                        original_manifest_sha256=acceptance_receipts.sha(manifest))
    raise ValueError("no original native run binds the matching seal to this exact commit")


def independent_reference():
    """Record an applicable result as a separate slice, outside the Stage1 seal."""
    value = os.environ.get("SANDPROOF_STAGE1_SOURCE_SAVE_RESULT")
    if not value:
        return None
    path = Path(value).resolve(strict=True)
    result = json.loads(path.read_text())
    package = Path("verification/experiments/source-save-order/unload-parent-link")
    original = runpy.run_path(str(package / "verify.py"))["candidate"]()
    keys = ("source_commit", "source_tree", "source_archive_sha256", "fixture_sha256", "entry_sha256")
    if (result.get("kind") != "source-save-order-formal" or result.get("complete") is not True
            or any(result.get(key) != original[key] for key in keys)
            or set(result.get("views", ())) != {"factored", "reordered"}):
        raise ValueError("independent SourceSaveOrder result is incomplete or uses different inputs")
    review = result.get("review_commit")
    tree = subprocess.check_output(["git", "rev-parse", f"{review}^{{tree}}"], text=True).strip()
    if (tree != result.get("review_tree")
            or subprocess.run(["git", "diff", "--quiet", review, "HEAD", "--", str(package)]).returncode):
        raise ValueError("independent SourceSaveOrder proof files differ from the reviewed candidate")
    return dict(path=str(path), sha256=acceptance_receipts.sha(path), review_commit=review,
                source_commit=result["source_commit"], scope="independent-source-save-order")


def execute(mode, commit, xfs, run, lock):
    group = cgroup(mode)
    native = run / "native"
    with lock.open("r+") as guard:
        try:
            fcntl.flock(guard, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            print("Shared Stage1 lock is busy. The current batch keeps its lease.", file=sys.stderr)
            return 75
        fd = str(guard.fileno())
        env = dict(os.environ, SANDPROOF_LOCK_FILE=str(lock), SANDPROOF_LOCK_FD=fd,
                   SANDPROOF_PROOF_HEAVY_LOCK=str(lock), SANDPROOF_PROOF_HEAVY_LOCK_FD=fd)
        started = time.monotonic()
        history = []
        log_errors = []
        status = None
        cleanup_error = None

        def sample():
            try:
                record_sample(run, xfs, group, started, history, log_errors)
            except OSError as error:
                log_errors.append(str(error))

        try:
            sample()
            admission = admit(commit, xfs, mode)
            if mode == "formal":
                admission["native_evidence"] = original_native_approval(commit)
                independent = independent_reference()
                if independent is not None:
                    admission["independent_result"] = independent
                command = ["bash", "scripts/verify-stage1-protocol.sh"]
            else:
                command = ["bash", "scripts/python-entry.sh", "tools/proof_native_suite.py", "run",
                           "--scope", "stage1-protocol", "--output", str(native)]
            acceptance_receipts.write_json(run / "admission.json", admission)
            with (run / f"{mode}.log").open("xb") as log, subprocess.Popen(
                    command, env=env, stdout=log, stderr=subprocess.STDOUT, pass_fds=(guard.fileno(),)) as child:
                while status is None:
                    try:
                        status = child.wait(timeout=SAMPLE_INTERVAL)
                    except subprocess.TimeoutExpired:
                        sample()
        finally:
            sample()
            # Scratch cleanup requires the original command to have exited and
            # the private cgroup to contain only this monitor.
            remaining = {int(pid) for pid in (group / "cgroup.procs").read_text().split()} - {os.getpid()}
            events = {k: int(v) for k, v in (row.split() for row in (group / "memory.events").read_text().splitlines())}
            cleanup = "formal artifacts retained" if mode == "formal" else "native scratch retained"
            if mode == "native" and status == 0 and not remaining and not events["oom_kill"]:
                try:
                    tidy_ephemeral(native, xfs)
                    cleanup = "private temporary files removed"
                except (OSError, ValueError) as error:
                    cleanup_error = str(error)
            peak = max((row["private_run_allocated_bytes"] for row in history
                        if row["private_run_allocated_bytes"] is not None), default=None)
            acceptance_receipts.write_json(run / "resources.json", dict(
                commit=commit, mode=mode, child_returncode=status,
                elapsed_seconds=round(time.monotonic() - started, 3),
                cgroup_memory_peak_bytes=int((group / "memory.peak").read_text()), memory_events=events,
                private_run_sampled_disk_peak_bytes=peak, samples=len(history), sample_log_errors=log_errors,
                private_run_disk_samples_missing=sum(row["private_run_allocated_bytes"] is None for row in history),
                dedicated_xfs_sampled_used_peak_bytes=max((row["dedicated_xfs_used_bytes"] for row in history), default=None),
                shared_data_disk_min_free_bytes=min((row["shared_data_disk_free_bytes"] for row in history), default=None),
                remaining_processes=sorted(remaining), cleanup=cleanup, cleanup_error=cleanup_error))
        return (128 - status if status < 0 else status) or int(bool(remaining or events["oom_kill"] or cleanup_error))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("preflight", "native", "formal"))
    parser.add_argument("commit")
    parser.add_argument("xfs", type=Path)
    parser.add_argument("run", type=Path, nargs="?")
    parser.add_argument("lock", type=Path, nargs="?")
    args = parser.parse_args()
    if args.mode == "preflight":
        print(json.dumps(admit(args.commit, args.xfs, args.mode), indent=2))
        return 0
    return execute(args.mode, args.commit, args.xfs, args.run, args.lock)


if __name__ == "__main__":
    sys.exit(main())
