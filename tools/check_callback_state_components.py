"""Reproduce callback-state native checks using immutable committed source copies.

Ordinary tests run on ning. Every variant must freshly compile the actual Store
library, callback library and test binary. Compilation failure is not a refutation.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

try:
    from .check_callback_state_contract import ROOT, SOURCE, FREEZE, check, sha
except ImportError:
    from check_callback_state_contract import ROOT, SOURCE, FREEZE, check, sha

VARIANTS = ("correct", "no-write-toggle", "double-record")


def mutate(source, variant):
    if variant == "correct":
        return source
    before, after = {
        "no-write-toggle": ("    *state = !*state;\n    *state", "    !*state"),
        "double-record": ("    visited.push((*key, *value));",
                          "    visited.push((*key, *value));\n    visited.push((*key, *value));"),
    }[variant]
    if source.count(before) != 1:
        raise ValueError("Ambiguous actual callback mutation")
    return source.replace(before, after)


def fresh_artifacts(log, target, source):
    rows = []
    for line in log.read_text().splitlines():
        try:
            row = json.loads(line)
        except ValueError:
            continue
        if row.get("reason") == "compiler-artifact":
            rows.append(row)
    selected = {}
    for name, kind in (("sandbox_controller_core", "lib"),
                       ("sandproof_callback_state_extract", "lib"),
                       ("contract", "test")):
        candidates = [row for row in rows if row["target"]["name"] == name
                      and kind in row["target"]["kind"]]
        if len(candidates) != 1 or candidates[0]["fresh"] is not False:
            raise ValueError("Need a fresh unambiguous actual library/binary: " + name)
        row = candidates[0]
        package = (source / "controller-core" if name == "sandbox_controller_core"
                   else source / "verification/extract-callback-state")
        expected_source = (package / "tests/contract.rs" if kind == "test"
                           else package / "src/lib.rs")
        if (Path(row.get("manifest_path", "")).resolve() != (package / "Cargo.toml").resolve()
                or Path(row["target"].get("src_path", "")).resolve() != expected_source.resolve()):
            raise ValueError("Actual compiled package/source identity differs: " + name)
        if not row["filenames"] or any(not Path(p).resolve().is_relative_to(target)
                                       for p in row["filenames"]):
            raise ValueError("Artifact escaped the registered private target")
        selected[name] = row
    if not selected["contract"].get("executable"):
        raise ValueError("Missing real callback test executable")
    return selected


def frozen_head(root, contract):
    for name in contract["inputs"] + [FREEZE]:
        if subprocess.check_output(["git", "show", "HEAD:" + name], cwd=root) != (root / name).read_bytes():
            raise ValueError("Commit all frozen callback inputs before reproduction: " + name)
    return subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()


def reproduce(output, root=ROOT):
    root = root.resolve()
    contract = check(root)
    commit = frozen_head(root, contract)
    compiler = subprocess.check_output(["rustc", "--version"], text=True).strip()
    if compiler != contract["production_compiler"]:
        raise ValueError("Changed native compiler")
    os.umask(0o077)
    output.mkdir(parents=True, exist_ok=False)
    started = time.monotonic()
    archive = subprocess.check_output(["git", "archive", "HEAD"], cwd=root)
    archive_path = output / "source.tar"
    archive_path.write_bytes(archive)
    target = Path(os.environ.get("CARGO_TARGET_DIR", str(output / "target"))).resolve()
    env = os.environ | {"CARGO_TARGET_DIR": str(target), "CARGO_INCREMENTAL": "0",
                        "CARGO_NET_OFFLINE": "true", "RUST_TEST_THREADS": "1"}
    receipt = {"kind": "actual-callback-state-native-component", "commit": commit,
               "complete": False, "source_archive_sha256": sha(archive_path),
               "rustc": subprocess.check_output(["rustc", "-vV"], text=True), "variants": {}}

    def run(args, directory, log):
        with log.open("xb") as stream:
            code = subprocess.run(args, cwd=directory, env=env, stdout=stream,
                                  stderr=subprocess.STDOUT, timeout=900).returncode
        return code

    for variant in VARIANTS:
        directory = output / variant
        directory.mkdir()
        subprocess.run(["tar", "-xf", str(archive_path), "-C", str(directory)], check=True)
        path = directory / SOURCE
        if path.read_bytes() != (root / SOURCE).read_bytes():
            raise ValueError("Source export differs from frozen actual helper")
        path.write_text(mutate(path.read_text(), variant))
        expected = (contract["helper_source_sha256"] if variant == "correct"
                    else contract["wrong_variants"][variant]["mutated_source_sha256"])
        if sha(path) != expected:
            raise ValueError("Native wrong body differs from the formal wrong body")
        manifest = "verification/extract-callback-state/Cargo.toml"
        build_log = output / (variant + "-build.log")
        if run(["cargo", "test", "--manifest-path", manifest, "--test", "contract", "--locked",
                "--offline", "--no-run", "--message-format=json"], directory, build_log):
            raise ValueError("Compile failure is not a semantic refutation: " + variant)
        artifacts = fresh_artifacts(build_log, target, directory)
        binary = output / (variant + "-tests")
        shutil.copy2(artifacts["contract"]["executable"], binary)
        marker = {"correct": "", "no-write-toggle": "toggle_mutates_the_external_state_and_returns_it",
                  "double-record": "trace_preserves_prefix_and_appends_exactly_one_full_row"}[variant]
        log = output / (variant + "-test.log")
        code = run([str(binary), marker, "--nocapture", "--test-threads=1"], directory, log)
        text = log.read_text()
        if variant == "correct":
            if code or "test result: ok. 12 passed; 0 failed; 0 ignored;" not in text:
                raise ValueError("Missing all twelve real callback checks")
            checks = []
            for tag, command in [
                ("fmt", ["cargo", "fmt", "--manifest-path", manifest, "--", "--check"]),
                ("clippy", ["cargo", "clippy", "--manifest-path", manifest, "--all-targets",
                            "--locked", "--offline", "--", "-D", "warnings"]),
                ("original-retain", ["cargo", "test", "--manifest-path", manifest, "-p",
                                     "sandbox-controller-core", "--lib", "--locked", "--offline",
                                     "store::tests::retain", "--", "--nocapture"]),
            ]:
                checked_log = output / (tag + ".log")
                status = run(command, directory, checked_log)
                if status:
                    raise ValueError("Positive ordinary check failed: " + tag)
                if tag == "original-retain" and "test result: ok. 2 passed; 0 failed; 0 ignored;" not in checked_log.read_text():
                    raise ValueError("Missing the two original retain tests")
                checks.append({"command": command, "exit_code": status, "log_sha256": sha(checked_log)})
            receipt["positive_checks"] = checks
        else:
            message = ("callback must write the original external state" if variant == "no-write-toggle"
                       else "assertion `left == right` failed")
            if (code != 101 or "test result: FAILED. 0 passed; 1 failed; 0 ignored;" not in text
                    or marker not in text or message not in text):
                raise ValueError("Wrong Rust did not fail the same unchanged assertion")
        receipt["variants"][variant] = {
            "build_exit_code": 0, "test_exit_code": code, "test_filter": marker,
            "source_sha256": sha(path), "store_sha256": sha(directory / "controller-core/src/store.rs"),
            "fresh_artifacts": artifacts, "binary_sha256": sha(binary),
            "build_log_sha256": sha(build_log), "test_log_sha256": sha(log),
        }
        (output / "result.json").write_text(json.dumps(receipt, indent=2) + "\n")
    if check(root) != contract or frozen_head(root, contract) != commit:
        raise ValueError("Frozen inputs changed during native checks")
    receipt.update(complete=True, exit_code=0, seconds=round(time.monotonic() - started, 3))
    (output / "result.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({"complete": True, "correct_cases": 12, "original_cases": 2,
                      "wrong_variants": 2, "seconds": receipt["seconds"]}))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    reproduce(parser.parse_args().output.resolve())
