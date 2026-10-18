"""Run strict native checks on a fresh exact candidate, without extraction roots."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time
from prepare import prepare


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-base", type=Path, required=True)
    parser.add_argument("--destination", type=Path, required=True)
    args = parser.parse_args()
    destination = args.destination.absolute()
    if destination.exists() or destination.is_symlink():
        raise ValueError("native destination exists")
    destination.mkdir(parents=True)
    source, evidence = destination / "source", destination / "evidence"
    evidence.mkdir()
    profile = prepare(args.source_base, source)
    files = subprocess.check_output(["git", "ls-files"], cwd=source, text=True).splitlines()
    files = sorted(set(files) | set(profile["candidate_files"]))
    before = {name: sha(source / name) for name in files}
    record = dict(base=profile["base"], patch_sha256=profile["patch_sha256"], stages=[])
    stages = [
        ("format", ["cargo", "fmt", "--all", "--", "--check"]),
        ("clippy", ["cargo", "clippy", "-p", "sandproof-controller-service", "--lib", "--tests",
                    "--locked", "--offline", "--", "-D", "warnings"]),
        ("cache-tests", ["cargo", "test", "-p", "sandproof-controller-service", "--locked", "--offline", "--lib",
                         "controller::cache_tests", "--", "--test-threads=1"]),
    ]
    (evidence / "pid").write_text(str(os.getpid()) + "\n")
    for stage, command in stages:
        print("START", stage, flush=True)
        started = time.monotonic()
        with (evidence / (stage + ".log")).open("xb") as output:
            process = subprocess.Popen(command, cwd=source, stdout=output, stderr=subprocess.STDOUT)
            (evidence / (stage + ".pid")).write_text(str(process.pid) + "\n")
            code = process.wait()
        record["stages"].append(dict(stage=stage, command=command, exit_code=code,
                                     seconds=round(time.monotonic()-started, 3)))
        (evidence / "result.json").write_text(json.dumps(record, indent=2) + "\n")
        print("END", stage, code, flush=True)
        if code:
            raise SystemExit(code)
    if "16 passed; 0 failed; 0 ignored" not in (evidence / "cache-tests.log").read_text():
        raise ValueError("original exact 16-test cache suite was not fully executed")
    if {name: sha(source / name) for name in files} != before:
        raise ValueError("candidate native inputs changed")
    record.update(exit_code=0, source_unchanged=True, input_count=len(files), tests=16, ignored=0)
    (evidence / "result.json").write_text(json.dumps(record, indent=2) + "\n")


if __name__ == "__main__":
    main()
