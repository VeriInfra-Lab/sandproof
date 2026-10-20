"""Run real Std BTreeSet native assertions in fresh committed private copies.

This is ordinary testing for ning. It does not run Lean, prove the standard
library, or require the approval-pending module audit files.
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
    from .check_btree_set_binding import ROOT, SOURCE, MANIFEST, mutate, sha
except ImportError:
    from check_btree_set_binding import ROOT, SOURCE, MANIFEST, mutate, sha

CRATE = "verification/extract-btree-set-binding"
TEST = CRATE + "/tests/insert_contract.rs"
NATIVE_INPUTS = [SOURCE, MANIFEST, CRATE + "/Cargo.lock", TEST,
                 "tools/check_btree_set_binding.py", "tools/check_btree_set_binding_components.py",
                 "tools/check_foundations_extraction.py", "tools/check_loop_callbacks_contract.py"]
PRODUCTION_RUSTC = "rustc 1.97.0 (2d8144b78 2026-07-07)"
CORRECT_SOURCE = "ae379c043d3247e47c261c684e5669a87c7313030be5f98a8a67a2d7722da8f7"
WRONG_SOURCE = "958049b09a411d2c78c1836c68d2ffac17b8fb1c88d5b3e5ad01db3c1fda8043"


def fresh_artifacts(log, target, source):
    rows = []
    for line in log.read_text().splitlines():
        try:
            row = json.loads(line)
        except ValueError:
            continue
        if row.get("reason") == "compiler-artifact":
            rows.append(row)
    result = {}
    for name, kind in [("sandproof_btree_set_binding", "lib"), ("insert_contract", "test")]:
        found = [r for r in rows if r["target"]["name"] == name and kind in r["target"]["kind"]]
        if len(found) != 1 or found[0]["fresh"] is not False:
            raise ValueError("Need the freshly compiled actual library/test: " + name)
        row = found[0]
        expected = source / CRATE / ("src/lib.rs" if kind == "lib" else "tests/insert_contract.rs")
        if (Path(row["manifest_path"]).resolve() != (source / MANIFEST).resolve()
                or Path(row["target"]["src_path"]).resolve() != expected.resolve()
                or not row["filenames"]
                or any(not Path(p).resolve().is_relative_to(target) for p in row["filenames"])):
            raise ValueError("Actual native package/source/target identity differs")
        result[name] = row
    if not result["insert_contract"].get("executable"):
        raise ValueError("Missing actual native test binary")
    return result


def reproduce(output, root=ROOT):
    root, output = Path(root).resolve(), Path(output).resolve()
    if subprocess.check_output(["rustc", "--version"], text=True).strip() != PRODUCTION_RUSTC:
        raise ValueError("Changed native compiler")
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
    for name in NATIVE_INPUTS:
        if subprocess.check_output(["git", "show", "HEAD:" + name], cwd=root) != (root / name).read_bytes():
            raise ValueError("Commit native inputs before testing: " + name)
    if sha(root / SOURCE) != CORRECT_SOURCE:
        raise ValueError("Original direct Std wrappers changed")
    os.umask(0o077)
    output.mkdir(parents=True, exist_ok=False)
    started = time.monotonic()
    archive = output / "source.tar"
    archive.write_bytes(subprocess.check_output(["git", "archive", "HEAD"], cwd=root))
    receipt = {"kind": "actual-Std-BTreeSet-native-component", "complete": False,
               "commit": commit, "source_archive_sha256": sha(archive),
               "inputs": {n: sha(root / n) for n in NATIVE_INPUTS},
               "rustc": subprocess.check_output(["rustc", "-vV"], text=True), "variants": {}}
    for variant in ["correct", "invert-flag"]:
        source = output / variant
        source.mkdir()
        subprocess.run(["tar", "-xf", str(archive), "-C", str(source)], check=True)
        path = source / SOURCE
        path.write_text(mutate(path.read_text(), variant))
        expected = CORRECT_SOURCE if variant == "correct" else WRONG_SOURCE
        if sha(path) != expected:
            raise ValueError("Native wrong source differs from formal wrong Rust")
        target = output / ("target-" + variant)
        env = os.environ | {"CARGO_TARGET_DIR": str(target), "CARGO_INCREMENTAL": "0",
                            "CARGO_NET_OFFLINE": "true", "RUST_TEST_THREADS": "1"}

        def run(command, tag):
            log = output / (variant + "-" + tag + ".log")
            with log.open("xb") as stream:
                code = subprocess.run(command, cwd=source, env=env, stdout=stream,
                                      stderr=subprocess.STDOUT, timeout=900).returncode
            return code, log

        code, log = run(["cargo", "test", "--manifest-path", MANIFEST, "--test", "insert_contract",
                         "--no-run", "--locked", "--offline", "--message-format=json"], "build")
        if code:
            raise ValueError("Compile failure is not a semantic refutation")
        artifacts = fresh_artifacts(log, target, source)
        binary = output / (variant + "-tests")
        shutil.copy2(artifacts["insert_contract"]["executable"], binary)
        marker = "" if variant == "correct" else "first_insert_reports_membership_and_writes_original_set"
        code, test_log = run([str(binary), marker, "--nocapture", "--test-threads=1"], "test")
        text = test_log.read_text()
        if variant == "correct":
            if code or "test result: ok. 6 passed; 0 failed; 0 ignored;" not in text:
                raise ValueError("Missing six original BTreeSet native assertions")
            extra = []
            for tag, command in [
                ("fmt", ["cargo", "fmt", "--manifest-path", MANIFEST, "--", "--check"]),
                ("clippy", ["cargo", "clippy", "--manifest-path", MANIFEST, "--all-targets",
                            "--locked", "--offline", "--", "-D", "warnings"]),
            ]:
                status, checked_log = run(command, tag)
                if status:
                    raise ValueError("Native ordinary check failed: " + tag)
                extra.append({"command": command, "exit_code": status, "log_sha256": sha(checked_log)})
            receipt["positive_checks"] = extra
        elif (code != 101 or "test result: FAILED. 0 passed; 1 failed; 0 ignored;" not in text
              or "BTreeSet insert flag must report a previously absent key" not in text
              or marker not in text):
            raise ValueError("Wrong Rust did not fail the same unchanged semantic assertion")
        receipt["variants"][variant] = {
            "source_sha256": sha(path), "test_exit_code": code, "test_filter": marker,
            "fresh_artifacts": artifacts, "binary_sha256": sha(binary),
            "build_log_sha256": sha(log), "test_log_sha256": sha(test_log)}
    receipt.update(complete=True, seconds=round(time.monotonic() - started, 3))
    (output / "result.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({"complete": True, "correct_cases": 6, "actual_wrong_versions": 1,
                      "seconds": receipt["seconds"]}))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    reproduce(args.output)
