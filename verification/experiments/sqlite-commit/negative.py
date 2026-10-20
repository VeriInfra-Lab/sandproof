"""Compile/extract one actual Rust fault; reject it with unchanged properties."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time

from prepare import HERE, prepare


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def diagnostics(path):
    result = []
    for line in path.read_text().splitlines():
        if line.startswith("{"):
            item = json.loads(line)
            if "severity" in item:
                result.append(item)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-base", type=Path, required=True)
    parser.add_argument("--destination", type=Path, required=True)
    parser.add_argument("--correct-replay", type=Path, required=True)
    parser.add_argument("--tool-root", type=Path, required=True)
    parser.add_argument("--charon-target", type=Path)
    parser.add_argument("--native-only", action="store_true")
    args = parser.parse_args()
    root = HERE.parents[2]
    sys.path.insert(0, str(root))
    from tools.check_sqlite_extraction import check_pair, read_json
    if not args.native_only:
        baseline = read_json(args.correct_replay / "evidence/result.json")
        if (not baseline.get("source_unchanged")
                or baseline.get("generated_files_byte_identical") != 3
                or any(x["exit_code"] != 0 for x in baseline["stages"])):
            raise ValueError("successful exact correct replay required")
        check_pair(root, read_json(args.correct_replay / "evidence/sandproof_sqlite_core.llbc"),
                   read_json(args.correct_replay / "evidence/sandproof_sqlite_wrapper.llbc"))
    destination = args.destination.absolute()
    if destination.exists() or destination.is_symlink():
        raise ValueError("negative destination exists")
    destination.mkdir(parents=True)
    source, evidence = destination / "source", destination / "evidence"
    evidence.mkdir()
    prepare(args.source_base, source)
    profile = read_json(HERE / "candidate.json")
    memo = source / "controller-service/src/controller/transaction/memo.rs"
    original = memo.read_text()
    line = "    cells.set_committed(None);\n"
    if original.count(line) != 2:
        raise ValueError("unique commit clear/rollback clear pair changed")
    memo.write_text(original.replace(line, "    // Fault: preserve the old committed memo.\n", 1))
    before = {name: sha(source / name) for name in profile["candidate_files"]}
    record = dict(actual_rust_mutant=True, mutation="only skip first committed=None", stages=[])

    def run(stage, command, cwd=source, expected=0, environment=None):
        command = [str(x) for x in command]
        if command[0] == 'cargo':
            from tools.acceptance_receipts import native_run
            output = evidence / (stage + '.log')
            native_run('sqlite-commit-' + stage, command, cwd, output, environment, expected)
            return output
        started = time.monotonic()
        print("START", stage, flush=True)
        output = evidence / (stage + ".log")
        with output.open("xb") as log:
            process = subprocess.Popen(command, cwd=cwd, env=environment,
                                       stdout=log, stderr=subprocess.STDOUT)
            (evidence / (stage + ".pid")).write_text(str(process.pid) + "\n")
            code = process.wait()
        record["stages"].append(dict(stage=stage, command=command, exit_code=code,
                                     expected_exit=expected, seconds=round(time.monotonic()-started, 3)))
        (evidence / "result.json").write_text(json.dumps(record, indent=2) + "\n")
        print("END", stage, code, flush=True)
        if code != expected:
            raise ValueError(stage + " unexpected exit: " + str(code))
        return output

    native = run("native-rejected", [
        "cargo", "test", "-p", "sandproof-controller-service", "--locked", "--offline", "--lib",
        "controller::cache_tests::uncertain_commit_reconciles_both_durable_outcomes_for_every_write_family",
        "--", "--exact", "--test-threads=1", "--nocapture"], expected=101)
    text = native.read_text()
    if ("running 1 test" not in text or "0 passed; 1 failed; 0 ignored" not in text
            or "controller/cache_tests.rs:102:" not in text
            or "assertion `left == right` failed" not in text):
        raise ValueError("bad Rust must compile and hit the original cache assertion")
    if sha(source / "controller-service/src/controller/cache_tests.rs") != profile["source_files"]["controller-service/src/controller/cache_tests.rs"]:
        raise ValueError("original native assertion changed")
    if args.native_only:
        return
    recipes = [x["command"] for x in baseline["stages"] if x["stage"] == "charon-core"]
    if len(recipes) != 1:
        raise ValueError("actual Charon recipe missing")
    environment = dict(os.environ, CARGO_TARGET_DIR=str(
        (args.charon_target or destination / "target-charon").absolute()))
    run("charon", recipes[0], environment=environment)
    llbc = source / "sandproof_sqlite_core.llbc"
    data = read_json(llbc)
    if data.get("has_errors") is not False:
        raise ValueError("bad Rust did not produce a complete LLBC")
    generated = evidence / "generated"
    run("aeneas", [args.tool_root / "aeneas/bin/aeneas", "-backend", "lean", "-abort-on-error",
        "-warnings-as-errors", "-split-files", "-emit-json", "-namespace", "SandproofSqliteRust",
        "-subdir", "SandproofSqlite", "-dest", generated, llbc])
    formal = root / "verification/aeneas/sqlite/SandproofSqlite"
    library = generated / "SandproofSqlite"
    if (library / "Types.lean").read_bytes() != (formal / "Types.lean").read_bytes():
        raise ValueError("fault changed generated types")
    if (library / "Funs.lean").read_bytes() == (formal / "Funs.lean").read_bytes():
        raise ValueError("fault did not change generated implementation")
    for name in ("TypesExternal.lean", "FunsExternal.lean", "Model.lean"):
        shutil.copy2(formal / name, library / name)
    proofs = formal / "Results.lean"
    same = evidence / "SameProperties.lean"
    shutil.copy2(proofs, same)
    counter = evidence / "Counterexample.lean"
    shutil.copy2(HERE / "Counterexample.lean", counter)
    project = destination / "lean"
    project.mkdir()
    view = (root / "lake-sqlite.toml").read_text()
    literal = 'srcDir = "verification/aeneas/sqlite"'
    if view.count(literal) != 1:
        raise ValueError("private library view changed")
    (project / "lakefile.toml").write_text(view.replace(literal, "srcDir = " + json.dumps(str(generated))))
    for name in ("lean-toolchain", "lake-manifest.json"):
        shutil.copy2(root / name, project / name)
    (project / "SandboxModel").symlink_to(root / "SandboxModel", target_is_directory=True)
    (project / ".lake").mkdir()
    (project / ".lake/packages").symlink_to(root / ".lake/packages", target_is_directory=True)
    run("generated-build", ["lake", "--no-cache", "build", "SandproofSqlite.Funs", "SandproofSqlite.Model", "SandboxModel.ProofAuditCommand"], cwd=project)
    rejected = run("same-properties-rejected", ["lake", "env", "lean", "--json", same], cwd=project, expected=1)
    errors = [x for x in diagnostics(rejected) if x["severity"] == "error"]
    starts = [(i+1, re.match(r"theorem (\w+)", line).group(1))
              for i, line in enumerate(proofs.read_text().splitlines()) if line.startswith("theorem ")]
    owners = {next((name for line, name in reversed(starts) if line <= e["pos"]["line"]), None) for e in errors}
    expected = {"commit_unknown_exact", "commit_unknown_abstract", "commit_success_abstract", "normal_commit_prefix"}
    if owners != expected:
        raise ValueError("not the expected unchanged-property failures: " + repr(owners))
    positive = run("counterexample", ["lake", "env", "lean", "--json", counter], cwd=project)
    items = diagnostics(positive)
    if (any(x["severity"] in {"error", "warning"} for x in items)
            or sum(str(x.get("data", "")).startswith("Checked SandproofSqliteRust.Counterexample.") for x in items) != 3):
        raise ValueError("counterexample dependency audit failed")
    if same.read_bytes() != proofs.read_bytes():
        raise ValueError("same properties changed")
    if before != {name: sha(source / name) for name in profile["candidate_files"]}:
        raise ValueError("bad Rust source changed during checks")
    record.update(exit_code=0, source_unchanged=True, generated_types_identical=True,
                  same_property_failures=sorted(owners), same_properties_sha256=sha(same),
                  kernel_counterexample_count=3, mutant_files=before)
    (evidence / "result.json").write_text(json.dumps(record, indent=2) + "\n")
    print("Actual bad Rust compiled/extracted; original SQLite assertion and unchanged Lean properties rejected it.", flush=True)


if __name__ == "__main__":
    main()
