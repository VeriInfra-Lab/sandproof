"""Reproduce both actual TLS candidate roots and the same-policy first/last fault."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time

try:
    from tools import check_tls_pin_contract as contract
except ModuleNotFoundError:
    import check_tls_pin_contract as contract


def reproduce(root, tool_root, output, native_only=False):
    root = root.resolve()
    tool_root = tool_root.resolve()
    output = output.resolve()
    if (output.exists() or output == root or root.is_relative_to(output)
            or any(output.is_relative_to(root / name) for name in (".git", ".lake", ".tools"))):
        raise ValueError("output must be a fresh private evidence directory")
    if sys.platform != "linux":
        raise ValueError("actual TLS candidate reproduction requires the frozen Linux environment")
    environment = contract.check_artifacts(root)
    contract.check_sources(root, environment)
    if not native_only:
        contract.check_tools(root, tool_root)
    for name in ("RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_BUILD_RUSTFLAGS", "CARGO_BUILD_TARGET"):
        if os.environ.get(name):
            raise ValueError("unreviewed compiler override: " + name)
    # An archive/clone of HEAD must contain exactly the package that was admitted.
    for line in (root / "verification/tls-pin-inputs.sha256").read_text().splitlines():
        digest, relative = line.split("  ", 1)
        committed = subprocess.check_output(["git", "-C", str(root), "show", "HEAD:" + relative])
        if hashlib.sha256(committed).hexdigest() != digest or contract.sha(root / relative) != digest:
            raise ValueError("commit the exact reviewed input before reproduction: " + relative)
    output.mkdir(parents=True)
    lean_env = os.environ.copy()
    lean_env.setdefault("LEAN_NUM_THREADS", "8")
    lean_env.setdefault("LEAN_STACK_SIZE_KB", "65536")
    head = subprocess.check_output(["git", "-C", str(root), "rev-parse", "HEAD"], text=True).strip()
    rows = []
    source_before = {name: contract.sha(root / name) for name in environment["source_files"]}
    before_inputs = contract.sha(root / "verification/tls-pin-inputs.sha256")

    def run(label, command, project=root, env=None, expected=0):
        if command[0] == 'rustfmt' and '--check' in command:
            try:
                from tools.acceptance_receipts import native_run
            except ModuleNotFoundError:
                from acceptance_receipts import native_run
            path = output / (label + '.log')
            native_run('tls-' + label, command, project, path, env, expected)
            return path
        start = time.monotonic()
        path = output / (label + ".log")
        with path.open("x") as log:
            process = subprocess.Popen(command, cwd=project, env=env, stdout=log, stderr=subprocess.STDOUT)
            (output / (label + ".pid")).write_text(str(process.pid))
            code = process.wait()
        row = {"label": label, "command": command, "exit_code": code, "expected_exit": expected,
               "seconds": round(time.monotonic() - start, 3), "log_sha256": contract.sha(path)}
        rows.append(row)
        (output / "steps.json").write_text(json.dumps(rows, indent=2) + "\n")
        if code != expected:
            raise ValueError(label + ": expected " + str(expected) + ", got " + str(code))
        return path

    def clone(name):
        project = output / name
        run(name + "-clone", ["git", "clone", "--quiet", "--local", "--no-hardlinks",
                             "--no-checkout", str(root), str(project)])
        run(name + "-checkout", ["git", "checkout", "--quiet", "--detach", head], project)
        (project / ".tools").symlink_to(tool_root, target_is_directory=True)
        return project

    def extract(project, label, mutant=False):
        saved = output / (label + "-previous-generated")
        saved.mkdir()
        for name in ("SandproofTls/Types.lean", "SandproofTls/Funs.lean", "translation.json"):
            old = project / contract.DEST / name
            target = saved / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.move(str(old), target)
        shutil.move(str(project / contract.LLBC), saved / "previous.llbc")
        env = os.environ.copy()
        env["RUSTFLAGS"] = "--remap-path-prefix=" + str(project) + "=."
        env["CARGO_TARGET_DIR"] = str(output / (label + "-charon-target"))
        env["CARGO_INCREMENTAL"] = "0"
        charon = tool_root / "aeneas/charon/bin/charon"
        command = [str(charon), "cargo", "--preset=aeneas", "--error-on-warnings",
                   "--sysroot", "default", "--targets", "x86_64-unknown-linux-gnu",
                   "--start-from", "crate::tls::verify_pin_bytes",
                   "--start-from", "crate::tls::leaf_pin_bytes", "--include", "crate::tls"]
        for name in ("rustls", "rustls_pki_types", "sha2", "digest", "generic_array", "typenum", "crypto_common"):
            command.extend(["--opaque", name])
        command.extend(["--dest-file", contract.LLBC, "--", "--manifest-path",
                        "sandbox-internal/Cargo.toml", "--lib", "--locked", "--offline",
                        "--ignore-rust-version"])
        run(label + "-charon", command, project, env)
        if not (project / contract.LLBC).is_file():
            raise ValueError("Charon did not generate the actual expected input")
        run(label + "-aeneas", [str(tool_root / "aeneas/bin/aeneas"), "-backend", "lean",
            "-abort-on-error", "-warnings-as-errors", "-split-files", "-emit-json",
            "-namespace", "SandproofTlsRust", "-subdir", "SandproofTls",
            "-dest", str(project / contract.DEST), contract.LLBC], project, env)
        # Preserve uncompiled external templates as evidence, never as assumptions.
        templates = output / (label + "-templates")
        templates.mkdir()
        for path in (project / contract.DEST).rglob("*_Template.lean"):
            shutil.move(str(path), templates / path.name)
        if mutant:
            external = project / contract.DEST / "SandproofTls/FunsExternal.lean"
            external.write_text(external.read_text() + """
namespace SandproofTlsRust
@[rust_fun "core::slice::{[@T]}::last"]
def core.slice.Slice.last {T : Type} (values : Aeneas.Std.Slice T) :
    Aeneas.Std.Result (Option T) := .ok values.val.getLast?
end SandproofTlsRust
""")
        contract.check_artifacts(project, mutant=mutant, frozen=not mutant)
        if not mutant:
            for relative in environment["generated_files"]:
                if (project / relative).read_bytes() != (root / relative).read_bytes():
                    raise ValueError("actual regenerated semantic input differs: " + relative)

    def native(project, label, mutant=False):
        try:
            from tools.acceptance_receipts import native_case
        except ModuleNotFoundError:
            from acceptance_receipts import native_case
        native_case('tls-' + label, project, output,
                    lambda: native_body(project, label, mutant))

    def native_body(project, label, mutant=False):
        native_project = clone(label + "-native")
        text = (project / contract.SOURCE).read_text()
        path = native_project / contract.SOURCE
        path.write_text(text + (root / "verification/fixtures/tls-pin-native.rs").read_text())
        run(label + "-native-format", ["rustfmt", "--edition", "2024", contract.SOURCE], native_project)
        if path.read_text().split("#[cfg(test)]", 1)[0].rstrip() != text.rstrip():
            raise ValueError("test overlay modified a production candidate statement")
        env = os.environ.copy()
        target = output / (label + "-native-target")
        if target.exists():
            raise ValueError("native target must be new")
        env["CARGO_TARGET_DIR"] = str(target)
        env["CARGO_INCREMENTAL"] = "0"
        log = run(label + "-native-build", ["cargo", "test", "--locked", "--offline",
            "-p", "sandproof-internal", "--lib", "--no-run", "--message-format=json"], native_project, env)
        binaries = []
        for line in log.read_text().splitlines():
            try:
                value = json.loads(line)
            except ValueError:
                continue
            if (value.get("reason") == "compiler-artifact" and value.get("executable")
                    and value.get("target", {}).get("name") == "sandproof_internal"
                    and value.get("profile", {}).get("test")):
                binaries.append(value["executable"])
        if len(binaries) != 1 or not Path(binaries[0]).is_relative_to(target):
            raise ValueError("missing actual fresh native test executable")
        if mutant:
            log = run(label + "-native-rejection", [binaries[0],
                "tls::tls_pin_candidate_tests::only_the_original_first_leaf_can_authorize",
                "--exact", "--nocapture"], native_project, env, 101)
            text = log.read_text()
            if not all(item in text for item in (
                    "a registered tail certificate cannot authorize an unregistered first leaf",
                    "test result: FAILED. 0 passed; 1 failed;", "left: Ok(())",
                    "right: Err(Authentication)")):
                raise ValueError("mutant failed for another reason")
        else:
            log = run(label + "-native-checks", [binaries[0], "tls_pin_candidate_tests", "--nocapture"],
                      native_project, env)
            if "test result: ok. 5 passed; 0 failed;" not in log.read_text():
                raise ValueError("candidate checks missing")
            log = run(label + "-secure-probe", ["cargo", "test", "--locked", "--offline",
                "-p", "sandproof-internal", "--test", "secure_probe", "--", "--nocapture"],
                native_project, env)
            if "test result: ok. 16 passed; 0 failed; 0 ignored;" not in log.read_text():
                raise ValueError("actual TLS checks missing or skipped")

    def lean_setup(project):
        (project / ".lake").mkdir()
        (project / ".lake/packages").symlink_to((root / ".lake/packages").resolve(), target_is_directory=True)

    def service(project, label, files):
        requests = [{"tool": "lean_diagnostic_messages", "arguments": {"file_path": name},
                     "expected_sha256": contract.sha(project / name)} for name in files]
        path = output / (label + "-service.log")
        env = lean_env.copy()
        text = "\n".join(json.dumps(item) for item in requests) + '\n{"tool":"exit"}\n'
        with path.open("x") as log:
            process = subprocess.Popen([str(project / ".tools/lean-mcp/bin/python"),
                "tools/lean_session.py"], cwd=project, env=env, stdin=subprocess.PIPE,
                stdout=log, stderr=subprocess.STDOUT)
            (output / (label + "-service.pid")).write_text(str(process.pid))
            process.communicate(text.encode())
        if process.returncode:
            raise ValueError("real Lean service transport failed")
        records = []
        for line in path.read_text().splitlines():
            try:
                row = json.loads(line)
            except ValueError:
                continue
            if row.get("request"):
                records.append(row)
        if len(records) != len(requests):
            raise ValueError("missing actual Lean request")
        for request, row in zip(requests, records):
            if (row["request"] != request or row["stale"]
                    or row["revision"]["file_sha256"] != request["expected_sha256"]):
                raise ValueError("stale or mismatched Lean source")
            result = row["result"]
            for _ in range(3):
                if "success" in result:
                    break
                result = result.get("structuredContent", result.get("result", {}))
            if (result.get("success") is not True or result.get("partial") is not False
                    or result.get("timed_out") or result.get("failed_dependencies")
                    or any(item.get("severity") in ("warning", "error") for item in result.get("items", []))):
                raise ValueError("Lean proof incomplete or rejected")
        return {"requests": len(records), "log_sha256": contract.sha(path)}

    env = lean_env
    correct = clone("correct")
    run("correct-install", ["git", "apply", str(root / contract.PATCH)], correct)
    if contract.sha(correct / contract.SOURCE) != environment["candidate_tls_sha256"]:
        raise ValueError("the exact formatted Rust candidate was not installed")
    run("correct-format", ["rustfmt", "--check", "--edition", "2024", contract.SOURCE], correct)
    if not native_only:
        extract(correct, "correct")
    native(correct, "correct")
    if not native_only:
        lean_setup(correct)
        run("correct-imports", ["lake", "build", "SandproofTls.Funs",
            "SandboxModel.Boundaries.TlsPin.Audit"], correct, env)
        correct_service = service(correct, "correct", [
            "SandboxModel/Boundaries/TlsPin/Refinement.lean",
            "SandboxModel/Boundaries/TlsPin/Calls.lean", "SandboxModel/Boundaries/TlsPin/Audit.lean"])
        log = run("correct-kernel-audit", ["lake", "env", "lean",
            "SandboxModel/Boundaries/TlsPin/Audit.lean"], correct, env)
        expected = {contract.NAMESPACE + name for name in contract.LAWS}
        found = {line.split("Checked ", 1)[1].split(":", 1)[0]
                 for line in log.read_text().splitlines() if "Checked " in line}
        if found != expected:
            raise ValueError("missing actual theorem dependency audit")
    
    wrong = clone("last-leaf")
    run("last-leaf-install", ["git", "apply", str(root / contract.PATCH)], wrong)
    path = wrong / contract.SOURCE
    original = path.read_text()
    begin = original.index("fn verify_pin_bytes")
    end = original.index("fn leaf_pin_bytes")
    region = original[begin:end]
    if region.count("chain.first()") != 1:
        raise ValueError("mutation no longer selects exactly the original guard")
    path.write_text(original[:begin] + region.replace("chain.first()", "chain.last()") + original[end:])
    if not native_only:
        extract(wrong, "last-leaf", True)
    native(wrong, "last-leaf", True)
    if native_only:
        return
    lean_setup(wrong)
    evidence = wrong / "TlsPinNegativeEvidence.lean"
    shutil.copyfile(root / "verification/fixtures/tls-pin-negative.lean.in", evidence)
    run("last-leaf-imports", ["lake", "build", "SandproofTls.Funs",
        "SandboxModel.Boundaries.TlsPin.Model", "SandboxModel.Boundaries.TlsPin.Library",
        "SandboxModel.ProofAuditCommand"], wrong, env)
    negative_service = service(wrong, "last-leaf", [evidence.name])
    log = run("last-leaf-kernel-audit", ["lake", "env", "lean", evidence.name], wrong, env)
    if "Checked SandboxModel.Boundaries.TlsPin.Negative.mutant_not_safe:" not in log.read_text():
        raise ValueError("missing same-property formal counterexample")
    if any(contract.sha(root / name) != digest for name, digest in source_before.items()):
        raise ValueError("original production inputs changed")
    if contract.sha(root / "verification/tls-pin-inputs.sha256") != before_inputs:
        raise ValueError("frozen inputs changed")
    receipt = {"success": True, "head": head, "candidate_only": True,
               "production_unchanged": True, "scope": "two isolated real Rust roots and explicit B4 composition",
               "same_property": contract.NAMESPACE + "RustPinSafe",
               "correct_service": correct_service, "negative_service": negative_service,
               "correct_native_tests": 5, "actual_tls_tests": 16,
               "mutant_compiled_and_translated": True, "mutant_native_expected_exit": 101,
               "formal_counterexample_checked": True, "steps": rows}
    (output / "receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print("TLS candidate exact regeneration, real Rust/TLS checks and same-property Lean counterexample passed.")
    print("Original borrowed production entry and outer B4/TLS binding are not thereby machine-refined.")

