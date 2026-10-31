"""Replay only this component's ordinary checks on the ordinary test host."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import time
import traceback


CASES = [
    ("terminal-pause", "controller-core/src/transition.rs",
     "if operation.kind == OperationKind::Pause {\n        let mut updated = record.clone();",
     "if operation.kind == OperationKind::Kill {\n        let mut updated = record.clone();",
     "ordinary_failure_keeps_live_original_and_invalidates_already_queued_freeze"),
    ("drop-material-use", "controller-core/src/capture.rs",
     "pub fn ordinary_failure(mut self) -> Self {",
     "pub fn ordinary_failure(mut self) -> Self {\n        self.capture_work = false;\n        self.unload_source = false;",
     "finite_capture_state_space_preserves_uses_and_never_revives_failed_registration"),
    ("revive-revoked-capture", "controller-core/src/capture.rs",
     "if self.eligibility == RestoreEligibility::Capturing {\n            self.eligibility = RestoreEligibility::Recoverable;\n        }",
     "self.eligibility = RestoreEligibility::Recoverable;",
     "commit_dispatch_is_unknown_and_late_success_tracks_material_without_reviving_capture"),
    ("ignore-capture-target", "controller-core/src/worker/late_capture.rs",
     "if generation != plan.generation\n        || plan.target_snapshot != Some(RestoreSource { snapshot, manifest })\n    {",
     "if generation != plan.generation {",
     "late_commit_rejects_a_different_original_target_without_changing_world"),
]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def mutate(root, case):
    _, relative, before, after, _ = case
    path = root / relative
    text = path.read_text()
    if text.count(before) != 1:
        raise ValueError("Mutation no longer identifies exactly one real implementation")
    path.write_text(text.replace(before, after))
    return sha(path)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--archive-sha256", required=True)
    parser.add_argument("--head", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--lock", type=Path, required=True)
    args = parser.parse_args()
    if sha(args.archive) != args.archive_sha256:
        raise ValueError("Source archive changed")
    if not os.environ.get("CARGO_HOME"):
        raise ValueError("Set the approved offline CARGO_HOME explicitly")
    os.umask(0o077)
    out = args.output.resolve()
    out.mkdir(mode=0o700)
    source = out / "source"
    source.mkdir()
    with tarfile.open(args.archive) as tree:
        tree.extractall(source, filter="data")
    started = time.monotonic()
    result = dict(head=args.head, archive_sha256=sha(args.archive), complete=False,
                  commands=[], variants=[])
    env = dict(os.environ, CARGO_NET_OFFLINE="true", CARGO_INCREMENTAL="0",
               CARGO_BUILD_JOBS="4", RUST_TEST_THREADS="1",
               CARGO_TARGET_DIR=str(out / "target-correct"), TMPDIR=str(out / "tmp"))
    (out / "tmp").mkdir()

    def run(label, argv, root, current_env, expected=0):
        start = time.monotonic()
        process = subprocess.run(argv, cwd=root, env=current_env, stdin=subprocess.DEVNULL,
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        (out / (label + ".stdout")).write_bytes(process.stdout)
        (out / (label + ".stderr")).write_bytes(process.stderr)
        result["commands"].append(dict(label=label, argv=argv, cwd=str(root),
            exit_code=process.returncode, seconds=time.monotonic()-start,
            stdout_sha256=sha(out / (label + ".stdout")),
            stderr_sha256=sha(out / (label + ".stderr"))))
        if process.returncode != expected:
            raise ValueError("Unexpected command result: " + label)
        return process

    def binaries(data, root):
        found = []
        for line in data.splitlines():
            try:
                row = json.loads(line)
            except (ValueError, UnicodeError):
                continue
            if row.get("reason") == "compiler-artifact" and row.get("executable"):
                if row["manifest_path"] != str(root / "controller-core/Cargo.toml"):
                    raise ValueError("Test binary belongs to another source")
                found.append(dict(path=row["executable"], sha256=sha(Path(row["executable"])),
                                  fresh=row["fresh"], package_id=row["package_id"], target=row["target"]))
        if not found:
            raise ValueError("No actual test binaries")
        return found

    try:
        with args.lock.open("a") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            run("format", ["cargo", "fmt", "--all", "--", "--check"], source, env)
            run("format-extract", ["cargo", "fmt", "--manifest-path",
                "verification/extract-capture-handoff/Cargo.toml", "--", "--check"], source, env)
            process = run("core-tests", ["cargo", "test", "--locked", "--offline", "-p",
                "sandbox-controller-core", "--all-features", "--message-format=json"], source, env)
            result["positive_binaries"] = binaries(process.stdout, source)
            run("core-clippy", ["cargo", "clippy", "--locked", "--offline", "-p",
                "sandbox-controller-core", "--all-features", "--all-targets", "--", "-D", "warnings"], source, env)
            run("extract-clippy", ["cargo", "clippy", "--locked", "--offline", "--manifest-path",
                "verification/extract-capture-handoff/Cargo.toml", "--all-targets", "--", "-D", "warnings"], source, env)
            for case in CASES:
                name, relative, _, _, expected = case
                wrong = out / ("wrong-" + name)
                shutil.copytree(source, wrong)
                digest = mutate(wrong, case)
                variant_env = env | {"CARGO_TARGET_DIR": str(out / ("target-" + name))}
                process = run(name + "-build", ["cargo", "test", "--locked", "--offline", "-p",
                    "sandbox-controller-core", "--all-features", "--lib", "--no-run", "--message-format=json"], wrong, variant_env)
                built = binaries(process.stdout, wrong)
                if len(built) != 1 or built[0]["fresh"]:
                    raise ValueError("Wrong implementation did not produce a fresh binary")
                process = run(name + "-assertions", [built[0]["path"], "capture::tests::",
                    "--test-threads=1"], wrong, variant_env, 101)
                if (expected.encode() + b" ... FAILED" not in process.stdout or
                    b"panicked at" not in process.stdout or b"test result: FAILED." not in process.stdout):
                    raise ValueError("Expected original assertion did not reject the real mutant")
                result["variants"].append(dict(name=name, source=relative, source_sha256=digest,
                    binary=built[0], build_exit=0, assertion_exit=101, expected_failure=expected))
            result["rustc_info"] = json.loads((out / "target-correct/.rustc_info.json").read_text())
        result["complete"] = True
    except BaseException as error:
        result.update(error=repr(error), traceback=traceback.format_exc())
    result["seconds"] = time.monotonic() - started
    (out / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    if not result["complete"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
