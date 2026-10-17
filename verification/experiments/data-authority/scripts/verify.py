#!/usr/bin/env python3
"""Reproduce this frozen data-authority experiment using the original tools."""

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
CORE_FILES = {
    "data_credentials.rs": "controller-service/src/application/coordinator/data_credentials.rs",
    "credential_core.rs": "controller-service/src/application/coordinator/data_credentials/credential_core.rs",
    "byte_core.rs": "controller-service/src/application/coordinator/data_credentials/byte_core.rs",
    "admission.rs": "controller-service/src/controller/admission.rs",
    "lifetime_core.rs": "controller-service/src/controller/admission/lifetime_core.rs",
    "lifetime_bytes.rs": "controller-service/src/controller/admission/lifetime_bytes.rs",
}
AUTHORITY_ROOTS = ["credential_core::digest_matches", "credential_core::registered_digest",
    "credential_core::inverse_binding", "credential_core::byte_core::token_format",
    "lifetime_core::decode_row", "lifetime_core::check_clock", "lifetime_core::check_record",
    "lifetime_core::check_live"]
ACCESS_ROOTS = ["cidr_contains", "protected_ipv4", "destination_allowed", "parse_ipv4_bits",
    "egress", "ingress", "clone_world", "initial_world", "runtime_crash", "runtime_recover",
    "slot_transition", "controller_input", "runtime", "create", "access_exec", "access_file_read",
    "access_file_write", "access"]
FORMAT_TESTS = ["controller-service/src/lib.rs", "controller-service/src/controller/mod.rs",
    "controller-service/src/controller/tests_support.rs", "controller-service/src/application/coordinator.rs",
    "controller-service/src/data_authority_pilot_support.rs", "controller-service/src/data_authority_pilot_tests.rs",
    "controller-core/src/lib.rs", "controller-core/src/data_authority_pilot.rs"]


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
         "--features", "sandbox-controller-core/data-authority-pilot",
         "--lib", "data_authority_pilot_tests::da_", "--", "--nocapture", "--test-threads=1"],
        source, output / "evidence" / f"{label}-tests.log", test_env,
        timeout=900, expected=101 if label.startswith("bad-") else 0,
    )
    counts = re.findall(r"test result: (?:ok|FAILED)\. (\d+) passed; (\d+) failed;.*?(\d+) filtered out", text)
    require(len(counts) == 1, f"missing/ambiguous test count: {label}")
    passed, failed, _filtered = map(int, counts[0])
    wanted = config["tests"][label]
    require([passed, failed] == wanted["counts"], f"wrong test coverage: {label}")
    if label.startswith("bad-"):
        for case in wanted["failing_cases"]:
            require(re.search(r"(?m)^\s{4}" + re.escape(case) + r"\s*$", text) is not None,
                    f"wrong negative failure: {case}")
        require("panicked at" in text and "error[E" not in text, "not a compiled assertion failure")
        require(wanted["assertion"] in text, "wrong negative business assertion")
    # libtest may print its `test NAME ... ` prefix on the first receipt's line.
    receipts = sorted(line[line.index("DA_RECEIPT "):].strip()
                      for line in text.splitlines() if "DA_RECEIPT " in line)
    if not label.startswith("bad-"):
        require(len(receipts) == config["tests"]["receipt_count"], "missing semantic receipt")
        run(["cargo", "clippy", "--locked", "--offline", "-p", "sandproof-controller-service",
             "--all-targets", "--features", "sandbox-controller-core/data-authority-pilot",
             "--", "-D", "warnings"], source, output / "evidence" / f"{label}-clippy.log",
            test_env, timeout=900)
    return receipts


def extract(source, family, label, namespace, output, config, env, llbc_digest):
    workspace = output / family
    probe = workspace / "probes" / label
    probe.mkdir(parents=True)
    tool = Path(config["tool_root"])
    extraction = dict(env, RUSTUP_TOOLCHAIN="nightly-2026-08-18",
                      CARGO_TARGET_DIR=str(workspace / f"target-extract-{label}"))
    command = [str(tool / "charon/bin/charon")]
    if family == "credentials":
        credential = os.path.relpath(source / CORE_FILES["credential_core.rs"], probe)
        lifetime = os.path.relpath(source / CORE_FILES["lifetime_core.rs"], probe)
        (probe / "entry.rs").write_text(f'#[path = "{credential}"]\npub mod credential_core;\n'
                                       f'#[path = "{lifetime}"]\npub mod lifetime_core;\n')
        command += ["rustc", "--preset=aeneas", "--error-on-warnings", "--sysroot", "default"]
        roots = ["credential_core::digest_matches"] if label == "digest" else AUTHORITY_ROOTS
        for root in roots:
            command += ["--start-from", "crate::" + root]
        command += ["--include", "crate", "--dest-file", f"probes/{label}/input.llbc", "--", "--edition",
                    "2024", "--crate-type", "lib", "--crate-name", "credential_lifetime_entry",
                    "--remap-path-prefix", f"{workspace}=.", str(probe / "entry.rs")]
        cwd = workspace
    else:
        extraction["RUSTFLAGS"] = f"--remap-path-prefix={source}=."
        command += ["cargo", "--preset=aeneas", "--error-on-warnings", "--sysroot", "default"]
        for root in ACCESS_ROOTS:
            command += ["--start-from", "crate::" + root]
        command += ["--include", "sandbox_controller_core", "--dest-file",
                    os.path.relpath(probe / "input.llbc", source / "verification/extract-network"), "--",
                    "--manifest-path", "verification/extract-network/Cargo.toml", "--lib", "--locked", "--offline"]
        cwd = source
    run(command, cwd, probe / "charon.log", extraction, timeout=300)
    actual_llbc = json.loads((probe / "input.llbc").read_text())
    expected_llbc = json.loads((PACKAGE / "llbc" / family / f"{label}.llbc").read_text())
    require(llbc_digest(actual_llbc) == llbc_digest(expected_llbc), f"LLBC drift: {family}/{label}")
    require(actual_llbc.get("has_errors") is False, "Charon reported errors")
    llbc = actual_llbc["translated"]
    require(llbc["crate_name"] == ("credential_lifetime_entry" if family == "credentials"
                                   else "sandproof_network_extract"), "wrong extraction crate")
    require({x["key"] for x in llbc["target_information"]} == {config["target"]}, "wrong extraction target")
    functions = [f for f in llbc["fun_decls"] if f is not None]
    for name in config["extractions"][family][label]["body_names"]:
        found = [f for f in functions if f["item_meta"]["name"] == name]
        require(len(found) == 1 and isinstance(found[0]["body"], dict)
                and '"Error"' not in json.dumps(found[0]["body"]), "missing/error business body")
    require(not llbc["options"]["opaque"], "unexpected opaque business rules")
    file_base = cwd if family == "credentials" else source / "verification/extract-network"
    for relative in config["extractions"][family][label]["source_bindings"]:
        core = source / relative
        # Original Charon embeds local crate text. Remapped dependency-crate
        # files are Virtual with no text; their exact paths and source hashes
        # are bound by the frozen Cargo source closure before and after tools.
        require(any(("Local" in f["name"] and (file_base / f["name"]["Local"]).resolve() == core.resolve()
                     and f["contents"] == core.read_text()) or
                    (family == "access" and f["name"] == {"Virtual": "./" + relative}
                     and f["contents"] is None) for f in llbc["files"]),
                f"LLBC is not bound to actual source: {relative}")
    command = [str(tool / "bin/aeneas"), "-backend", "lean", "-abort-on-error",
               "-warnings-as-errors", "-split-files"]
    if family == "access":
        command += ["-emit-json"]
    subdir = "SandproofNetwork" if family == "access" else namespace
    command += ["-namespace", namespace, "-subdir", subdir,
                "-dest", str(probe / "lean"), str(probe / "input.llbc")]
    run(command, cwd, probe / "aeneas.log", extraction, timeout=300)
    expected = PACKAGE / "generated" / family / label
    actual = probe / "lean" / subdir
    if family == "credentials":
        require({p.name for p in actual.glob("*.lean")} == {"Types.lean", "Funs.lean"},
                "unexpected credential/lifetime external interface")
    for name in ("Types.lean", "Funs.lean"):
        require((actual / name).read_bytes() == (expected / name).read_bytes(),
                f"generated Lean drift: {namespace}/{name}")


def worker_retry_allowed(record):
    result = record.get("result", {})
    content = result.get("content", [])
    return (record.get("stale") is False and result.get("isError") is True
            and result.get("timed_out") in (None, False) and not result.get("diagnostics")
            and result.get("success") is not True and result.get("resultType") == "complete"
            and len(content) == 1 and content[0].get("type") == "text"
            and re.fullmatch(r"Error executing tool lean_run_code: File worker for [^\r\n]+ died: "
                r"Server process for [^\r\n]+ crashed, likely due to a stack overflow or a bug\.",
                content[0].get("text", "")) is not None)


def service_record(proof, runtime, request, log, env, config, proof_inputs, retries):
    def exchange():
        with log.open("w") as stream:
            result = subprocess.run([str(runtime / "bin/python"), "tools/lean_session.py"],
                input=json.dumps(request) + '\n{"tool":"exit"}\n', text=True,
                cwd=proof, env=env, stdout=stream, stderr=subprocess.STDOUT,
                timeout=600, check=False)
        require(result.returncode == 0, "service transport failed")
        records = []
        for line in log.read_text().splitlines():
            try:
                item = json.loads(line)
            except ValueError:
                continue
            if "request" in item:
                records.append(item)
        require(len(records) == 1, "missing service record")
        return records[0]

    record = exchange()
    if worker_retry_allowed(record):
        # Only this explicit worker-death tool error can retry once. No proof
        # diagnostic, stale result, timeout, transport or import error retries.
        verify_files(proof, proof_inputs)
        require(record["revision"].get("file_sha256") == request["expected_sha256"], "worker source drift")
        for key, name in (("lakefile_sha256", "lakefile.toml"), ("manifest_sha256", "lake-manifest.json")):
            require(record["revision"].get(key) == proof_inputs[name], "worker config drift")
        for key, value in config["service_versions"].items():
            require(record["revision"].get(key) == value, "worker service version drift")
        original = log.with_name(log.stem + "-first-worker-error.log")
        require(not original.exists(), "worker retry already used")
        log.rename(original)
        retries.append({"code_file": request["code_file"], "file_sha256": request["expected_sha256"],
                        "lakefile_sha256": proof_inputs["lakefile.toml"],
                        "manifest_sha256": proof_inputs["lake-manifest.json"],
                        "first_worker_error_log": original.name, "final_service_log": log.name,
                        "retries": 1})
        record = exchange()
    return record


def proof_checks(label, output, source_repository, runtime, packages, config, env):
    config = dict(config, **config["proof_projects"][label])
    proof = output / f"proof-{label}"
    proof.mkdir()
    for original, destination in config["baseline_files"].items():
        target = proof / destination
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(output / "pristine" / original, target)
    for original, destination in config["package_files"].items():
        target = proof / destination
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(PACKAGE / original, target)
    for name in config["service_files"]:
        target = proof / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source_repository / name, target)
        require(sha(target) == config["service_files"][name], "client/launcher copy drift")
    proof_inputs = tree(proof)
    retries = []
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
                output / "evidence" / f"{label}-build-{prerequisite}.log", env, timeout=600)
        code_file = f"audits/{module}.lean" if module in config["audit_roots"] else f"lean/{module}.lean"
        request = {"tool": "lean_run_code", "code_file": code_file,
                   "expected_sha256": sha(proof / code_file)}
        log = output / "evidence" / f"{label}-{module}-service.log"
        record = service_record(proof, runtime, request, log, env, config, proof_inputs, retries)
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
        run(kernel_command, proof, output / "evidence" / f"{label}-{module}-kernel.log", env, timeout=600)
        if module in config["audit_roots"]:
            audit = "\n".join(strings(record["result"]))
            for root in config["audit_roots"][module]:
                pattern = re.escape("'" + root + "'") + r" (?:does not depend on any axioms|depends on axioms: \[([^]]*)\])"
                matched = re.search(pattern, audit)
                require(matched is not None, f"missing actual axiom audit: {root}")
                axioms = {x.strip() for x in (matched.group(1) or "").split(",") if x.strip()}
                require(axioms <= {"propext", "Quot.sound", "Classical.choice"},
                        f"nonstandard proof assumption: {root}")
    for attempt in config.get("expected_failures", []):
        file = attempt["file"]
        name = Path(file).stem
        for prerequisite in attempt.get("prerequisites", []):
            run(["lake", "build", prerequisite], proof,
                output / "evidence" / f"{label}-{name}-build-{prerequisite}.log", env, timeout=600)
        request = {"tool": "lean_run_code", "code_file": file,
                   "expected_sha256": sha(proof / file)}
        log = output / "evidence" / f"{label}-{name}-rejected-service.log"
        record = service_record(proof, runtime, request, log, env, config, proof_inputs, retries)
        require(record.get("stale") is False and record["result"].get("success") is False
                and record["result"].get("timed_out") is False, "not a fresh mathematical rejection")
        require(record["revision"].get("file_sha256") == request["expected_sha256"], "negative file drift")
        for key, source in (("lakefile_sha256", "lakefile.toml"), ("manifest_sha256", "lake-manifest.json")):
            require(record["revision"].get(key) == proof_inputs[source], "negative config drift")
        for key, value in config["service_versions"].items():
            require(record["revision"].get(key) == value, "negative service version drift")
        diagnostics = "\n".join(strings(record["result"].get("diagnostics", [])))
        require("unsolved goals" in diagnostics and "⊢ False" in diagnostics,
                "negative failed for a reason other than its false safety goal")
        require(not any(text in diagnostics for text in ("unknown identifier", "unknown module", "unknown namespace",
                    "type mismatch", "failed to synthesize", "unexpected token", "invalid field")),
                "negative import/type/environment failure")
        run(["lake", "env", "lean", file], proof,
            output / "evidence" / f"{label}-{name}-rejected-kernel.log", env, timeout=600, expected=1)
    run(["lake", "build"], proof, output / "evidence" / f"{label}-full-experiment-kernel.log", env, timeout=600)
    verify_files(proof, proof_inputs)
    return {"input_sha256": proof_inputs, "proof_modules": config["proof_order"],
            "audit_roots": config["audit_roots"], "expected_failures": config.get("expected_failures", []),
            "service_retries": retries}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ("baseline-archive", "source-repository", "data-permit-package", "tool-root", "mcp-runtime",
                "lake-packages", "cargo-home", "tmp-directory", "output"):
        parser.add_argument("--" + key, type=Path, required=True)
    parser.add_argument("--phase", choices=("all", "extraction", "proof"), default="all",
                        help="Partial runs never emit a full reproduction receipt")
    args = parser.parse_args()
    for key, value in vars(args).items():
        if isinstance(value, Path):
            setattr(args, key, value.resolve())
    require((3, 12) <= sys.version_info < (3, 14), "requires Python >=3.12,<3.14")
    require(platform.system() == "Linux" and platform.machine() == "x86_64", "requires x86_64 Linux")
    expected_inputs = checksums(PACKAGE / "inputs.sha256")
    actual_inputs = {str(p.relative_to(PACKAGE)) for p in PACKAGE.rglob("*") if p.is_file()
                     and p.name != "inputs.sha256"}
    require(actual_inputs == set(expected_inputs), "unregistered package input")
    verify_files(PACKAGE, expected_inputs)
    for path in PACKAGE.rglob("*.lean"):
        text = re.sub(r"/-.*?-/", "", path.read_text(), flags=re.DOTALL)
        text = re.sub(r"--[^\n]*", "", text)
        require(re.search(r"\b(?:sorry|admit|axiom|native_decide|ofReduceBool)\b", text) is None,
                f"forbidden proof mechanism: {path.name}")
    config = json.loads((PACKAGE / "environment.json").read_text())
    require(config["schema"] == 1, "unsupported experiment schema")
    require(config["frozen_baseline"] == "0a23b57af60752f61067fc00b0b0b656e98347cb", "wrong baseline")
    if args.phase in ("all", "proof"):
        require(set(config["proof_projects"]) == {"correct", "digest", "expiry", "access"},
                "incomplete proof project registration")
    require(sha(args.data_permit_package / "inputs.sha256") == config["data_permit_inputs_sha256"],
            "wrong frozen data-permit package")
    permit_inputs = checksums(args.data_permit_package / "inputs.sha256")
    require({str(p.relative_to(args.data_permit_package)) for p in args.data_permit_package.rglob("*")
             if p.is_file() and p.name != "inputs.sha256"} == set(permit_inputs),
            "unregistered data-permit dependency input")
    verify_files(args.data_permit_package, permit_inputs)
    verify_files(args.source_repository, config["service_files"])
    verify_files(args.source_repository, config["audit_files"])
    # Reuse the frozen project's existing map normalization, not another algorithm.
    normalizer_file = args.source_repository / "tools/check_foundations_extraction.py"
    audit_namespace = {"__file__": str(normalizer_file), "__name__": "frozen_data_authority_audit"}
    exec(compile(normalizer_file.read_bytes(), str(normalizer_file), "exec"), audit_namespace)
    llbc_digest = audit_namespace["llbc_digest"]
    verify_files(args.tool_root, config["tool_binaries"])
    for name, revision in config["tool_revisions"].items():
        actual = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=args.tool_root / name, text=True).strip()
        require(actual == revision, "wrong tool revision")
    for dependency in config["lake_dependencies"]:
        directory = args.lake_packages / dependency["name"]
        require(subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=directory,
                text=True).strip() == dependency["rev"], "wrong Lake dependency")
        require(not subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=no"],
                cwd=directory, text=True).strip(), "modified Lake dependency")
    output = args.output.resolve()
    require(not output.exists(), "output already exists")
    for protected in (PACKAGE, args.source_repository.resolve(), args.data_permit_package.resolve(), args.tool_root.resolve(),
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
    prepare(args.baseline_archive, output / "pristine", base_files)
    for kind in ("baseline", "source"):
        shutil.copytree(output / "pristine", output / kind)
    for kind in ("baseline", "source"):
        source = output / kind
        run([sys.executable, str(PACKAGE / "tests/install.py"), str(source),
             str(output / "evidence" / f"{kind}-test-overlay.json")], output,
            output / "evidence" / f"{kind}-install.log", env)
        run(["rustfmt", "--edition", "2024", "--config", "skip_children=true", *FORMAT_TESTS],
            source, output / "evidence" / f"{kind}-test-overlay-format.log", env, timeout=120)
        if kind == "source":
            for name, relative in {
                "data_access.rs": "controller-service/src/application/coordinator/data_access.rs",
                "permit_guards.rs": "controller-service/src/application/coordinator/data_access/permit_guards.rs",
            }.items():
                target = source / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(args.data_permit_package / "core" / name, target)
            for name, relative in CORE_FILES.items():
                target = source / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(PACKAGE / "core" / name, target)
            shutil.copyfile(PACKAGE / "wrappers/network.rs", source / "verification/extract-network/src/lib.rs")
        require(altered(source, base_files) == config["overlays"][kind], f"wrong overlay: {kind}")
    for label in ("digest", "expiry", "access"):
        bad = output / f"bad-{label}"
        shutil.copytree(output / "source", bad)
        mutation = config["mutations"][label]
        path = bad / mutation["path"]
        require(sha(path) == mutation["before_sha256"], "wrong mutation input")
        text = path.read_text()
        require(text.count(mutation["before"]) == 1, "mutation context mismatch")
        path.write_text(text.replace(mutation["before"], mutation["after"], 1))
        require(sha(path) == mutation["after_sha256"], "wrong mutant")
        require(altered(bad, base_files) == config["overlays"][f"bad-{label}"], "wrong mutant overlay")
    baseline = []
    if args.phase == "all":
        baseline = cargo_tests(output / "baseline", "baseline", output, config, env)
        candidate = cargo_tests(output / "source", "source", output, config, env)
        require(baseline and baseline == candidate, "original/candidate semantic receipts differ/empty")
        for label in ("digest", "expiry", "access"):
            cargo_tests(output / f"bad-{label}", f"bad-{label}", output, config, env)
        run([sys.executable, str(PACKAGE / "tests/summarize.py"), str(output)], output,
            output / "evidence/business-summary.log", env)
    config["tool_root"] = str(args.tool_root.resolve())
    if args.phase in ("all", "extraction"):
        for label, source, namespace in (("correct", "source", "Authority"),
                ("digest", "bad-digest", "BadDigest"), ("expiry", "bad-expiry", "BadExpiry")):
            extract(output / source, "credentials", label, namespace, output, config, env, llbc_digest)
        for label, source in (("correct", "source"), ("bypass-capability", "bad-access")):
            extract(output / source, "access", label, "SandproofNetworkRust", output, config, env, llbc_digest)
    projects = {}
    if args.phase in ("all", "proof"):
        for label in ("correct", "digest", "expiry", "access"):
            projects[label] = proof_checks(label, output, args.source_repository, args.mcp_runtime,
                                           args.lake_packages, config, env)
    verify_files(PACKAGE, expected_inputs)
    verify_files(args.tool_root, config["tool_binaries"])
    verify_files(args.source_repository, config["service_files"])
    verify_files(args.source_repository, config["audit_files"])
    verify_files(args.data_permit_package, permit_inputs)
    for name, revision in config["tool_revisions"].items():
        require(subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=args.tool_root / name,
                text=True).strip() == revision, "final tool revision drift")
    for dependency in config["lake_dependencies"]:
        directory = args.lake_packages / dependency["name"]
        require(subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=directory,
                text=True).strip() == dependency["rev"], "final Lake revision drift")
        require(not subprocess.check_output(["git", "status", "--porcelain", "--untracked-files=no"],
                cwd=directory, text=True).strip(), "final Lake source drift")
    # Check source drift after all tools; Cargo output remains outside source copies.
    for label in ("baseline", "source", "bad-digest", "bad-expiry", "bad-access"):
        require(altered(output / label, base_files) == config["overlays"][label], f"final source drift: {label}")
    receipt = {"success": True, "full_reproduction": args.phase == "all", "phase": args.phase,
               "frozen_baseline": config["frozen_baseline"],
               "environment_sha256": sha(PACKAGE / "environment.json"),
               "inputs_sha256": sha(PACKAGE / "inputs.sha256"),
               "original_candidate_receipts": baseline, "tests": config["tests"],
               "data_permit_inputs_sha256": sha(args.data_permit_package / "inputs.sha256"),
               "source_closure": {"baseline_manifest_sha256": sha(PACKAGE / "baseline-files.sha256"),
                                  "overlays_before_and_after": config["overlays"]},
               "source_binding": {"credentials": "Local path plus embedded text and complete source closure",
                                  "access": "Local wrapper text; exact Virtual dependency paths with contents=None plus complete compiled Cargo source closure"},
               "proof_projects": projects, "outputs": tree(output / "evidence"),
               "scope": "isolated real synchronous redispatch decisions; query, dependency and caller binding obligations remain"}
    destination = output / ("receipt.json" if args.phase == "all" else f"partial-{args.phase}.json")
    destination.write_text(json.dumps(receipt, indent=2, ensure_ascii=False) + "\n")
    print(json.dumps({"success": True, "full_reproduction": args.phase == "all", "receipt": str(destination)}))


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, ValueError, subprocess.SubprocessError, KeyError) as error:
        print(f"data-authority experiment failed: {error}", file=sys.stderr)
        sys.exit(1)
