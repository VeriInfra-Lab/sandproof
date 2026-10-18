"""Reproduce B1 engineering evidence in fresh copies; never edit production."""

import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile

from check_task_contract import ROOT, check, digest


def run(command, directory, log, env=None):
    with log.open("w") as output:
        process = subprocess.run(
            command, cwd=directory, env=env, stdout=output, stderr=subprocess.STDOUT,
            timeout=900, check=False,
        )
    return process.returncode


def change(path, old, new):
    text = path.read_text()
    if text.count(old) != 1:
        raise ValueError(f"Expected exactly one reviewed hook in {path}")
    path.write_text(text.replace(old, new))


def test_hook(root):
    path = root / "sandbox-host-service/src/lib.rs"
    original = path.read_bytes()
    path.write_bytes(original + b'\n#[cfg(test)]\n#[path = "../../verification/fixtures/task-contract-tests.rs"]\nmod task_contract_tests;\n')
    path = root / "controller-service/tests/support/internal_application.rs"
    needle = "    pub block_prepare: Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>,"
    change(path, needle, needle + "\n    pub block_disk: Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>,")
    change(path,
           "    fn available_disk_bytes(&self) -> Result<u64, ApplicationError> {\n        Ok(self.0.lock().unwrap().disk_available.unwrap_or(u64::MAX))\n    }",
           """    fn available_disk_bytes(&self) -> Result<u64, ApplicationError> {
        let mut state = self.0.lock().unwrap();
        if let Some((entered, release)) = state.block_disk.take() {
            entered.send(()).unwrap();
            release.recv_timeout(std::time::Duration::from_secs(15)).unwrap();
        }
        Ok(state.disk_available.unwrap_or(u64::MAX))
    }""")
    return hashlib.sha256(original).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True, help="New, isolated evidence directory")
    args = parser.parse_args()
    check()
    # Results must be reproducible from the exact committed inputs being cited.
    manifest = "verification/task-contract-inputs.sha256"
    frozen = subprocess.check_output(["git", "show", f"HEAD:{manifest}"], cwd=ROOT)
    if frozen != (ROOT / manifest).read_bytes():
        raise ValueError("Commit the reviewed inputs before the reproduction run")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    source = output / "source"
    source.mkdir()
    archive = subprocess.check_output(["git", "archive", "HEAD"], cwd=ROOT)
    with tarfile.open(fileobj=io.BytesIO(archive)) as tree:
        tree.extractall(source, filter="data")
    env = check(source)
    original_lib = test_hook(source)
    variants = {
        "correct": None,
        "ordinary-early": (
            "            let _permit = permit;\n            run(&app)",
            "            drop(permit);\n            run(&app)",
            "cancelled_waiter_keeps_same_budget_and_admission_busy_does_not_call",
        ),
        "status-early": (
            "            let _permits = permits;\n            app.internal_free_resources",
            "            drop(permits);\n            app.internal_free_resources",
            "status_running_cancel_keeps_both_real_permits_until_callback_exits",
        ),
        "wrong-pool": (
            "let permit = Arc::clone(&self.work)\n            .try_acquire_owned()\n            .map_err(|_| ApplicationError::Busy)?;",
            "let permit = Arc::clone(&self.status)\n            .try_acquire_owned()\n            .map_err(|_| ApplicationError::Busy)?;",
            "cancelled_waiter_keeps_same_budget_and_admission_busy_does_not_call",
        ),
    }
    receipt = {"commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
               "proof_kind": env["proof_kind"], "source_lib_sha256": original_lib, "variants": {}}
    receipt["rustc"] = subprocess.check_output(["rustc", "-vV"], cwd=ROOT, text=True)
    for name, mutation in variants.items():
        directory = output / name
        shutil.copytree(source, directory)
        path = directory / "sandbox-host-service/src/lib.rs"
        if mutation:
            change(path, mutation[0], mutation[1])
        build_env = os.environ | {"CARGO_TARGET_DIR": str(output / f"target-{name}"), "RUST_TEST_THREADS": "1"}
        log = output / f"{name}-build.log"
        code = run(["cargo", "test", "--locked", "--offline", "-p", "sandproof-host-service", "--lib",
                    "--no-run", "--message-format=json"], directory, log, build_env)
        if code != 0:
            raise ValueError(f"{name}: build failure is NOT a semantic rejection; see {log}")
        artifacts = [json.loads(line) for line in log.read_text().splitlines() if line.startswith("{")]
        binaries = [a for a in artifacts if a.get("reason") == "compiler-artifact"
                    and a.get("target", {}).get("name") == "sandproof_host_service" and a.get("executable")]
        if len(binaries) != 1 or binaries[0]["fresh"]:
            raise ValueError("Need one freshly compiled actual host test binary per variant")
        binary = Path(binaries[0]["executable"]).resolve()
        if not binary.is_relative_to(Path(build_env["CARGO_TARGET_DIR"]).resolve()):
            raise ValueError("Test executable escaped its independent target directory")
        filters = [("task_contract_tests::", 7), ("status_admission::tests::", 2)] if not mutation else [
            ("task_contract_tests::" + mutation[2], 1)]
        results = []
        for index, (test_filter, count) in enumerate(filters):
            test_log = output / f"{name}-test-{index}.log"
            command = [str(binary), test_filter, "--nocapture", "--test-threads=1"]
            if mutation:
                command.append("--exact")
            code = run(command, directory, test_log, build_env)
            text = test_log.read_text()
            if mutation:
                if code != 101 or f"test result: FAILED. 0 passed; {count} failed; 0 ignored;" not in text or "assertion" not in text:
                    raise ValueError(f"{name}: missing expected same-assertion rejection")
            elif code != 0 or re.findall(r"test result: ok\. (\d+) passed; 0 failed; 0 ignored;", text) != [str(count)]:
                raise ValueError(f"{name}: correct tests did not execute and pass")
            results.append({"filter": test_filter, "exit": code, "log_sha256": digest(test_log)})
        receipt["variants"][name] = {"build_exit": 0, "fresh_binary": True,
                                     "binary_sha256": digest(binary),
                                     "lib_with_test_hook_sha256": digest(path), "tests": results}
        (output / "result.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps(receipt, indent=2))


if __name__ == "__main__":
    main()
