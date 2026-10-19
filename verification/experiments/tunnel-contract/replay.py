"""Reproduce the frozen relay candidate, real Rust fault and kernel counterexample."""
from pathlib import Path
import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import time

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[2]
sys.path.insert(0, str(ROOT))
from tools.check_tunnel_extraction import check_libraries, check_pair, read_json, sha


def run(out, name, command, cwd, environment, expected=0):
    command = [str(x) for x in command]
    start = time.monotonic()
    with (out / (name + ".log")).open("xb") as log:
        process = subprocess.Popen(command, cwd=cwd, env=environment,
                                   stdout=log, stderr=subprocess.STDOUT)
        (out / (name + ".pid")).write_text(str(process.pid) + "\n")
        code = process.wait()
    row = dict(stage=name, command=command, exit_code=code, expected_exit=expected,
               seconds=round(time.monotonic() - start, 3))
    (out / (name + ".json")).write_text(json.dumps(row, indent=2) + "\n")
    if code != expected:
        raise ValueError(name + " unexpected exit: " + str(code))
    return row


def materialize(source_base, destination, profile):
    if destination.exists() or destination.is_symlink():
        raise ValueError("source destination already exists")
    for name, expected in profile["source_files"].items():
        data = subprocess.check_output(["git", "show", profile["base"] + ":" + name], cwd=source_base)
        if hashlib.sha256(data).hexdigest() != expected:
            raise ValueError("base source mismatch: " + name)
    subprocess.run(["git", "clone", "--quiet", "--no-local", "--no-checkout",
                    str(source_base), str(destination)], check=True)
    subprocess.run(["git", "-c", "advice.detachedHead=false", "checkout", "--detach", profile["base"]],
                   cwd=destination, check=True)
    for name, expected in profile["source_files"].items():
        if sha(destination / name) != expected:
            raise ValueError("checkout mismatch: " + name)


def snapshot(source):
    names = subprocess.check_output(["git", "ls-files"], cwd=source, text=True).splitlines()
    return {name: sha(source / name) for name in names}


def unchanged(source, original, exceptions=None):
    exceptions = exceptions or {}
    for name, expected in original.items():
        if sha(source / name) != exceptions.get(name, expected):
            raise ValueError("unregistered source change: " + name)
    for name, expected in exceptions.items():
        if sha(source / name) != expected:
            raise ValueError("registered candidate changed: " + name)


def apply_candidate(source, profile):
    patch = HERE / "candidate.patch"
    if sha(patch) != profile["patch_sha256"]:
        raise ValueError("patch mismatch")
    subprocess.run(["git", "apply", "--check", str(patch)], cwd=source, check=True)
    subprocess.run(["git", "apply", str(patch)], cwd=source, check=True)
    changed = subprocess.check_output(["git", "diff", "--name-only"], cwd=source, text=True).splitlines()
    added = subprocess.check_output(["git", "ls-files", "--others", "--exclude-standard"],
                                    cwd=source, text=True).splitlines()
    if sorted(changed + added) != sorted(profile["candidate_files"]):
        raise ValueError("patch modified unregistered files")
    for name, expected in profile["candidate_files"].items():
        if sha(source / name) != expected:
            raise ValueError("candidate mismatch: " + name)


def llbc(source, name, out):
    matches = list(source.rglob(name))
    if len(matches) != 1:
        raise ValueError("expected exactly one fresh " + name)
    data = read_json(matches[0])
    shutil.copy2(matches[0], out / name)
    return matches[0], data


def native_tests(out, prefix, source, environment):
    stages = []
    for role, package, pattern, count in (
        ("relay", "sandproof-data-transport", "tunnel::tests", 5),
        ("pipe", "sandproof-host-service", "data_gateway::relay_stdio::tests", 2),
    ):
        name = prefix + "-" + role
        stages.append(run(out, name, ["cargo", "test", "-p", package, "--locked", "--offline",
                                      "--lib", pattern, "--", "--test-threads=1"], source, environment))
        text = (out / (name + ".log")).read_text()
        if (f"running {count} tests" not in text
                or f"{count} passed; 0 failed; 0 ignored" not in text):
            raise ValueError(name + " did not execute every registered test")
    return stages


def lean_diagnostics(path):
    return [json.loads(line) for line in path.read_text().splitlines() if line.startswith("{")]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-base", type=Path, required=True)
    parser.add_argument("--destination", type=Path, required=True)
    parser.add_argument("--tool-root", type=Path, required=True)
    args = parser.parse_args()
    check_libraries(ROOT, args.tool_root)
    profile = read_json(HERE / "candidate.json")
    out = args.destination.absolute()
    if out.exists() or out.is_symlink():
        raise ValueError("evidence destination already exists")
    out.mkdir()
    correct = out / "correct"
    mutant = out / "mutant"
    native = dict(os.environ, CARGO_TARGET_DIR=str(out / "target-native"))
    frontend = dict(os.environ, CARGO_TARGET_DIR=str(out / "target-charon"))
    charon = args.tool_root / "aeneas/charon/bin/charon"
    aeneas = args.tool_root / "aeneas/bin/aeneas"
    common = [charon, "cargo", "--preset=aeneas", "--error-on-warnings", "--sysroot", "default",
              "--opaque", "tokio", "--opaque", "hyper", "--opaque", "axum",
              "--include", "sandproof_data_transport"]
    cargo = ["--", "--manifest-path", "sandbox-data-transport/Cargo.toml", "--lib", "--locked", "--offline"]
    roots = [arg for root in profile["roots"] for arg in ("--start-from", root)]
    stages = []
    materialize(args.source_base, correct, profile)
    before = snapshot(correct)
    stages.extend(native_tests(out, "original", correct, native))
    original_name = "sandproof_tunnel_original.llbc"
    stages.append(run(out, "original-unsupported", common + ["--start-from", "crate::tunnel::relay",
                       "--dest-file", original_name] + cargo, correct, frontend, 101))
    path, partial = llbc(correct, original_name, out)
    if partial.get("has_errors") is not True or "Coroutines are not supported" not in (
            out / "original-unsupported.log").read_text():
        raise ValueError("original unsupported evidence changed")
    # Preserve the actual failed frontend output outside the candidate tree.
    path.unlink()
    unchanged(correct, before)
    apply_candidate(correct, profile)
    stages.append(run(out, "format", ["cargo", "fmt", "--all", "--", "--check"], correct, native))
    stages.append(run(out, "clippy", ["cargo", "clippy", "-p", "sandproof-data-transport", "--lib",
                      "--tests", "--locked", "--offline", "--", "-D", "warnings"], correct, native))
    stages.extend(native_tests(out, "candidate", correct, native))
    policy_name = "sandproof_tunnel_policy.llbc"
    stages.append(run(out, "charon-correct", common + roots + ["--dest-file", policy_name] + cargo,
                      correct, frontend))
    path, data = llbc(correct, policy_name, out)
    check_pair(ROOT, data)
    stages.append(run(out, "aeneas-correct", [aeneas, "-backend", "lean", "-abort-on-error",
                      "-warnings-as-errors", "-split-files", "-emit-json", "-namespace", "SandproofTunnelRust",
                      "-subdir", "SandproofTunnel", "-dest", out / "generated", path], correct, frontend))
    for name in ("SandproofTunnel/Types.lean", "SandproofTunnel/Funs.lean", "translation.json"):
        if (out / "generated" / name).read_bytes() != (ROOT / "verification/aeneas/tunnel" / name).read_bytes():
            raise ValueError("fresh generation differs: " + name)
    unchanged(correct, before, profile["candidate_files"])
    materialize(args.source_base, mutant, profile)
    mutant_before = snapshot(mutant)
    apply_candidate(mutant, profile)
    control = mutant / "sandbox-data-transport/src/tunnel/control.rs"
    original = control.read_text()
    old = "    download?;\n    upload\n"
    fault = "    download?;\n    match upload {\n        Ok(()) => Ok(()),\n        Err(_error) => Ok(()),\n    }\n"
    if original.count(old) != 1:
        raise ValueError("fault insertion point changed")
    control.write_text(original.replace(old, fault))
    wrong_files = dict(profile["candidate_files"])
    wrong_files[str(control.relative_to(mutant))] = sha(control)
    source_tests = subprocess.check_output(["git", "show", profile["base"] +
        ":sandbox-data-transport/src/tunnel.rs"], cwd=mutant, text=True).split("#[cfg(test)]", 1)[1]
    if (mutant / "sandbox-data-transport/src/tunnel.rs").read_text().split("#[cfg(test)]", 1)[1] != source_tests:
        raise ValueError("original test body changed")
    stages.append(run(out, "mutant-compile", ["cargo", "test", "-p", "sandproof-data-transport",
                      "--locked", "--offline", "--lib", "--no-run"], mutant, native))
    stages.append(run(out, "mutant-native-rejected", ["cargo", "test", "-p", "sandproof-data-transport",
        "--locked", "--offline", "--lib",
        "tunnel::tests::upload_write_failure_preserves_the_guests_rejection_response",
        "--", "--exact", "--test-threads=1", "--nocapture"], mutant, native, 101))
    text = (out / "mutant-native-rejected.log").read_text()
    if ("running 1 test" not in text or "0 passed; 1 failed; 0 ignored" not in text
            or "called `Result::unwrap_err()` on an `Ok` value" not in text):
        raise ValueError("fault did not reach the original error assertion")
    stages.append(run(out, "charon-mutant", common + roots +
                      ["--dest-file", "sandproof_tunnel_mutant.llbc"] + cargo, mutant, frontend))
    wrong_path, wrong_data = llbc(mutant, "sandproof_tunnel_mutant.llbc", out)
    if wrong_data.get("has_errors") is not False:
        raise ValueError("fault did not produce a complete frontend output")
    stages.append(run(out, "aeneas-mutant", [aeneas, "-backend", "lean", "-abort-on-error",
        "-warnings-as-errors", "-split-files", "-emit-json", "-namespace", "SandproofTunnelRust",
        "-subdir", "SandproofTunnel", "-dest", out / "wrong-generated", wrong_path], mutant, frontend))
    library = ROOT / "verification/aeneas/tunnel/SandproofTunnel"
    if (out / "wrong-generated/SandproofTunnel/Types.lean").read_bytes() != (library / "Types.lean").read_bytes():
        raise ValueError("fault changed generated types")
    if (out / "wrong-generated/SandproofTunnel/Funs.lean").read_bytes() == (library / "Funs.lean").read_bytes():
        raise ValueError("fault did not change the generated function")
    project = out / "wrong-lean"
    project.mkdir()
    view = (ROOT / "lake-tunnel.toml").read_text()
    literal = 'srcDir = "verification/aeneas/tunnel"'
    if view.count(literal) != 1:
        raise ValueError("private library configuration changed")
    (project / "lakefile.toml").write_text(view.replace(literal,
        "srcDir = " + json.dumps(str(out / "wrong-generated"))))
    for name in ("lean-toolchain", "lake-manifest.json"):
        shutil.copy2(ROOT / name, project / name)
    for name in ("SandboxModel", "verification"):
        (project / name).symlink_to(ROOT / name, target_is_directory=True)
    (project / ".lake").mkdir()
    (project / ".lake/packages").symlink_to(ROOT / ".lake/packages", target_is_directory=True)
    stages.append(run(out, "wrong-generated-build", ["lake", "--no-cache", "build", "SandproofTunnel.Funs",
        "SandboxModel.ProofAuditCommand", "SandboxModel.Foundations.StreamHistory"], project, native))
    same = out / "SameContract.lean"
    shutil.copy2(library / "Contract.lean", same)
    stages.append(run(out, "same-properties-rejected", ["lake", "env", "lean", "--json", same],
                      project, native, 1))
    errors = [x for x in lean_diagnostics(out / "same-properties-rejected.log") if x["severity"] == "error"]
    starts = [(i + 1, line.split()[1]) for i, line in enumerate(same.read_text().splitlines())
              if line.startswith("theorem ")]
    owners = {next((name for line, name in reversed(starts) if line <= error["pos"]["line"]), None)
              for error in errors}
    if owners != {"finish_download_ok"}:
        raise ValueError("not the unchanged property's expected failure: " + repr(owners))
    counter = out / "Counterexample.lean"
    shutil.copy2(HERE / "Counterexample.lean", counter)
    stages.append(run(out, "counterexample", ["lake", "env", "lean", "--json", counter], project, native))
    items = lean_diagnostics(out / "counterexample.log")
    if (any(x["severity"] in ("error", "warning") for x in items)
            or sum(str(x.get("data", "")).startswith("Checked SandproofTunnelRust.Counterexample.")
                   for x in items) != 3):
        raise ValueError("independent counterexample/audit incomplete")
    unchanged(mutant, mutant_before, wrong_files)
    record = dict(exit_code=0, stages=stages, candidate_files=profile["candidate_files"],
                  wrong_files=wrong_files, original_and_candidate_tests=dict(relay=5, pipe=2),
                  original_native_test_body_unchanged=True, same_properties_sha256=sha(same),
                  same_property_failures=sorted(owners), counterexample_audits=3,
                  source_unchanged=True, types_identical=True)
    (out / "result.json").write_text(json.dumps(record, indent=2) + "\n")
    print("Relay original/candidate replay and real Rust same-property rejection passed.")


if __name__ == "__main__":
    main()
