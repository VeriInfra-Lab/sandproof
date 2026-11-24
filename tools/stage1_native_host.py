"""Run the unchanged Stage1 native entrance in a private cgroup."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from tools import acceptance_receipts, ci_redis, proof_native_suite
from tools.benchmark_ci import remove_private_tree
from tools.ci_retention import reject_mounts


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


def admit(commit, xfs):
    root = Path.cwd()
    proof_native_suite.trusted_checkout(root)
    if (subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip() != commit
            or subprocess.check_output(["git", "status", "--porcelain"])):
        raise ValueError("the exact clean source checkout is required")
    workers = int(os.environ["SANDPROOF_STAGE1_NATIVE_JOBS"])
    light_jobs = int(os.environ["SANDPROOF_STAGE1_LIGHT_JOBS"])
    cpus = sorted(os.sched_getaffinity(0))
    if not 1 <= workers <= 16 or not 1 <= light_jobs <= 32 or len(cpus) < max(4*workers, light_jobs):
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
    if subprocess.run(["cargo", "metadata", "--offline", "--locked", "--format-version", "1"],
                      stdout=subprocess.DEVNULL, check=False).returncode:
        raise ValueError("the fixed Rust toolchain cannot resolve the locked dependencies offline")
    proof_native_suite.stage1_source_admission("stage1-protocol")
    runtime = proof_native_suite.runtime(temporary_parent=xfs)
    return dict(commit=commit, cpu_affinity=cpus, native_jobs=workers, light_jobs=light_jobs,
                data_disk_free_bytes=shutil.disk_usage(root).free, xfs_free_bytes=shutil.disk_usage(xfs).free,
                redis=material, source_archives=ARCHIVE_SHA256, runtime=runtime)


def cgroup():
    line, = (row for row in Path("/proc/self/cgroup").read_text().splitlines() if row.startswith("0::/"))
    path = Path("/sys/fs/cgroup") / line.removeprefix("0::/")
    if (not path.name.startswith("sandproof-stage1-native-")
            or int((path / "memory.max").read_text()) != MEMORY_LIMIT):
        raise ValueError("run native acceptance in its dedicated bounded systemd unit")
    return path


def allocated(run):
    # Sample one private tree only. The shared workspace is not scanned.
    return int(subprocess.check_output(["nice", "-n", "10", "du", "-sxB1", "--", str(run)]).split()[0])


def sample(run, xfs, group, started):
    return dict(elapsed_seconds=round(time.monotonic() - started, 3), private_run_allocated_bytes=allocated(run),
                dedicated_xfs_used_bytes=shutil.disk_usage(xfs).used,
                shared_data_disk_free_bytes=shutil.disk_usage(run).free,
                memory_current_bytes=int((group / "memory.current").read_text()),
                cgroup_memory_peak_bytes=int((group / "memory.peak").read_text()))


def tidy_ephemeral(run, native, xfs):
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
    redis = run / "redis"
    if redis.is_symlink():
        raise ValueError("original private Redis scratch was replaced")
    reject_mounts(redis)
    identity = redis.stat()
    remove_private_tree(redis, [identity.st_dev, identity.st_ino])


def execute(commit, xfs, run, lock):
    group = cgroup()
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
        admission = admit(commit, xfs)
        acceptance_receipts.write_json(run / "admission.json", admission)
        started = time.monotonic()
        history = []
        with (run / "native.log").open("xb") as log, (run / "resource-samples.jsonl").open("x") as samples:
            child = subprocess.Popen(
                ["bash", "scripts/python-entry.sh", "tools/proof_native_suite.py", "run",
                 "--scope", "stage1-protocol", "--output", str(native)],
                env=env, stdout=log, stderr=subprocess.STDOUT, pass_fds=(guard.fileno(),))
            while True:
                row = sample(run, xfs, group, started)
                history.append(row)
                samples.write(json.dumps(row) + "\n")
                samples.flush()
                try:
                    status = child.wait(timeout=SAMPLE_INTERVAL)
                except subprocess.TimeoutExpired:
                    continue
                break
            history.append(sample(run, xfs, group, started))
            samples.write(json.dumps(history[-1]) + "\n")

        # The cgroup is private. Check for leftover processes before touching
        # test scratch; the build manifest remains under the official handoff.
        remaining = {int(pid) for pid in (group / "cgroup.procs").read_text().split()} - {os.getpid()}
        events = {k: int(v) for k, v in (row.split() for row in (group / "memory.events").read_text().splitlines())}
        cleanup = "unknown" if remaining else "retained after native failure"
        if status == 0 and not remaining and not events["oom_kill"]:
            tidy_ephemeral(run, native, xfs)
            cleanup = "private temporary files removed"
        acceptance_receipts.write_json(run / "resources.json", dict(
            commit=commit, exit_code=status, elapsed_seconds=history[-1]["elapsed_seconds"],
            cgroup_memory_peak_bytes=history[-1]["cgroup_memory_peak_bytes"], memory_events=events,
            private_run_sampled_disk_peak_bytes=max(row["private_run_allocated_bytes"] for row in history),
            dedicated_xfs_sampled_used_peak_bytes=max(row["dedicated_xfs_used_bytes"] for row in history),
            shared_data_disk_min_free_bytes=min(row["shared_data_disk_free_bytes"] for row in history),
            remaining_processes=sorted(remaining), cleanup=cleanup))
        return status or (1 if remaining or events["oom_kill"] else 0)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("preflight", "run"))
    parser.add_argument("commit")
    parser.add_argument("xfs", type=Path)
    parser.add_argument("run", type=Path, nargs="?")
    parser.add_argument("lock", type=Path, nargs="?")
    args = parser.parse_args()
    if args.mode == "preflight":
        print(json.dumps(admit(args.commit, args.xfs), indent=2))
        return 0
    return execute(args.commit, args.xfs, args.run, args.lock)


if __name__ == "__main__":
    sys.exit(main())
