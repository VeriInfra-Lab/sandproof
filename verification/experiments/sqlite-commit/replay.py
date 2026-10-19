"""Replay the original tools on the actual frozen SQLite candidate."""
from pathlib import Path
import argparse
import hashlib
import json
import os
import shutil
import subprocess
import sys
import time

from prepare import HERE, prepare


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(evidence, name, command, cwd, environment):
    started = time.monotonic()
    with (evidence / f"{name}.log").open("xb") as log:
        code = subprocess.run(command, cwd=cwd, env=environment,
                              stdout=log, stderr=subprocess.STDOUT).returncode
    record = {"stage": name, "command": command, "exit_code": code,
              "seconds": round(time.monotonic() - started, 3)}
    (evidence / f"{name}.json").write_text(json.dumps(record, indent=2) + "\n")
    if code:
        raise RuntimeError(f"{name} failed with {code}; see {evidence / f'{name}.log'}")
    return record


def generated_llbc(source, name):
    matches = list(source.rglob(name))
    if len(matches) != 1:
        raise ValueError(f"expected one actual new {name}, found {matches}")
    path = matches[0]
    data = json.loads(path.read_text())
    if data.get("has_errors") is not False:
        raise ValueError("Charon reported an incomplete or erroneous output")
    if data["translated"]["options"]["dest_file"] != name:
        raise ValueError("Charon output does not preserve the fixed relative destination")
    return path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-base", type=Path, required=True)
    parser.add_argument("--destination", type=Path, required=True)
    parser.add_argument("--tool-root", type=Path, required=True)
    parser.add_argument("--charon-target", type=Path)
    args = parser.parse_args()
    repository = HERE.parents[2]
    sys.path.insert(0, str(repository))
    from tools.check_sqlite_extraction import check_libraries, check_pair, read_json
    check_libraries(repository, args.tool_root)
    destination = args.destination.absolute()
    if destination.exists() or destination.is_symlink():
        raise ValueError("replay destination already exists")
    destination.mkdir(parents=True)
    source = destination / "source"
    profile = prepare(args.source_base, source)
    evidence = destination / "evidence"
    evidence.mkdir()
    config = json.loads((HERE / "candidate.json").read_text())
    tracked = subprocess.check_output(["git", "ls-files"], cwd=source, text=True).splitlines()
    inputs = sorted(set(tracked) | set(config["candidate_files"]))
    before = {name: digest(source / name) for name in inputs}
    environment = dict(os.environ)
    environment["CARGO_TARGET_DIR"] = str(
        (args.charon_target or destination / "target-charon").absolute()
    )
    charon = args.tool_root.resolve() / "aeneas/charon/bin/charon"
    aeneas = args.tool_root.resolve() / "aeneas/bin/aeneas"
    common = [str(charon), "cargo", "--preset=aeneas", "--error-on-warnings",
              "--sysroot", "default", "--opaque", "rusqlite",
              "--include", "sandproof_controller_service",
              "--include", "sandbox_controller_core", "--include", "sandproof_protocol"]
    cargo = ["--", "--manifest-path", "controller-service/Cargo.toml",
             "--lib", "--locked", "--offline"]
    core_name = "sandproof_sqlite_core.llbc"
    command = list(common)
    for name in config["roots"]:
        command += ["--start-from", name]
    command += ["--dest-file", core_name] + cargo
    stages = [run(evidence, "charon-core", command, source, environment)]
    core = generated_llbc(source, core_name)
    shutil.copy2(core, evidence / core_name)
    generated = evidence / "generated"
    command = [str(aeneas), "-backend", "lean", "-abort-on-error", "-warnings-as-errors",
               "-split-files", "-emit-json", "-namespace", "SandproofSqliteRust",
               "-subdir", "SandproofSqlite", "-dest", str(generated), str(core)]
    stages.append(run(evidence, "aeneas", command, source, environment))
    expected_root = HERE.parents[1] / "aeneas/sqlite"
    for name in ["SandproofSqlite/Types.lean", "SandproofSqlite/Funs.lean", "translation.json"]:
        expected = expected_root / name
        if not expected.is_file() or (generated / name).read_bytes() != expected.read_bytes():
            raise ValueError(f"original-tool regeneration differs: {name}")
    for name, expected in config["candidate_files"].items():
        if digest(source / name) != expected:
            raise ValueError(f"candidate source changed during extraction: {name}")
    transaction = source / "controller-service/src/controller/transaction.rs"
    original = transaction.read_bytes()
    if digest(HERE / "wrapper-roots.rs") != config["wrapper_probes_sha256"]:
        raise ValueError("wrapper root source differs from the frozen profile")
    probes = (HERE / "wrapper-roots.rs").read_bytes()
    transaction.write_bytes(original + probes)
    wrapper_name = "sandproof_sqlite_wrapper.llbc"
    command = common + ["--include", "rusqlite::types::value_ref::ValueRef",
                        "--start-from", "crate::controller::transaction::sqlite_wrapper_probe",
                        "--dest-file", wrapper_name] + cargo
    stages.append(run(evidence, "charon-wrapper", command, source, environment))
    wrapper = generated_llbc(source, wrapper_name)
    shutil.copy2(wrapper, evidence / wrapper_name)
    if transaction.read_bytes() != original + probes:
        raise ValueError("wrapper instrumentation changed more than the frozen append-only roots")
    after = {name: digest(source / name) for name in inputs}
    # Only the frozen append-only roots differ; never erase AST fields or
    # mutate source to make its digests agree.
    after["controller-service/src/controller/transaction.rs"] = hashlib.sha256(original).hexdigest()
    if after != before:
        raise ValueError("candidate inputs changed during extraction")
    check_pair(repository, read_json(evidence / core_name), read_json(evidence / wrapper_name))
    result = {"source_unchanged": True, "candidate_input_count": len(inputs), "base": profile["base"], "patch_sha256": config["patch_sha256"],
              "wrapper_probes_sha256": digest(HERE / "wrapper-roots.rs"), "stages": stages,
              "generated_files_byte_identical": 3,
              "core_llbc_sha256": digest(evidence / core_name),
              "wrapper_llbc_sha256": digest(evidence / wrapper_name),
              "scope": "actual core and wrapper/Drop; full commit validator graph is not admitted"}
    (evidence / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
