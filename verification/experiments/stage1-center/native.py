"""Ordinary component checks on the test host; no proof or product VM claims."""
from pathlib import Path
import argparse
import hashlib
import json
import os
import shutil
import subprocess
import time
import traceback

ROOT = Path(__file__).resolve().parents[3]


def sources():
    # The original docs checker also requires the checkout/export's Git index.
    # Preserve tracked fixtures even if a nested directory is named artifacts.
    names = subprocess.check_output(["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"], cwd=ROOT).split(b"\0")
    result = {}
    for raw in sorted(set(names)):
        if not raw:
            continue
        name = raw.decode()
        path = ROOT / name
        if not path.is_file() or path.is_symlink():
            raise ValueError("source input is not a regular file: " + name)
        result[name] = hashlib.sha256(path.read_bytes()).hexdigest()
    return result


def run(output, target):
    output.mkdir(mode=0o700, parents=True)
    env = dict(os.environ, CARGO_TARGET_DIR=str(target), CARGO_NET_OFFLINE="true", CARGO_BUILD_JOBS="4")
    record = {"kind": "stage1-center-native-component-not-full-acceptance", "steps": [], "mutants": [], "sources": sources()}
    started = time.monotonic()

    def command(name, argv, cwd=ROOT, expected=0):
        now = time.monotonic()
        log = output / (name + ".log")
        with log.open("xb") as stream:
            result = subprocess.run(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                                    stdout=stream, stderr=subprocess.STDOUT, timeout=1200)
        row = {"name": name, "argv": argv, "cwd": str(cwd), "exit_code": result.returncode,
               "seconds": time.monotonic() - now,
               "log_sha256": hashlib.sha256(log.read_bytes()).hexdigest()}
        record["steps"].append(row)
        (output / "progress.json").write_text(json.dumps(record, indent=2) + "\n")
        if result.returncode != expected:
            raise RuntimeError(name + " did not return expected " + str(expected))
        return log

    try:
        command("format", ["cargo", "fmt", "--all", "--", "--check"])
        command("extract-format", ["cargo", "fmt", "--manifest-path", "verification/experiments/stage1-center/Cargo.toml", "--", "--check"])
        command("center", ["cargo", "test", "-p", "sandproof-center-core", "--locked", "--offline"])
        command("store", ["cargo", "test", "-p", "sandproof-center-store", "--features", "redis-tests", "--locked", "--offline", "--", "--test-threads=1"])
        command("clippy", ["cargo", "clippy", "-p", "sandproof-center-core", "-p", "sandproof-center-store", "--all-targets", "--features", "sandproof-center-store/redis-tests", "--locked", "--offline", "--", "-D", "warnings"])
        command("docs", ["bash", "scripts/python-entry.sh", "tools/check_docs.py"])
        for name, before, after, test in [
            ("key-only", "if self.submission != *submission {", "if self.submission.key() != submission.key() {",
             "changed_full_submission_and_foreign_owner_do_not_reuse_key"),
            ("skip-version", "if expected != current {", "if expected != current && false {",
             "new_versions_preserve_successor_and_reject_stale_or_exhausted"),
        ]:
            project = output / ("wrong-" + name)
            project.mkdir(mode=0o700)
            for source_name in record["sources"]:
                destination = project / source_name
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(ROOT / source_name, destination)
            source = project / "sandbox-center-core/src/lifecycle.rs"
            original = source.read_text()
            if original.count(before) != 1:
                raise ValueError("mutation anchor changed: " + name)
            source.write_text(original.replace(before, after))
            row = {"name": name, "test": test, "source_sha256": hashlib.sha256(source.read_bytes()).hexdigest()}
            record["mutants"].append(row)
            log = command(name + "-build", ["cargo", "test", "-p", "sandproof-center-core", "--test", "lifecycle", "--no-run", "--message-format=json", "--locked", "--offline"], project)
            artifacts = []
            for line in log.read_text().splitlines():
                if not line.startswith("{"):
                    continue
                item = json.loads(line)
                if item.get("reason") == "compiler-artifact" and item.get("manifest_path") == str(project / "sandbox-center-core/Cargo.toml"):
                    artifacts.append(item)
            library = [a for a in artifacts if a["target"]["kind"] == ["lib"]]
            tests = [a for a in artifacts if a["target"]["name"] == "lifecycle" and a.get("executable")]
            if len(library) != 1 or len(tests) != 1 or library[0]["fresh"] or tests[0]["fresh"]:
                raise ValueError("actual changed library and test must both be newly compiled")
            if library[0]["target"]["src_path"] != str(project / "sandbox-center-core/src/lib.rs"):
                raise ValueError("wrong business library source")
            executable = Path(tests[0]["executable"])
            row["artifacts"] = artifacts
            row["executable_sha256"] = hashlib.sha256(executable.read_bytes()).hexdigest()
            kept = output / "binaries" / name
            kept.mkdir(mode=0o700, parents=True)
            row["preserved_artifacts"] = {}
            for filename in [*library[0]["filenames"], str(executable)]:
                original_binary = Path(filename)
                destination = kept / original_binary.name
                shutil.copy2(original_binary, destination)
                row["preserved_artifacts"][str(destination)] = hashlib.sha256(destination.read_bytes()).hexdigest()
            failure = command(name + "-assertion", [str(executable), test, "--exact", "--nocapture"], project, expected=101)
            text = failure.read_text()
            if "assertion" not in text or "test result: FAILED. 0 passed; 1 failed;" not in text:
                raise ValueError("test did not reject the mutation at its original assertion")
            row["same_assertion_rejected"] = True
            # Remove only this changed package's build results, preserving all
            # original dependency caches. Never clean Lean or global processes.
            command(name + "-package-reset", ["cargo", "clean", "-p", "sandproof-center-core"], project)
        command("restore-correct", ["cargo", "test", "-p", "sandproof-center-core", "--test", "lifecycle", "--locked", "--offline"])
        if sources() != record["sources"]:
            raise ValueError("original source inputs changed during checks")
        record["success"] = True
    except BaseException as error:
        record.update(success=False, error=repr(error), traceback=traceback.format_exc())
    record["seconds"] = time.monotonic() - started
    (output / "result.json").write_text(json.dumps(record, indent=2) + "\n")
    return record


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--target", required=True, type=Path)
    args = parser.parse_args()
    result = run(args.output.resolve(), args.target.resolve())
    print(json.dumps({k: v for k, v in result.items() if k not in ("sources", "mutants")}, indent=2))
    raise SystemExit(0 if result["success"] else 1)
