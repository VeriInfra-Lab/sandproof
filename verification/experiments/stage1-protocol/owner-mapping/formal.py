"""Develop the frozen, unadopted owner-mapping candidate and real Rust mutants."""
import argparse
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time
import traceback

ROOT = Path(__file__).resolve().parents[4]
HERE = Path("verification/experiments/stage1-protocol/owner-mapping")
GEN = Path("verification/aeneas/stage1-protocol/owner-mapping/SandproofStage1OwnerMapping")
PROOFS = Path("SandboxModel/Lifecycle/Protocol")


def module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


parent = module("owner_existing_load_inputs", ROOT / HERE.parent / "inbox-load/formal.py")
runtime = module("owner_existing_runtime_mapping", ROOT / HERE.parent / "formal_runtime.py")
admission = module("owner_retained_root_admission", ROOT / HERE / "admission.py")
checker = module("owner_original_actual_files", ROOT / HERE.parent / "formal.py")
resolver = module("owner_original_closure", ROOT / "tools/proof_batch.py")
sha = parent.sha


def validate_sources(inv):
    for name, expected in inv["source_inputs"].items():
        if sha(ROOT / name) != expected:
            raise ValueError("reviewed production input differs: " + name)
    if sha(Path(__file__)) != inv["driver_sha256"] or sha(ROOT / HERE / "admission.py") != inv["admission_sha256"]:
        raise ValueError("reviewed driver differs")
    for name, expected in inv["templates"].items():
        if sha(ROOT / HERE / name) != expected:
            raise ValueError("reviewed template differs: " + name)
    for name, expected in inv["providers"].items():
        if sha(ROOT / name) != expected:
            raise ValueError("existing provider differs: " + name)
    if sha(ROOT / HERE / "original-owner-for.rs.in") != inv["candidate_original_body_sha256"]:
        raise ValueError("original owner body differs")
    if sha(ROOT / HERE / "candidate-owner-for.rs.in") != inv["candidate_body_sha256"]:
        raise ValueError("frozen candidate body differs")
    checked = {}
    for kind, expected in inv["std_cmp_sources"].items():
        flags = [] if kind == "production" else ["+nightly-2026-08-18"]
        prefix = Path(subprocess.check_output(["rustc", *flags, "--print", "sysroot"], text=True).strip())
        path = prefix / "lib/rustlib/src/rust/library/core/src/cmp.rs"
        if sha(path) != expected["sha256"]:
            raise ValueError("fixed Rust cmp source differs: " + kind)
        text = path.read_text()
        start = text.index("fn max(self, other: Self) -> Self")
        body = text[start:text.index("\n    }", start) + 6]
        import hashlib
        if hashlib.sha256(body.encode()).hexdigest() != expected["max_body_sha256"]:
            raise ValueError("original default max body differs")
        checked[kind] = dict(path=str(path), sha256=sha(path), max_body=body)
    if checked["production"]["max_body"] != checked["extraction"]["max_body"]:
        raise ValueError("the two reviewed default-method bodies differ")
    for name, expected in inv["fixed_library_model_sources"].items():
        paths = [ROOT / ".tools/aeneas/backends/lean/Aeneas/Std" / name,
                 ROOT / ".lake/packages/aeneas/backends/lean/Aeneas/Std" / name]
        if any(sha(path) != expected for path in paths):
            raise ValueError("actual fixed backend/package model differs: " + name)
    return checked


def run(output, variant, inv, inputs, modes):
    output.mkdir(mode=0o700)
    started = time.monotonic()
    rec = dict(kind="unadopted-owner-mapping-development", variant=variant, commands=[],
        capture_complete=False, translation_complete=False, proof_complete=False,
        property_rejected=False, production_adopted=False, public_Ord_binding_adopted=False,
        original_equivalence_proved=False, generated_bodies_modified=False,
        new_axioms=False, native_checks_run=False, formal_acceptance=False,
        ning_acceptance=False, shared_acceptance=False, full_v3=False)
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE="1", LEAN_NUM_THREADS="4",
               LEAN_STACK_SIZE_KB="65536", CARGO_NET_OFFLINE="true")

    def command(project, folder, label, argv):
        log = folder / (label + ".log")
        begin = time.monotonic()
        with log.open("xb") as stream:
            result = subprocess.run(argv, cwd=project, env=env, stdin=subprocess.DEVNULL,
                                    stdout=stream, stderr=subprocess.STDOUT, timeout=1800)
        rec["commands"].append(dict(label=label, argv=argv, cwd=str(project),
            exit_code=result.returncode, seconds=time.monotonic()-begin, log_sha256=sha(log)))
        (output / "progress.json").write_text(json.dumps(rec, indent=2) + "\n")
        if result.returncode:
            raise ValueError("command failed: " + str(log))

    try:
        recipe = inv["variants"][variant]
        rec.update(inputs=inputs, input_modes=modes,
            source_commit=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
            source_tree=subprocess.check_output(["git", "rev-parse", "HEAD^{tree}"], cwd=ROOT, text=True).strip())
        rec["std_sources"] = validate_sources(inv)
        fixed = json.loads((ROOT / "verification/foundations-environment.json").read_text())
        for name, suffix in [("charon", "charon/bin/charon"), ("aeneas", "bin/aeneas")]:
            if sha(ROOT / ".tools/aeneas" / suffix) != fixed[name]["binary_sha256"]:
                raise ValueError("fixed tool differs: " + name)
        rec["tools"] = fixed
        project = output / "source"
        project.mkdir(mode=0o700)
        expected = dict(inputs)
        for name, digest in inputs.items():
            destination = project / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / name, destination)
            if sha(destination) != digest:
                raise ValueError("source changed during copy: " + name)
        (project / ".tools").symlink_to(runtime.tool_root(ROOT), target_is_directory=True)
        (project / ".lake").mkdir()
        (project / ".lake/packages").symlink_to(runtime.package_root(ROOT), target_is_directory=True)
        (project / "artifacts/extract-target").mkdir(parents=True)

        source = project / inv["source"]
        original, candidate = [(ROOT / HERE / name).read_bytes()
            for name in ("original-owner-for.rs.in", "candidate-owner-for.rs.in")]
        if source.read_bytes().count(original) != 1:
            raise ValueError("original business body is not unique")
        mutation = recipe["mutation"]
        if mutation is not None:
            before, after = mutation["before"].encode(), mutation["after"].encode()
            if candidate.count(before) != 1:
                raise ValueError("real Rust mutation is not unique")
            candidate = candidate.replace(before, after, 1)
        source.write_bytes(source.read_bytes().replace(original, candidate, 1)
                           + (ROOT / HERE / "root.rs.in").read_bytes())
        expected[inv["source"]] = sha(source)
        rec["private_source_derivation"] = dict(path=inv["source"],
            original_source_sha256=inputs[inv["source"]], derived_source_sha256=sha(source),
            private_candidate=True, mutation=mutation, production_modified=False)

        paths = [Path.home(), runtime.tool_root(ROOT), runtime.package_root(ROOT)]
        for key in ("CARGO_HOME", "RUSTUP_HOME", "ELAN_HOME", "SANDPROOF_PROOF_RUNTIME_ROOT"):
            if os.environ.get(key):
                paths.append(Path(os.environ[key]).resolve(strict=True))
        wrapper = ["bwrap", "--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc",
            "--tmpfs", "/tmp", "--tmpfs", "/home",
            *[a for path in dict.fromkeys(paths) for a in ("--ro-bind", str(path), str(path))],
            "--bind", str(project), str(runtime.VIRTUAL),
            "--chdir", str(runtime.VIRTUAL), "--setenv", "CARGO_TARGET_DIR",
            str(runtime.VIRTUAL / "artifacts/extract-target"), "--setenv", "CARGO_NET_OFFLINE", "true",
            "--setenv", "CARGO_BUILD_JOBS", "4", "--unsetenv", "RUSTFLAGS",
            "--unsetenv", "CARGO_ENCODED_RUSTFLAGS", "--"]
        includes, opaque = list(parent.admission.INCLUDE), list(parent.admission.OPAQUE)
        arguments = [str(runtime.VIRTUAL / ".tools/aeneas/charon/bin/charon"), "cargo",
            "--preset=aeneas", "--error-on-warnings", "--sysroot", "default"]
        for name in includes:
            arguments += ["--include", name]
        for name in opaque:
            arguments += ["--opaque", name]
        arguments += ["--start-from", inv["root"], "--dest-file",
            str(runtime.VIRTUAL / "artifacts/owner-mapping.llbc"), "--", "--manifest-path",
            "verification/experiments/stage1-protocol/inbox-record/Cargo.toml",
            "--lib", "--locked", "--offline", "--target", fixed["target"]]
        command(project, output, "charon", wrapper + arguments)
        ir_path = project / "artifacts/owner-mapping.llbc"
        raw = json.loads(ir_path.read_text())
        options = raw["translated"]["options"]
        if (raw.get("has_errors") is not False or options["start_from"] != [inv["root"]]
                or options["include"] != includes or options["opaque"] != opaque
                or options["exclude"] or options["skip_borrowck"] or options["no_typecheck"]
                or options["monomorphize"] or options["remove_unused_clauses"]
                or {t["key"] for t in raw["translated"]["target_information"]} != {fixed["target"]}):
            raise ValueError("unreviewed capture options or target")
        rec.update(capture_complete=True, llbc_sha256=sha(ir_path),
                   actual_call_binding=admission.admit_ir(raw))
        generated = output / "generated"
        generated.mkdir()
        command(project, output, "aeneas", [str(ROOT / ".tools/aeneas/bin/aeneas"),
            "-backend", "lean", "-abort-on-error", "-warnings-as-errors", "-split-files",
            "-emit-json", "-namespace", "SandproofStage1OwnerMappingRust", "-subdir",
            "SandproofStage1OwnerMapping", "-dest", str(generated), str(ir_path)])
        translated = json.loads((generated / "translation.json").read_text())
        rec["generated_root_admission"] = admission.admit_generated(translated, generated)
        rec["admission_rejection_regressions"] = admission.regression_checks(raw, translated, generated)
        actual_functions = [{k:f[k] for k in ("lean_name", "rust_name", "is_opaque", "lean_file")}
                            for f in translated["functions"]]
        if actual_functions != recipe["generated_functions"]:
            raise ValueError("actual transparent/external function closure differs")
        for name, digest in recipe["generated_raw_files"].items():
            if sha(generated / "SandproofStage1OwnerMapping" / name) != digest:
                raise ValueError("raw unmodified generated file differs: " + name)
        rec["translation_complete"] = True
        (project / GEN).mkdir(parents=True)
        for name in ("Types.lean", "Funs.lean"):
            shutil.copy2(generated / "SandproofStage1OwnerMapping" / name, project / GEN / name)
            expected[str(GEN / name)] = sha(project / GEN / name)
        for name in ("TypesExternal", "FunsExternal", "SharedValueBindings", "OrdDefaultBinding"):
            shutil.copy2(ROOT / HERE / (name + ".lean.in"), project / GEN / (name + ".lean"))
            expected[str(GEN / (name + ".lean"))] = sha(project / GEN / (name + ".lean"))
        templates = inv["common_templates"] + recipe["templates"]
        installed_proofs = []
        for name in templates:
            target = "OwnerMappingCounter" if name.endswith("Counter") else name
            destination = project / PROOFS / (target + ".lean")
            shutil.copy2(ROOT / HERE / (name + ".lean.in"), destination)
            expected[str(destination.relative_to(project))] = sha(destination)
            installed_proofs.append(str(destination.relative_to(project)))
        lake = project / "lakefile.toml"
        lake.write_text(lake.read_text() +
            '\n[[lean_lib]]\nname = "SandproofStage1OwnerMapping"\n'
            'srcDir = "verification/aeneas/stage1-protocol/owner-mapping"\n')
        expected["lakefile.toml"] = sha(lake)
        seeds = installed_proofs + [str(GEN / (name + ".lean")) for name in
            ("Types", "Funs", "TypesExternal", "FunsExternal", "SharedValueBindings", "OrdDefaultBinding")]
        files = sorted(resolver.closure(project, seeds, active_view="lakefile.toml"))
        audits = [a for name in files for a in re.findall(
            r"^#audit_controller_proof (\S+)$", (project / name).read_text(), re.M)]
        if (files != recipe["expected_files"] or audits != recipe["expected_audits"]
                or len(audits) != len(set(audits))):
            raise ValueError("missing, duplicate or changed physical-file/audit inventory")
        rec.update(files={name:sha(project / name) for name in files}, audits=audits)
        command(project, output, "build", ["lake", "build",
            "SandboxModel.Lifecycle.Protocol." + recipe["seed"]])
        physical = output / "actual-files"
        physical.mkdir()
        rec["actual"] = checker.actual_files(project, files, physical, env, command)
        if rec["actual"]["files"] != rec["files"] or rec["actual"]["audits"] != audits:
            raise ValueError("actual complete checker closure differs")
        if any(sha(project / name) != digest for name, digest in expected.items()):
            raise ValueError("private source changed during check")
        rec["private_derived_inputs"] = {name:digest for name,digest in expected.items()
                                       if inputs.get(name) != digest}
        rec["proof_complete"] = True
        rec["property_rejected"] = variant != "candidate"
    except BaseException as error:
        rec.update(error=repr(error), traceback=traceback.format_exc())
    finally:
        try:
            rec["original_inputs_unchanged"] = parent.base.snapshot(ROOT) == inputs
            rec["original_modes_unchanged"] = all(
                ("100755" if (ROOT / name).stat().st_mode & 0o111 else "100644") == mode
                for name, mode in modes.items())
            if not rec["original_inputs_unchanged"] or not rec["original_modes_unchanged"]:
                raise ValueError("original input bytes or modes changed")
        except BaseException as error:
            rec.update(proof_complete=False, property_rejected=False,
                       final_input_error=repr(error))
        rec["generated"] = {str(p.relative_to(output / "generated")):sha(p)
                           for p in (output / "generated").rglob("*") if p.is_file()}
        rec["execution_seconds"] = time.monotonic() - started
        (output / "result.json").write_text(json.dumps(rec, indent=2) + "\n")
    return rec


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--variant", choices=("all", "candidate", "wrong-owner-guard",
        "wrong-world-maximum", "wrong-high-water", "wrong-overflow"), default="all")
    parser.add_argument("--lock", type=Path, required=True, help="existing shared heavy-proof lock")
    args = parser.parse_args()
    os.umask(0o077)
    output = args.output.absolute()
    output.mkdir(parents=True, mode=0o700)
    inv = json.loads((ROOT / HERE / "inventory.json").read_text())
    batch = dict(kind="unadopted-owner-mapping-development-batch", complete=False,
        native_checks_run=False, formal_acceptance=False, shared_acceptance=False, full_v3=False,
        production_adopted=False, public_Ord_binding_adopted=False, variants={})
    waiting = time.monotonic()
    try:
        with args.lock.open("r+") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            batch["lock_wait_seconds"] = time.monotonic() - waiting
            admitted = time.monotonic()
            os.environ.setdefault("SANDPROOF_FORMAL_CACHE", str(output / "runtime-cache"))
            resolver.ROOT, resolver.MODULES = ROOT, ("stage1-protocol",)
            original_runtime = resolver.runtime()
            inputs = parent.base.snapshot(ROOT)
            modes = {name:("100755" if (ROOT / name).stat().st_mode & 0o111 else "100644")
                     for name in inputs}
            variants = list(inv["variants"]) if args.variant == "all" else [args.variant]
            for variant in variants:
                result = run(output / variant, variant, inv, inputs, modes)
                batch["variants"][variant] = dict(proof_complete=result["proof_complete"],
                    property_rejected=result["property_rejected"],
                    result_sha256=sha(output / variant / "result.json"))
                if not result["proof_complete"]:
                    raise ValueError("development view failed: " + variant)
            if resolver.runtime() != original_runtime:
                raise ValueError("actual fixed runtime changed")
            batch.update(complete=True, actual_runtime=original_runtime,
                         execution_seconds=time.monotonic()-admitted)
    except BaseException as error:
        batch.update(error=repr(error), traceback=traceback.format_exc())
    (output / "result.json").write_text(json.dumps(batch, indent=2) + "\n")
    print(json.dumps({k:batch.get(k) for k in ("complete", "lock_wait_seconds", "execution_seconds", "error")}))
    raise SystemExit(not batch["complete"])
