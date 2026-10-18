"""Test actual B3 Rust and a work-first mutant from one frozen committed tree."""

import argparse
import io
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile

from check_revocation_contract import ROOT, check, digest


def run(command, directory, log, env):
    with log.open("w") as output:
        return subprocess.run(command, cwd=directory, env=env, stdout=output,
                              stderr=subprocess.STDOUT, timeout=900, check=False).returncode


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    contract = check()
    manifest = "verification/revocation-contract-inputs.sha256"
    if subprocess.check_output(["git", "show", f"HEAD:{manifest}"], cwd=ROOT) != (ROOT / manifest).read_bytes():
        raise ValueError("Commit the frozen B3 inputs before component reproduction")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    source = output / "source"
    source.mkdir()
    archive = subprocess.check_output(["git", "archive", "HEAD"], cwd=ROOT)
    with tarfile.open(fileobj=io.BytesIO(archive)) as tree:
        tree.extractall(source, filter="data")
    check(source)
    receipt = {"commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
               "proof_kind": contract["proof_kind"], "variants": {},
               "rustc": subprocess.check_output(["rustc", "-vV"], text=True)}
    for name in ["correct", "work-first"]:
        directory = output / name
        shutil.copytree(source, directory)
        path = directory / "sandbox-data-transport/src/revocation.rs"
        if name == "work-first":
            old = "        _ = signal.wait_for(|revoked| *revoked) => None,\n        result = work => Some(result),"
            new = "        result = work => Some(result),\n        _ = signal.wait_for(|revoked| *revoked) => None,"
            text = path.read_text()
            if text.count(old) != 1:
                raise ValueError("Expected one reviewed select-order mutation hook")
            path.write_text(text.replace(old, new))
        env = os.environ | {"CARGO_TARGET_DIR": str(output / f"target-{name}"), "RUST_TEST_THREADS": "1"}
        build_log = output / f"{name}-build.log"
        code = run(["cargo", "test", "--locked", "--offline", "-p", "sandproof-data-transport", "--lib",
                    "--no-run", "--message-format=json"], directory, build_log, env)
        if code != 0:
            raise ValueError(f"{name}: compile failure is not a semantic rejection; see {build_log}")
        artifacts = [json.loads(line) for line in build_log.read_text().splitlines() if line.startswith("{")]
        binaries = [a for a in artifacts if a.get("reason") == "compiler-artifact"
                    and a.get("target", {}).get("name") == "sandproof_data_transport" and a.get("executable")]
        if len(binaries) != 1 or binaries[0]["fresh"]:
            raise ValueError("Need one freshly compiled real Rust test binary per variant")
        binary = Path(binaries[0]["executable"]).resolve()
        if not binary.is_relative_to(Path(env["CARGO_TARGET_DIR"]).resolve()):
            raise ValueError("Component binary escaped its independent target")
        test_filter = "revocation::" if name == "correct" else (
            "revocation::tests::prior_revocation_or_lost_authority_never_polls_work")
        command = [str(binary), test_filter, "--nocapture", "--test-threads=1"]
        if name != "correct":
            command.append("--exact")
        log = output / f"{name}-test.log"
        code = run(command, directory, log, env)
        text = log.read_text()
        if name == "correct":
            if code != 0 or re.findall(r"test result: ok\. (\d+) passed; 0 failed; 0 ignored;", text) != ["9"]:
                raise ValueError("All nine actual B3 tests must execute and pass")
        elif code != 101 or "test result: FAILED. 0 passed; 1 failed; 0 ignored;" not in text or "assertion" not in text:
            raise ValueError("Work-first mutant did not fail the same original priority assertion")
        receipt["variants"][name] = {"build_exit": 0, "fresh_binary": True,
                                     "source_sha256": digest(path), "binary_sha256": digest(binary),
                                     "test_exit": code, "test_filter": test_filter, "log_sha256": digest(log)}
        (output / "result.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps(receipt, indent=2))


if __name__ == "__main__":
    main()
