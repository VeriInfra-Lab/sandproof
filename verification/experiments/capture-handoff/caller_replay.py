"""Check the current real Pause/late-Commit slice on the formal host.

The older four-field value replay keeps its original historical scope. This
entry checks the local-core roots, current six-field proofs and one actual
wrong-target mutant. The caller supplies a private component lock, not the
formal-heavy lock already held by the outer executor.
"""
import argparse
import fcntl
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import time
import tomllib
import traceback

from native import CASES, mutate, sha
from replay import TOOLS

PREFIX = "SandboxModel.Lifecycle.CaptureHandoffCaller."
BASE = "SandboxModel/Lifecycle/CaptureHandoff/"
GENERATED = "verification/aeneas/capture-handoff/"
MODULES = (
    "CallerStores", "CallerGuards", "CallerCloning", "CallerValues", "CallerCurrent",
    "CallerCompensation", "CallerIndex", "CallerObservation", "CallerLate", "CallerRecord",
    "CallerWitness", "CallerCommitChecks", "CallerRecordChecks", "CallerComposition",
)
LIBRARIES = (
    ("CaptureCaller", GENERATED + "caller"),
    ("CaptureTargetWrong", GENERATED + "wrong-target"),
)
EXTERNALS = {
    "Bool.Insts.CoreCmpPartialEqBool.ne", "core.option.Option.is_some_and",
    "core.option.Option.unwrap_or_default", "core.option.OptionShared0T.cloned",
    "core.option.Option.Insts.CoreCloneClone.clone", "core.option.Option.Insts.CoreCmpPartialEqOption.eq",
    "core.option.Option.Insts.CoreOpsTry_traitTry.branch",
    "core.option.Option.Insts.CoreOpsTry_traitFromResidualOptionInfallible.from_residual",
    "alloc.string.String.Insts.CoreCmpPartialEqString.eq",
    "alloc.string.String.Insts.CoreCloneClone.clone", "alloc.vec.Vec.Insts.CoreDefaultDefault.default",
}
CRITICAL = {
    "failure_preserves_uses", "late_success_safe", "current_capture_exact",
    "begin_pause_compensation_exact", "compensation_ordinary_caller_connects",
    "late_commit_success_checks", "late_commit_success_accounts_without_revival",
    "record_inner_commit_success_identity", "record_commit_success_outer",
    "record_commit_success_accounts_without_revival", "late_commit_bound_inputs_accept",
    "same_target_property_accepts", "caller_index_register_membership",
}


def without_comments(text):
    return re.sub(r"/-.*?-/", "", text, flags=re.S)


def inventory(source):
    declarations = []
    files = {BASE + "Conditions.lean": []}
    for module in MODULES:
        relative = BASE + module + ".lean"
        text = (source / relative).read_text()
        names = re.findall(r"^(?:@\[[^\]]+\]\s*)?theorem\s+(\w+)", without_comments(text), re.M)
        if not names:
            raise ValueError("Missing actual theorem declarations: " + relative)
        declarations.extend(names)
        files[relative] = re.findall(r"^#audit_controller_proof (\S+)$", text, re.M)
    if len(declarations) != len(set(declarations)) or not CRITICAL.issubset(declarations):
        raise ValueError("Duplicate or missing required caller theorem")
    audit = BASE + "CallerAudit.lean"
    expected = [PREFIX + name for name in declarations]
    if re.findall(r"^#audit_controller_proof (\S+)$", (source / audit).read_text(), re.M) != expected:
        raise ValueError("Caller audit does not cover the complete declared closure")
    files[audit] = expected
    for module, root in LIBRARIES:
        for name in ["Types", "FunsExternal", "Funs"]:
            files[root + "/" + module + "/" + name + ".lean"] = []
    wrong = "verification/experiments/capture-handoff/model/IgnoreTarget.lean"
    files[wrong] = [PREFIX + "IgnoreTarget." + name for name in [
        "empty_snapshot_query", "actual_wrong_reaches_later_error", "same_property_rejects"]]
    if re.findall(r"^#audit_controller_proof (\S+)$", (source / wrong).read_text(), re.M) != files[wrong]:
        raise ValueError("Wrong-target audit changed")
    return files


def verify_generation(destination, module, namespace, required, local):
    llbc = json.loads((destination / "capture_handoff.llbc").read_text())
    if llbc["has_errors"]:
        raise ValueError("Charon errors cannot stand for a counterexample")
    translation = json.loads((destination / "lean/translation.json").read_text())
    functions = {item["rust_name"]: item for item in translation["functions"]}
    source = (destination / "lean" / module / "Funs.lean").read_text()
    selected = {}
    for name in required:
        item = functions.get(name)
        if item is None or item["is_opaque"] or item["lean_file"] != module + "/Funs.lean":
            raise ValueError("Required real caller missing from final translation: " + name)
        if local and item.get("is_local") is not True:
            raise ValueError("A local caller was replaced with an external root")
        relative = item["lean_name"].removeprefix(namespace + ".")
        if not re.search(r"\bdef\s+" + re.escape(relative) + r"(?=\s|[({])", source):
            raise ValueError("Required real definition missing from final Lean: " + name)
        selected[name] = item
    if local:
        for name in ["accepted", "registration_dispatched", "register_bundle", "observe_bundle",
                     "ordinary_failure", "terminate", "may_stop_original", "accepts_registration",
                     "commit", "eligibility", "has_capture_work", "has_unload_source"]:
            matches = [item for item in translation["functions"] if item["rust_name"].endswith("::" + name)
                       and item.get("source", {}).get("file", "").endswith("controller-core/src/capture.rs")
                       and not item["is_opaque"]]
            if len(matches) != 1:
                raise ValueError("Real capture method missing or ambiguous: " + name)
    template = destination / "lean" / module / "FunsExternal_Template.lean"
    names = set(re.findall(r"\baxiom\s+([\w.]+)", without_comments(template.read_text())))
    if names != EXTERNALS:
        raise ValueError("Unreviewed external function in actual caller extraction")
    return selected


def check_service(source, files, text, sha):
    actual = []
    for line in text.splitlines():
        try:
            row = json.loads(line)
        except ValueError:
            continue
        if row.get("request", {}).get("tool") == "lean_diagnostic_messages":
            actual.append(row)
    if len(actual) != len(files):
        raise ValueError("Missing actual-file diagnostics")
    seen = set()
    for row in actual:
        name = row["request"]["arguments"]["file_path"]
        if name not in files or name in seen:
            raise ValueError("Unexpected or duplicate actual file")
        seen.add(name)
        value = row["result"].get("result", row["result"])
        if (row["stale"] or value.get("success") is not True or value.get("partial") or
            value.get("timed_out") or value.get("failed_dependencies") or
            row["revision"]["file_sha256"] != sha(source / name) or
            row["request"]["expected_sha256"] != sha(source / name)):
            raise ValueError("Incomplete or changed actual file: " + name)
        items = value.get("items", [])
        if any(item.get("severity") in ["error", "warning"] for item in items):
            raise ValueError("Unresolved actual-file diagnostics: " + name)
        checked = [item for item in items if item["message"].startswith("Checked ")]
        if any(type(item.get(axis)) is not int or item[axis] < 1
               for item in checked for axis in ("line", "column")):
            raise ValueError("Invalid actual proof audit source position: " + name)
        # LSP diagnostics can arrive out of source order. Retain the response
        # and compare every audit, including duplicates, in command order.
        checked = sorted(checked, key=lambda item: (item["line"], item["column"]))
        parsed = [re.fullmatch(r"Checked (\S+): ([0-3]) standard logical dependencies",
                              item["message"]) for item in checked]
        if any(item is None for item in parsed) or [item.group(1) for item in parsed] != files[name]:
            raise ValueError("Missing, changed or extra actual proof audit: " + name)
    return actual


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ["archive", "output", "lock", "tools", "packages"]:
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--archive-sha256", required=True)
    parser.add_argument("--head", required=True)
    args = parser.parse_args()
    if sha(args.archive) != args.archive_sha256:
        raise ValueError("Source archive changed")
    tools = args.tools.resolve()
    for relative, expected in TOOLS.items():
        if sha(tools / relative) != expected:
            raise ValueError("Fixed proof tool changed: " + relative)
    os.umask(0o077)
    out = args.output.resolve()
    out.mkdir(mode=0o700)
    source = out / "source"
    source.mkdir()
    with tarfile.open(args.archive) as tree:
        tree.extractall(source, filter="data")
    (source / ".tools").symlink_to(tools, target_is_directory=True)
    (source / ".lake").mkdir()
    (source / ".lake/packages").symlink_to(args.packages.resolve(), target_is_directory=True)
    started = time.monotonic()
    record = dict(scope="capture-caller", head=args.head, archive_sha256=sha(args.archive), complete=False,
                  tools=TOOLS, commands=[], variants=[], legacy_four_field_migration=False)
    env = dict(os.environ, CARGO_NET_OFFLINE="true", CARGO_INCREMENTAL="0",
               CARGO_TARGET_DIR=str(out / "formal-target"))

    def run(label, argv, root=source, current_env=env, forward=False, data=None):
        argv = [str(arg) for arg in argv]
        log = out / (label + ".log")
        begin = time.monotonic()
        with log.open("xb") as stream:
            standard_input = {"input": data} if data is not None else {"stdin": subprocess.DEVNULL}
            process = subprocess.run(argv, cwd=root, env=current_env, **standard_input,
                                     stdout=stream, stderr=subprocess.STDOUT)
        record["commands"].append(dict(label=label, argv=argv, cwd=str(root), exit_code=process.returncode,
            seconds=time.monotonic() - begin, log_sha256=sha(log)))
        if forward:
            sys.stdout.buffer.write(log.read_bytes())
            sys.stdout.buffer.flush()
        if process.returncode:
            raise ValueError("Command failure is not a property rejection: " + label)

    def compare(destination, module, committed):
        for relative in [module + "/Types.lean", module + "/Funs.lean", "translation.json"]:
            if (destination / "lean" / relative).read_bytes() != (source / committed / relative).read_bytes():
                raise ValueError("Current generated file differs: " + committed + "/" + relative)

    try:
        with args.lock.open("a") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            files = inventory(source)
            record["expected_audits"] = files
            correct = out / "caller"
            run("generate-caller", ["bash", "verification/experiments/capture-handoff/regenerate.sh", "caller"],
                current_env=env | {"SANDPROOF_PROOF_TOOL_ROOT": str(tools), "SANDPROOF_CAPTURE_OUTPUT": str(correct)})
            required = ["sandbox_controller_core::worker::late_capture::" + name for name in ["record", "record_inner", "commit"]]
            required += ["sandbox_controller_core::request::handle_compensation"]
            record["real_callers"] = verify_generation(correct, "CaptureCaller",
                "CaptureCallerRust.sandbox_controller_core", required, True)
            compare(correct, "CaptureCaller", GENERATED + "caller")
            wrong = out / "wrong-source-target"
            shutil.copytree(source, wrong, ignore=shutil.ignore_patterns(".lake", ".tools", "target"))
            mutated = mutate(wrong, CASES[3])
            destination = out / "wrong-target"
            destination.mkdir()
            wrong_env = env | {"RUSTFLAGS": "--remap-path-prefix=" + str(wrong) + "=.",
                               "CARGO_TARGET_DIR": str(out / "target-wrong-target")}
            run("charon-ignore-target", [tools / "aeneas/charon/bin/charon", "cargo", "--preset=aeneas",
                "--error-on-warnings", "--sysroot", "default", "--start-from",
                "sandbox_controller_core::worker::late_capture::record", "--start-from",
                "sandbox_controller_core::worker::late_capture::commit", "--include", "sandbox_controller_core",
                "--include", "core::option::*::and_then", "--dest-file", destination / "capture_handoff.llbc",
                "--", "--manifest-path", "Cargo.toml", "--lib", "--locked", "--offline"],
                wrong / "verification/extract-capture-handoff", wrong_env)
            run("aeneas-ignore-target", [tools / "aeneas/bin/aeneas", "-backend", "lean", "-abort-on-error",
                "-warnings-as-errors", "-split-files", "-emit-json", "-namespace", "CaptureTargetWrongRust",
                "-subdir", "CaptureTargetWrong", "-dest", destination / "lean", destination / "capture_handoff.llbc"],
                wrong, wrong_env)
            selected = verify_generation(destination, "CaptureTargetWrong", "CaptureTargetWrongRust",
                ["sandbox_controller_core::worker::late_capture::commit"], False)
            compare(destination, "CaptureTargetWrong", GENERATED + "wrong-target")
            record["variants"].append(dict(role="ignore-target", source_sha256=mutated, actual_functions=selected))
            config = source / "lakefile.toml"
            text = config.read_text()
            registrations = tomllib.loads(text).get("lean_lib", [])
            for module, path in LIBRARIES:
                existing = [item for item in registrations if item.get("name") == module]
                if existing and existing != [dict(name=module, srcDir=path)]:
                    raise ValueError("Ambiguous caller library registration")
                if not existing:
                    text += '\n[[lean_lib]]\nname = "' + module + '"\nsrcDir = "' + path + '"\n'
            config.write_text(text)  # Private archive view only; public registration has one owner.
            record["private_lakefile_sha256"] = sha(config)
            # Use the existing local-import resolver, including the current
            # generated views. Imported proofs receive their own real-file
            # diagnostics and audits, not only an indirect import check.
            sys.path.insert(0, str(source))
            from tools.proof_batch import closure
            dependencies = closure(source, seeds=list(files), active_view="lakefile.toml")
            for name in dependencies:
                if name not in files:
                    files[name] = re.findall(r"^#audit_controller_proof (\S+)$",
                                             (source / name).read_text(), re.M)
            record["local_import_closure"] = dependencies
            run("build-caller", ["lake", "build", "CaptureCaller.Funs", "CaptureTargetWrong.Funs",
                "SandboxModel.Lifecycle.CaptureHandoff.CallerAudit"], forward=True)
            run("audit-ignore-target", ["lake", "env", "lean",
                "verification/experiments/capture-handoff/model/IgnoreTarget.lean"], forward=True)
            requests = [dict(tool="lean_diagnostic_messages", arguments=dict(file_path=name, timeout_s=120),
                expected_sha256=sha(source / name)) for name in files] + [dict(tool="exit")]
            data = "".join(json.dumps(row) + "\n" for row in requests).encode()
            (out / "requests.jsonl").write_bytes(data)
            run("service", [source / ".tools/lean-mcp/bin/python", "tools/lean_session.py"], data=data)
            actual = check_service(source, files, (out / "service.log").read_text(), sha)
            record["actual_results"] = actual
            record["checked_file_count"] = len(files)
            record["complete"] = True
    except BaseException as error:
        record.update(error=repr(error), traceback=traceback.format_exc())
    record["seconds"] = time.monotonic() - started
    (out / "result.json").write_text(json.dumps(record, indent=2) + "\n")
    print(json.dumps({key: record.get(key) for key in ["scope", "head", "complete", "seconds", "error"]}), flush=True)
    if not record["complete"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
