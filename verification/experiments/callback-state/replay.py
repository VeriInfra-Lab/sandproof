"""Re-extract real callback helpers and check positive/wrong bodies on a6000.

This component is independent of shared registration; the hard module entry
also requires the sealed ning callback-state receipt. No whole FnMut refinement.
"""
import argparse
import importlib.util
import json
import os
import re
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import io
import time

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from tools.check_callback_state_contract import (
    check, check_extraction, DEST, LLBC, SOURCE, LAKE_EXTENSION, sha)
from tools.check_callback_state_components import mutate, frozen_head, VARIANTS
from tools.check_first_chain_extraction import check_tools

# Reuse the existing complete/non-stale actual-file diagnostic and exact-audit
# admission. This original file is a frozen input; it is not copied or weakened.
_shared_spec = importlib.util.spec_from_file_location(
    "_callback_query_replay", ROOT / "verification/experiments/loop-callbacks/replay.py")
_shared = importlib.util.module_from_spec(_shared_spec)
_shared_spec.loader.exec_module(_shared)
run, service, library_sources = _shared.run, _shared.service, _shared.library_sources


def prepare(root, archive, destination):
    destination.mkdir()
    with tarfile.open(fileobj=io.BytesIO(archive)) as tree:
        tree.extractall(destination, filter="data")
    (destination / ".tools").symlink_to((root / ".tools").resolve(), target_is_directory=True)
    (destination / ".lake").mkdir()
    (destination / ".lake/packages").symlink_to(
        (root / ".lake/packages").resolve(), target_is_directory=True)
    lake = (root / "lakefile.toml").read_text()
    if 'name = "SandproofCallbackState"' not in lake:
        lake += LAKE_EXTENSION
    (destination / "lakefile.toml").write_text(lake)
    return destination


def reproduce(root, tool_root, output):
    root, tool_root, output = root.resolve(), tool_root.resolve(), output.resolve()
    contract = check(root)
    commit = frozen_head(root, contract)
    check_tools(root, tool_root)
    if subprocess.check_output(["git", "-C", str(root / ".lake/packages/aeneas"),
                                "status", "--porcelain", "--untracked-files=no"], text=True):
        raise ValueError("Fixed Lean backend has tracked modifications")
    libraries = library_sources(root, tool_root)
    output.mkdir(parents=True, exist_ok=False)
    started = time.monotonic()
    archive = subprocess.check_output(["git", "archive", "HEAD"], cwd=root)
    (output / "source.tar").write_bytes(archive)
    tracked = subprocess.check_output(["git", "ls-files", "-z"], cwd=root, text=True).split("\0")
    rust_inputs = {name: sha(root / name) for name in tracked if name.endswith(".rs")}
    env = os.environ | {"SANDPROOF_PROOF_TOOL_ROOT": str(tool_root)}
    receipt = {"kind": "actual-callback-helpers-and-conditional-state-connection",
               "commit": commit, "source_archive_sha256": sha(output / "source.tar"),
               "library_sources": libraries, "variants": {}, "complete": False}
    for variant in VARIANTS:
        begin = time.monotonic()
        out = output / variant
        out.mkdir()
        project = prepare(root, archive, out / "source")
        source = project / SOURCE
        source.write_text(mutate(source.read_text(), variant))
        expected_source = (contract["helper_source_sha256"] if variant == "correct"
                           else contract["wrong_variants"][variant]["mutated_source_sha256"])
        if sha(source) != expected_source:
            raise ValueError("Wrong formal body differs from frozen actual Rust")
        for name, expected in rust_inputs.items():
            if name != SOURCE and sha(project / name) != expected:
                raise ValueError("Other real Rust changed in private export: " + name)
        one = {"source_sha256": sha(source), "commands": []}
        one["commands"].append(run(["bash", "scripts/regenerate-callback-state-lean.sh", variant],
                                   project, env, out / "generate.log"))
        selected = contract if variant == "correct" else contract | contract["wrong_variants"][variant]["extraction"]
        llbc = json.loads((project / LLBC).read_text())
        inventory = json.loads((project / DEST / "translation.json").read_text())
        check_extraction(project, llbc, inventory, selected)
        if variant == "correct":
            audit = "SandboxModel/Foundations/CallbackState/Audit.lean"
            one["commands"].append(run(["lake", "build",
                "SandboxModel.Foundations.CallbackState.Audit",
                "SandboxModel.Foundations.CallbackState"], project, env, out / "build.log"))
            files = ["SandboxModel/Foundations/CallbackState.lean"] + [
                "SandboxModel/Foundations/CallbackState/" + name + ".lean"
                for name in contract["model_modules"]]
            expected_roots = contract["theorems"]
        else:
            info = contract["wrong_variants"][variant]
            audit = "SandboxModel/Foundations/CallbackState/Counterexample.lean"
            (project / audit).write_bytes((root / info["template"]).read_bytes())
            one["commands"].append(run(["lake", "build",
                "SandboxModel.Foundations.CallbackState.Counterexample"], project, env, out / "build.log"))
            files, expected_roots = [audit], info["theorems"]
        one["commands"].append(run(["lake", "env", "lean", audit],
                                   project, env, out / "kernel-audit.log"))
        one["service"] = service(project, files, expected_roots, out, env)
        # Forward the original, already admitted audit messages to the shared
        # batch. Integration must register both negative templates as expected
        # roots as well; missing registration must not silently omit a mutant.
        for line in (out / "service.log").read_text().splitlines():
            try:
                record = json.loads(line)
            except ValueError:
                continue
            if record.get("request", {}).get("tool") != "lean_diagnostic_messages":
                continue
            result = record["result"]
            for _ in range(4):
                if "success" in result:
                    break
                result = result.get("structuredContent", result.get("result", {}))
            for item in result.get("items", []):
                message = item.get("message", "")
                if re.fullmatch(r"Checked (\S+): \d+ standard logical dependencies", message):
                    print(message, flush=True)
        one.update(success=True, actual_llbc_sha256=sha(project / LLBC),
                   seconds=round(time.monotonic() - begin, 3))
        receipt["variants"][variant] = one
        (out / "result.json").write_text(json.dumps(one, indent=2) + "\n")
    if check(root) != contract or frozen_head(root, contract) != commit:
        raise ValueError("Callback inputs changed during formal replay")
    check_tools(root, tool_root)
    receipt.update(complete=True, exit_code=0, seconds=round(time.monotonic() - started, 3))
    (output / "result.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({"complete": True, "correct_audits": len(contract["theorems"]),
                      "wrong_variants": 2, "seconds": receipt["seconds"]}))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--tool-root", type=Path, default=Path(os.environ.get(
        "SANDPROOF_PROOF_TOOL_ROOT", str(ROOT / ".tools"))))
    args = parser.parse_args()
    reproduce(ROOT, args.tool_root, args.output)
