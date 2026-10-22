"""Re-extract and audit this value-method component on the proof host.

This is not the shared V0 gate or a proof of the World handler. The handler
root is re-extracted as a preflight; its external template is never imported.
"""
import argparse
import fcntl
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import time
import tomllib
import traceback

from native import CASES, mutate, sha


PREFIX = "SandboxModel.Lifecycle.CaptureHandoffProof."
PROOFS = {
    "SandboxModel/Lifecycle/CaptureHandoff/Conditions.lean": [],
    "SandboxModel/Lifecycle/CaptureHandoff/Values.lean": [
        PREFIX + name for name in ["material_projection", "failure_preserves_uses",
        "late_success_safe", "unknown_failure_preserves_unknown", "normal_handoff_accepts",
        "revoked_late_success_accepts_material"]],
    "verification/experiments/capture-handoff/model/DropMaterial.lean": [
        PREFIX + "DropRejected.same_property_rejects"],
    "verification/experiments/capture-handoff/model/ReviveEligibility.lean": [
        PREFIX + "ReviveRejected.same_property_rejects",
        PREFIX + "ReviveRejected.actual_late_success_grants_stop"],
}
LIBRARIES = [
    ("CaptureHandoff", "verification/aeneas/capture-handoff"),
    ("CaptureHandoffDrop", "verification/aeneas/capture-handoff/wrong-drop"),
    ("CaptureHandoffRevive", "verification/aeneas/capture-handoff/wrong-revive"),
]
ROOTS = ["accepted", "dispatch", "register", "observe", "pause_failure", "terminate",
         "stop_permit", "registration_allowed", "commit", "eligibility", "material_uses"]
TOOLS = {
    "aeneas/charon/bin/charon": "2603f720f0d26a4c77ae215485e2457029d69504dbd9d318a497ed866f21d4fd",
    "aeneas/bin/aeneas": "22b326476f854e19b52ca5a2acfd953fd186da7694cd10d8d008ed5502f7c46a",
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", type=Path, required=True)
    parser.add_argument("--archive-sha256", required=True)
    parser.add_argument("--head", required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--lock", type=Path, required=True)
    parser.add_argument("--tools", type=Path, required=True)
    parser.add_argument("--packages", type=Path, required=True)
    args = parser.parse_args()
    if sha(args.archive) != args.archive_sha256:
        raise ValueError("Source archive changed")
    tools = args.tools.resolve()
    for path, expected in TOOLS.items():
        if sha(tools / path) != expected:
            raise ValueError("Fixed proof binary changed: " + path)
    os.umask(0o077)
    out = args.output.resolve()
    out.mkdir(mode=0o700)
    source = out / "source"
    source.mkdir()
    with tarfile.open(args.archive) as tree:
        tree.extractall(source, filter="data")
    started = time.monotonic()
    record = dict(head=args.head, archive_sha256=sha(args.archive), complete=False,
                  tools=TOOLS, commands=[], variants=[])
    (source / ".tools").symlink_to(tools, target_is_directory=True)
    (source / ".lake").mkdir()
    (source / ".lake/packages").symlink_to(args.packages.resolve(), target_is_directory=True)
    env = dict(os.environ, CARGO_NET_OFFLINE="true", CARGO_INCREMENTAL="0",
               CARGO_TARGET_DIR=str(out / "formal-target"))

    def run(label, argv, root=source, current_env=env):
        start = time.monotonic()
        with (out / (label + ".log")).open("xb") as log:
            process = subprocess.run([str(arg) for arg in argv], cwd=root, env=current_env,
                                     stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT)
        record["commands"].append(dict(label=label, argv=[str(arg) for arg in argv],
            cwd=str(root), exit_code=process.returncode, seconds=time.monotonic()-start,
            log_sha256=sha(out / (label + ".log"))))
        if process.returncode:
            raise ValueError("Command failure is not a property rejection: " + label)

    def outputs(path):
        return {str(p.relative_to(path)): sha(p) for p in path.rglob("*") if p.is_file()}

    try:
        with args.lock.open("a") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            for mode in ["values", "handler"]:
                destination = out / mode
                run("generate-" + mode, ["bash", "verification/experiments/capture-handoff/regenerate.sh", mode],
                    current_env=env | {"SANDPROOF_PROOF_TOOL_ROOT": str(tools),
                                       "SANDPROOF_CAPTURE_OUTPUT": str(destination)})
                if json.loads((destination / "capture_handoff.llbc").read_text())["has_errors"]:
                    raise ValueError("Charon reported extraction errors")
                record[mode + "_outputs"] = outputs(destination)
            for name in ["CaptureHandoff/Types.lean", "CaptureHandoff/Funs.lean", "translation.json"]:
                if ((source / "verification/aeneas/capture-handoff" / name).read_bytes() !=
                    (out / "values/lean" / name).read_bytes()):
                    raise ValueError("Committed correct generated file changed: " + name)
            template = out / "handler/lean/CaptureHandoff/FunsExternal_Template.lean"
            record["handler_external_template_sha256"] = sha(template)
            if "core.option.Option.map" in template.read_text():
                raise ValueError("Unexpected new Option.map binding")
            for role, case, namespace, module in [
                ("drop", CASES[1], "CaptureHandoffDropRust", "CaptureHandoffDrop"),
                ("revive", CASES[2], "CaptureHandoffReviveRust", "CaptureHandoffRevive")]:
                wrong = out / ("wrong-source-" + role)
                shutil.copytree(source, wrong, ignore=shutil.ignore_patterns(".lake", ".tools"))
                digest = mutate(wrong, case)
                destination = out / ("wrong-values-" + role)
                destination.mkdir()
                current_env = env | {"RUSTFLAGS": "--remap-path-prefix=" + str(wrong) + "=.",
                                     "CARGO_TARGET_DIR": str(out / ("target-" + role))}
                command = [tools / "aeneas/charon/bin/charon", "cargo", "--preset=aeneas",
                           "--error-on-warnings", "--sysroot", "default"]
                for name in ROOTS:
                    command += ["--start-from", "crate::" + name]
                command += ["--include", "sandbox_controller_core", "--dest-file",
                            destination / "capture_handoff.llbc", "--", "--manifest-path",
                            "Cargo.toml", "--lib", "--locked", "--offline"]
                run("charon-" + role, command, wrong / "verification/extract-capture-handoff", current_env)
                if json.loads((destination / "capture_handoff.llbc").read_text())["has_errors"]:
                    raise ValueError("Wrong-source extraction failed")
                run("aeneas-" + role, [tools / "aeneas/bin/aeneas", "-backend", "lean",
                    "-abort-on-error", "-warnings-as-errors", "-split-files", "-emit-json",
                    "-namespace", namespace, "-subdir", module, "-dest", destination / "lean",
                    destination / "capture_handoff.llbc"], wrong, current_env)
                shutil.copytree(destination / "lean" / module,
                    source / "verification/aeneas/capture-handoff" / ("wrong-" + role) / module)
                record["variants"].append(dict(role=role, source_sha256=digest, outputs=outputs(destination)))
            config = source / "lakefile.toml"
            text = config.read_text()
            registrations = tomllib.loads(text).get("lean_lib", [])
            for module, path in LIBRARIES:
                existing = [entry for entry in registrations if entry.get("name") == module]
                if existing and existing != [dict(name=module, srcDir=path)]:
                    raise ValueError("Ambiguous private library registration")
                if not existing:
                    text += '\n[[lean_lib]]\nname = "' + module + '"\nsrcDir = "' + path + '"\n'
            config.write_text(text)  # Private extracted source only.
            record["private_lakefile_sha256"] = sha(config)
            run("build", ["lake", "build", "SandboxModel.Lifecycle.CaptureHandoff.Values"] +
                [module + ".Funs" for module, _ in LIBRARIES])
            for name in ["DropMaterial", "ReviveEligibility"]:
                run("audit-" + name, ["lake", "env", "lean",
                    "verification/experiments/capture-handoff/model/" + name + ".lean"])
            files = dict(PROOFS)
            for module, path in LIBRARIES:
                for name in ["Types", "Funs"]:
                    files[path + "/" + module + "/" + name + ".lean"] = []
            requests = [dict(tool="lean_diagnostic_messages", arguments=dict(file_path=path, timeout_s=120),
                             expected_sha256=sha(source / path)) for path in files]
            data = "".join(json.dumps(row) + "\n" for row in requests + [dict(tool="exit")])
            (out / "requests.jsonl").write_text(data)
            with (out / "service.log").open("xb") as log:
                process = subprocess.run([str(source / ".tools/lean-mcp/bin/python"), "tools/lean_session.py"],
                    cwd=source, env=env, input=data.encode(), stdout=log, stderr=subprocess.STDOUT)
            record["client_exit_code"] = process.returncode
            if process.returncode:
                raise ValueError("Original Lean service client failed")
            actual = []
            for line in (out / "service.log").read_text().splitlines():
                try:
                    row = json.loads(line)
                except ValueError:
                    continue
                if row.get("request", {}).get("tool") == "lean_diagnostic_messages":
                    actual.append(row)
            record["actual_results"] = actual
            if len(actual) != len(files):
                raise ValueError("Missing actual-file diagnostics")
            seen = set()
            for row in actual:
                path = row["request"]["arguments"]["file_path"]
                if path not in files or path in seen:
                    raise ValueError("Unexpected or duplicate file diagnostic")
                seen.add(path)
                if (row["request"]["expected_sha256"] != sha(source / path) or
                    row["revision"]["file_sha256"] != sha(source / path)):
                    raise ValueError("Checked file identity changed: " + path)
                value = row["result"].get("result", row["result"])
                if (row["stale"] or value.get("success") is not True or
                    value.get("partial") or value.get("timed_out") or value.get("failed_dependencies")):
                    raise ValueError("Incomplete actual-file check: " + path)
                messages = value.get("items", [])
                if any(item.get("severity") in ["warning", "error"] for item in messages):
                    raise ValueError("Actual file has diagnostics: " + path)
                audited = [item["message"] for item in messages if item["message"].startswith("Checked ")]
                if audited != ["Checked " + name + ": 3 standard logical dependencies" for name in files[path]]:
                    raise ValueError("Missing, changed or extra proof audit: " + path)
        record["complete"] = True
    except BaseException as error:
        record.update(error=repr(error), traceback=traceback.format_exc())
    record["seconds"] = time.monotonic() - started
    (out / "result.json").write_text(json.dumps(record, indent=2) + "\n")
    if not record["complete"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
