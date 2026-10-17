#!/usr/bin/env python3
"""Reproduce only this frozen, isolated data-permit experiment; no installation."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import tarfile


PACKAGE = Path(__file__).resolve().parents[1]
CORE = "controller-service/src/application/coordinator/data_access/permit_guards.rs"
CALLER = "controller-service/src/application/coordinator/data_access.rs"
FUNCTIONS = {
    "register_gate_allows", "signal_needs_revocation_read", "retain_after_revocation",
    "create_signal_required", "dispatch_not_revoked", "dispatch_configured",
    "dispatch_target_matches",
}


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def checksums(path):
    result = {}
    for line in path.read_text().splitlines():
        if line.strip() and not line.startswith("#"):
            digest, name = line.split("  ", 1)
            require(len(digest) == 64 and name not in result, f"invalid checksum entry: {name}")
            item = Path(name)
            require(not item.is_absolute() and ".." not in item.parts, "unsafe checksum path")
            result[name] = digest
    require(result, f"empty checksum manifest: {path.name}")
    return result


def verify_files(root, expected):
    for name, digest in expected.items():
        path = root / name
        require(path.is_file() and not path.is_symlink(), f"missing/nonregular input: {name}")
        require(sha(path) == digest, f"input drift: {name}")


def run(command, cwd, log, env, timeout=240, expected=0):
    log.parent.mkdir(parents=True, exist_ok=True)
    with log.open("w") as output:
        try:
            result = subprocess.run(command, cwd=cwd, env=env, stdout=output,
                                    stderr=subprocess.STDOUT, timeout=timeout, check=False)
            code = result.returncode
        except subprocess.TimeoutExpired:
            code = 124
    log.with_suffix(".exit").write_text(f"{code}\n")
    require(code == expected, f"unexpected exit {code} (wanted {expected}): {log.name}")
    return log.read_text()


def tree(root):
    return {str(p.relative_to(root)): sha(p) for p in root.rglob("*") if p.is_file()}


def strings(value):
    if isinstance(value, str):
        return [value]
    if isinstance(value, dict):
        return [s for item in value.values() for s in strings(item)]
    if isinstance(value, list):
        return [s for item in value for s in strings(item)]
    return []


def prepare(archive, output, base_files):
    output.mkdir()
    found = set()
    with tarfile.open(archive, "r:*") as source:
        for entry in source:
            name = Path(entry.name)
            require(not name.is_absolute() and ".." not in name.parts, "unsafe archive path")
            if entry.isdir():
                continue
            require(entry.isfile() and entry.name in base_files, "unexpected archive entry")
            require(entry.name not in found, "duplicate archive entry")
            data = source.extractfile(entry).read()
            require(hashlib.sha256(data).hexdigest() == base_files[entry.name],
                    f"wrong frozen baseline: {entry.name}")
            target = output / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
            target.chmod(entry.mode & 0o777)
            found.add(entry.name)
    require(found == set(base_files), "incomplete frozen baseline archive")


def altered(root, base_files):
    actual = tree(root)
    require(set(base_files) <= set(actual), "overlay deleted baseline files")
    return {name: digest for name, digest in actual.items() if base_files.get(name) != digest}


def cargo_tests(source, label, output, config, env):
    test_env = dict(env, CARGO_TARGET_DIR=str(output / f"target-{label}"))
    text = run(
        ["cargo", "test", "--locked", "--offline", "-p", "sandproof-controller-service",
         "--lib", "data_permit_pilot_tests::dp_", "--", "--nocapture", "--test-threads=1"],
        source, output / "evidence" / f"{label}-tests.log", test_env,
        timeout=300, expected=101 if label == "bad" else 0,
    )
    counts = re.findall(r"test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed;.*?(\d+) filtered out", text)
    require(len(counts) == 1, f"missing/ambiguous test count: {label}")
    passed, failed, _filtered = map(int, counts[0])
    wanted = config["tests"][label]
    require([passed, failed] == wanted["counts"], f"wrong test coverage: {label}")
    if label == "bad":
        for case in wanted["failing_cases"]:
            require(re.search(r"(?m)^\s{4}" + re.escape(case) + r"\s*$", text) is not None,
                    f"wrong negative failure: {case}")
        require("panicked at" in text and "error[E" not in text, "not a compiled assertion failure")
        require("left: 1" in text and "right: 0" in text, "wrong negative business assertion")
    # libtest may print its `test NAME ... ` prefix on the first receipt's line.
    receipts = sorted(line[line.index("DP_RECEIPT "):].strip()
                      for line in text.splitlines() if "DP_RECEIPT " in line)
    if label != "bad":
        require(len(receipts) == config["tests"]["receipt_count"], "missing semantic receipt")
    return receipts


def extract(source, label, namespace, output, config, env, llbc_digest):
    probe = output / "probes" / label
    probe.mkdir(parents=True)
    core = source / CORE
    entry_path = os.path.relpath(core, probe)
    (probe / "entry.rs").write_text(f'#[path = "{entry_path}"]\npub mod permit_guards;\n')
    tool = Path(config["tool_root"])
    extraction = dict(env, RUSTUP_TOOLCHAIN="nightly-2026-08-18",
                      CARGO_TARGET_DIR=str(output / "target-extract"))
    run([str(tool / "charon/bin/charon"), "rustc", "--preset=aeneas",
         "--error-on-warnings", "--sysroot", "default", "--start-from",
         "crate::permit_guards", "--include", "crate", "--dest-file", f"probes/{label}/input.llbc",
         "--", "--edition", "2024", "--crate-type", "lib", "--crate-name", "data_permit_entry",
         "--remap-path-prefix", f"{output}=.", str(probe / "entry.rs")],
        output, probe / "charon.log", extraction, timeout=120)
    actual_llbc = json.loads((probe / "input.llbc").read_text())
    expected_llbc = json.loads((PACKAGE / "llbc" / f"{label}.llbc").read_text())
    require(llbc_digest(actual_llbc) == llbc_digest(expected_llbc), f"LLBC drift: {label}")
    llbc = actual_llbc["translated"]
    local = [f for f in llbc["fun_decls"] if f is not None and f["item_meta"]["is_local"]]
    names = {f["item_meta"]["name"][-1]["Ident"][0] for f in local}
    require(names == FUNCTIONS and len(local) == len(FUNCTIONS), "wrong Rust extraction roots")
    require(all(isinstance(f["body"], dict) and "Error" not in json.dumps(f["body"])
                for f in local), "missing/error local body")
    require(not llbc["options"]["opaque"], "unexpected opaque business rules")
    require(any(os.path.normpath(f["name"].get("Local", "")) == str(core.relative_to(output))
                and f["contents"] == core.read_text() for f in llbc["files"]),
            "LLBC is not bound to the actual candidate helper")
    run([str(tool / "bin/aeneas"), "-backend", "lean", "-abort-on-error",
         "-warnings-as-errors", "-split-files", "-namespace", namespace, "-subdir", namespace,
         "-dest", str(probe / "lean"), str(probe / "input.llbc")],
        output, probe / "aeneas.log", extraction, timeout=120)
    expected = PACKAGE / "lean" / namespace
    actual = probe / "lean" / namespace
    require({p.name for p in actual.glob("*.lean")} == {"Types.lean", "Funs.lean"},
            "unexpected generated module/external interface")
    for name in ("Types.lean", "Funs.lean"):
        require((actual / name).read_bytes() == (expected / name).read_bytes(),
                f"generated Lean drift: {namespace}/{name}")


def proof_checks(output, source_repository, runtime, packages, config, env):
    proof = output / "proof"
    proof.mkdir()
    shutil.copytree(PACKAGE / "lean", proof / "lean")
    shutil.copytree(PACKAGE / "audits", proof / "audits")
    for name in ("lean-toolchain", "lakefile.toml", "lake-manifest.json"):
        shutil.copyfile(PACKAGE / name, proof / name)
    for name in config["service_files"]:
        target = proof / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source_repository / name, target)
        require(sha(target) == config["service_files"][name], "client/launcher copy drift")
    proof_inputs = {f"{folder}/{name}": digest for folder in ("lean", "audits")
                    for name, digest in tree(proof / folder).items()}
    proof_inputs.update({name: sha(proof / name)
                         for name in ("lean-toolchain", "lakefile.toml", "lake-manifest.json")})
    proof_inputs.update(config["service_files"])
    (proof / ".tools").mkdir()
    (proof / ".tools/lean-mcp").symlink_to(runtime, target_is_directory=True)
    (proof / ".lake").mkdir()
    (proof / ".lake/packages").symlink_to(packages, target_is_directory=True)
    (proof / "artifacts").mkdir()
    for dependency in config["lake_dependencies"]:
        directory = packages / dependency["name"]
        revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=directory, text=True).strip()
        require(revision == dependency["rev"], "wrong Lake dependency")
        require(not subprocess.check_output(
            ["git", "status", "--porcelain", "--untracked-files=no"], cwd=directory, text=True).strip(),
            "modified Lake dependency")
    for module in config["proof_order"] + list(config["audit_roots"]):
        # Build previously checked imports before opening this module in the service.
        for prerequisite in config["proof_prerequisites"].get(module, []):
            run(["lake", "build", prerequisite], proof,
                output / "evidence" / f"build-{prerequisite}.log", env)
        code_file = f"audits/{module}.lean" if module in config["audit_roots"] else f"lean/{module}.lean"
        request = {"tool": "lean_run_code", "code_file": code_file,
                   "expected_sha256": sha(proof / code_file)}
        log = output / "evidence" / f"{module}-service.log"
        with log.open("w") as stream:
            result = subprocess.run([str(runtime / "bin/python"), "tools/lean_session.py"],
                input=json.dumps(request) + '\n{"tool":"exit"}\n', text=True,
                cwd=proof, env=env, stdout=stream, stderr=subprocess.STDOUT,
                timeout=300, check=False)
        require(result.returncode == 0, f"service transport failed: {module}")
        records = []
        for line in log.read_text().splitlines():
            try:
                item = json.loads(line)
            except ValueError:
                continue
            if "request" in item:
                records.append(item)
        require(len(records) == 1, f"missing service record: {module}")
        record = records[0]
        require(record.get("stale") is False and record["result"].get("success") is True
                and record["result"].get("timed_out") is False,
                f"stale/failed proof: {module}")
        require(record["revision"].get("file_sha256") == request["expected_sha256"],
                f"wrong proof file binding: {module}")
        for key, name in (("lakefile_sha256", "lakefile.toml"),
                          ("manifest_sha256", "lake-manifest.json")):
            require(record["revision"].get(key) == proof_inputs[name], f"proof configuration drift: {module}")
        for key, value in config["service_versions"].items():
            require(record["revision"].get(key) == value, f"wrong proof service version: {key}")
        diagnostics = record["result"].get("diagnostics", [])
        require(not any(m.get("severity") in ("error", "warning") for m in diagnostics),
                f"proof diagnostics: {module}")
        kernel_command = ["lake", "env", "lean", code_file] if module in config["audit_roots"] else ["lake", "build", module]
        run(kernel_command, proof, output / "evidence" / f"{module}-kernel.log", env)
        if module in config["audit_roots"]:
            audit = "\n".join(strings(record["result"]))
            for root in config["audit_roots"][module]:
                pattern = re.escape("'" + root + "'") + r" (?:does not depend on any axioms|depends on axioms: \[([^]]*)\])"
                matched = re.search(pattern, audit)
                require(matched is not None, f"missing actual axiom audit: {root}")
                axioms = {x.strip() for x in (matched.group(1) or "").split(",") if x.strip()}
                require(axioms <= {"propext", "Quot.sound", "Classical.choice"},
                        f"nonstandard proof assumption: {root}")
    run(["lake", "build"], proof, output / "evidence/full-experiment-kernel.log", env)
    verify_files(proof, proof_inputs)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ("baseline-archive", "source-repository", "tool-root", "mcp-runtime",
                "lake-packages", "cargo-home", "tmp-directory", "output"):
        parser.add_argument("--" + key, type=Path, required=True)
    args = parser.parse_args()
    for key, value in vars(args).items():
        setattr(args, key, value.resolve())
    require((3, 12) <= sys.version_info < (3, 14), "requires Python >=3.12,<3.14")
    require(platform.system() == "Linux" and platform.machine() == "x86_64", "requires x86_64 Linux")
    expected_inputs = checksums(PACKAGE / "inputs.sha256")
    actual_inputs = {str(p.relative_to(PACKAGE)) for p in PACKAGE.rglob("*") if p.is_file()
                     and p.name != "inputs.sha256"}
    require(actual_inputs == set(expected_inputs), "unregistered package input")
    verify_files(PACKAGE, expected_inputs)
    for path in (PACKAGE / "lean").rglob("*.lean"):
        text = re.sub(r"/-.*?-/", "", path.read_text(), flags=re.DOTALL)
        text = re.sub(r"--[^\n]*", "", text)
        require(re.search(r"\b(?:sorry|admit|axiom|native_decide|ofReduceBool)\b", text) is None,
                f"forbidden proof mechanism: {path.name}")
    config = json.loads((PACKAGE / "environment.json").read_text())
    require(config["schema"] == 1, "unsupported experiment schema")
    require(config["frozen_baseline"] == "0a23b57af60752f61067fc00b0b0b656e98347cb", "wrong baseline")
    verify_files(args.source_repository, config["service_files"])
    verify_files(args.source_repository, config["audit_files"])
    # Reuse the frozen project's existing map normalization, not another algorithm.
    normalizer_file = args.source_repository / "tools/check_foundations_extraction.py"
    audit_namespace = {"__file__": str(normalizer_file), "__name__": "frozen_data_permit_audit"}
    exec(compile(normalizer_file.read_bytes(), str(normalizer_file), "exec"), audit_namespace)
    llbc_digest = audit_namespace["llbc_digest"]
    verify_files(args.tool_root, config["tool_binaries"])
    for name, revision in config["tool_revisions"].items():
        actual = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=args.tool_root / name, text=True).strip()
        require(actual == revision, "wrong tool revision")
    output = args.output.resolve()
    require(not output.exists(), "output already exists")
    for protected in (PACKAGE, args.source_repository.resolve(), args.tool_root.resolve(),
                      args.mcp_runtime.resolve(), args.lake_packages.resolve(), args.cargo_home.resolve()):
        require(not protected.is_relative_to(output) and not output.is_relative_to(protected),
                "output overlaps a protected input")
    require(args.tmp_directory.is_dir() and args.tmp_directory.stat().st_uid == os.getuid()
            and args.tmp_directory.stat().st_mode & 0o077 == 0, "temporary directory must be private")
    env = dict(os.environ, CARGO_HOME=str(args.cargo_home.resolve()), CARGO_NET_OFFLINE="true",
               CARGO_TARGET_DIR=str(output / "target-tests"), TMPDIR=str(args.tmp_directory.resolve()),
               RUSTUP_TOOLCHAIN="1.97.0")
    for key in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_BUILD_RUSTFLAGS", "RUSTC_WRAPPER",
                "RUSTC_WORKSPACE_WRAPPER", "RUSTC", "RUSTDOC", "CARGO_BUILD_TARGET"):
        require(not env.get(key), f"unregistered compiler override: {key}")
    for command, exact in config["versions"]:
        actual = subprocess.check_output(command, env=env, text=True).strip()
        require(actual == exact, f"wrong installed version: {command}")
    output.mkdir(parents=True)
    (output / "evidence").mkdir()
    base_files = checksums(PACKAGE / "baseline-files.sha256")
    prepare(args.baseline_archive, output / "baseline", base_files)
    shutil.copytree(output / "baseline", output / "source")
    for kind in ("baseline", "source"):
        source = output / kind
        run([sys.executable, str(PACKAGE / "tests/install.py"), str(source),
             str(output / "evidence" / f"{kind}-test-overlay.json")], output,
            output / "evidence" / f"{kind}-install.log", env)
        run(["rustfmt", "--edition", "2024", "--config", "skip_children=true",
             "controller-service/src/lib.rs", "controller-service/src/controller/mod.rs",
             "controller-service/src/controller/tests_support.rs",
             "controller-service/src/application/coordinator.rs",
             "controller-service/src/data_permit_pilot_support.rs",
             "controller-service/src/data_permit_pilot_tests.rs"],
            source, output / "evidence" / f"{kind}-test-overlay-format.log", env, timeout=120)
        if kind == "source":
            (source / CORE).parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(PACKAGE / "core/permit_guards.rs", source / CORE)
            shutil.copyfile(PACKAGE / "core/data_access.rs", source / CALLER)
        require(altered(source, base_files) == config["overlays"][kind], f"wrong overlay: {kind}")
    bad = output / "proof-mutants/bypass-revoked/source"
    shutil.copytree(output / "source", bad)
    mutation = config["mutation"]
    text = (bad / CORE).read_text()
    require(text.count(mutation["before"]) == 1, "mutation context mismatch")
    (bad / CORE).write_text(text.replace(mutation["before"], mutation["after"]))
    require(sha(bad / CORE) == mutation["sha256"], "wrong mutant")
    baseline = cargo_tests(output / "baseline", "baseline", output, config, env)
    candidate = cargo_tests(output / "source", "source", output, config, env)
    require(baseline and baseline == candidate, "original/candidate semantic receipts differ/empty")
    cargo_tests(bad, "bad", output, config, env)
    config["tool_root"] = str(args.tool_root.resolve())
    extract(output / "source", "correct", "Permit", output, config, env, llbc_digest)
    extract(bad, "bypass-revoked", "BadPermit", output, config, env, llbc_digest)
    proof_checks(output, args.source_repository, args.mcp_runtime, args.lake_packages, config, env)
    verify_files(PACKAGE, expected_inputs)
    verify_files(args.tool_root, config["tool_binaries"])
    verify_files(args.source_repository, config["service_files"])
    verify_files(args.source_repository, config["audit_files"])
    # Check source drift after all tools; Cargo output remains outside source copies.
    require(altered(output / "baseline", base_files) == config["overlays"]["baseline"], "baseline drift")
    require(altered(output / "source", base_files) == config["overlays"]["source"], "candidate drift")
    bad_expected = dict(config["overlays"]["source"], **{CORE: mutation["sha256"]})
    require(altered(bad, base_files) == bad_expected, "mutant drift")
    receipt = {"success": True, "frozen_baseline": config["frozen_baseline"],
               "environment_sha256": sha(PACKAGE / "environment.json"),
               "inputs_sha256": sha(PACKAGE / "inputs.sha256"),
               "original_candidate_receipts": baseline, "tests": config["tests"],
               "proof_modules": config["proof_order"], "outputs": tree(output / "evidence"),
               "scope": "isolated finite guards; trusted A/B and reviewed caller binding remain"}
    (output / "receipt.json").write_text(json.dumps(receipt, indent=2, ensure_ascii=False) + "\n")
    print(json.dumps({"success": True, "receipt": str(output / "receipt.json")}))


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, ValueError, subprocess.SubprocessError, KeyError) as error:
        print(f"data-permit experiment failed: {error}", file=sys.stderr)
        sys.exit(1)
