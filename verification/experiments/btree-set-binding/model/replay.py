"""Replay the fixed BTreeSet formal component with original Charon/Aeneas/Lean.

Run the separate ning native component as well. This entry alone is not the
shared V0 gate; its native-case registration and global imports belong to the
integration owner.
"""
import argparse
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import time
import tomllib

ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT))
from tools.check_btree_set_binding import (
    check, sha, mutate, service_records, LLBC, GENERATED, PROOFS, EXPERIMENT,
    SOURCE, MANIFEST, FREEZE, ROOT_NAMES, POSITIVE, NEGATIVE)
from tools.check_foundations_extraction import llbc_digest
from tools.check_loop_callbacks_contract import foreign_surface
from tools.check_first_chain_extraction import check_tools

EXTENSION = ('\n[[lean_lib]]\nname = "SandproofBtreeSetProbe"\n'
             'srcDir = "verification/aeneas/btree-set-binding"\n')


def run(command, project, env, log):
    with log.open("xb") as stream:
        code = subprocess.run([str(a) for a in command], cwd=project, env=env,
                              stdout=stream, stderr=subprocess.STDOUT, timeout=900).returncode
    if code:
        raise ValueError("Formal command failure is not a property rejection: " + str(log))
    return {"command": [str(a) for a in command], "exit_code": code, "log_sha256": sha(log)}


def frozen_head(root, contract):
    for name in contract["inputs"] + [FREEZE]:
        if subprocess.check_output(["git", "show", "HEAD:" + name], cwd=root) != (root / name).read_bytes():
            raise ValueError("Commit complete BTreeSet inputs before replay: " + name)
    return subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()


def prepare(root, archive, destination, tools):
    destination.mkdir()
    with tarfile.open(fileobj=io.BytesIO(archive)) as tree:
        tree.extractall(destination, filter="data")
    (destination / ".tools").symlink_to(tools.resolve(), target_is_directory=True)
    (destination / ".lake").mkdir()
    (destination / ".lake/packages").symlink_to((root / ".lake/packages").resolve(),
                                               target_is_directory=True)
    # Private output view only: never edit any tracked shared Lake configuration.
    parent = (root / "lakefile.toml").read_text()
    registered = [entry for entry in tomllib.loads(parent).get("lean_lib", [])
                  if entry.get("name") == "SandproofBtreeSetProbe"]
    exact = {"name": "SandproofBtreeSetProbe", "srcDir": GENERATED}
    if registered and registered != [exact]:
        raise ValueError("Ambiguous or changed BTreeSet library registration")
    (destination / "lakefile.toml").write_text(parent if registered else parent + EXTENSION)
    return destination


def library_sources(root, tools, contract):
    charon = tools / "aeneas/charon/bin/charon"
    frontend = Path(subprocess.check_output([str(charon), "toolchain-path"], text=True).strip()) / "bin/rustc"
    result = {}
    for role, compiler in [("production", "rustc"), ("extraction", str(frontend))]:
        sysroot = Path(subprocess.check_output([compiler, "--print", "sysroot"], text=True).strip())
        library = sysroot / "lib/rustlib/src/rust/library"
        observed = {name: sha(library / name) for name in contract["identities"][role]["library"]}
        expected = {name: info["sha256"] for name, info in contract["identities"][role]["library"].items()}
        if observed != expected:
            raise ValueError("Fixed actual Std source changed: " + role)
        result[role] = observed
    return result


def generate(project, tools, out, env):
    llbc = project / LLBC
    if llbc.exists():
        llbc.rename(out / "previous.llbc")
    dest = project / GENERATED
    retained = out / "previous-generated"
    retained.mkdir()
    for name in ["translation.json", "SandproofBtreeSetProbe/Types.lean",
                 "SandproofBtreeSetProbe/Funs.lean"]:
        path = dest / name
        saved = retained / name
        saved.parent.mkdir(parents=True, exist_ok=True)
        path.rename(saved)
    charon = [tools / "aeneas/charon/bin/charon", "cargo", "--preset=aeneas",
              "--error-on-warnings", "--sysroot", "default"]
    for name in ROOT_NAMES:
        charon += ["--start-from", "crate::" + name]
    charon += ["--dest-file", "../aeneas/llbc/sandproof_btree_set_binding.llbc",
               "--", "--manifest-path", MANIFEST, "--lib", "--locked", "--offline"]
    one = run(charon, project, env | {"RUSTFLAGS": "--remap-path-prefix=" + str(project) + "=."},
              out / "charon.log")
    if not llbc.is_file() or json.loads(llbc.read_text()).get("has_errors") is not False:
        raise ValueError("Missing fresh successful complete LLBC")
    two = run([tools / "aeneas/bin/aeneas", "-backend", "lean", "-abort-on-error",
               "-warnings-as-errors", "-split-files", "-emit-json",
               "-namespace", "SandproofBtreeSetProbeRust", "-subdir", "SandproofBtreeSetProbe",
               "-dest", GENERATED, LLBC], project, env, out / "aeneas.log")
    templates = out / "unused-templates"
    templates.mkdir()
    for name in ["TypesExternal_Template.lean", "FunsExternal_Template.lean"]:
        path = dest / "SandproofBtreeSetProbe" / name
        if path.exists():
            path.rename(templates / name)
    return [one, two]


def service(project, files, audits, out, env):
    requests = [{"tool": "lean_diagnostic_messages", "arguments": {"file_path": name, "timeout_s": 120},
                 "expected_sha256": sha(project / name)} for name in files]
    data = "".join(json.dumps(r) + "\n" for r in requests) + '{"tool":"exit"}\n'
    (out / "requests.jsonl").write_text(data)
    log = out / "service.log"
    with log.open("xb") as stream:
        code = subprocess.run([str(project / ".tools/lean-mcp/bin/python"), "tools/lean_session.py"],
                              cwd=project, env=env, input=data.encode(), stdout=stream,
                              stderr=subprocess.STDOUT, timeout=1200).returncode
    if code:
        raise ValueError("Original Lean service client failed")
    messages = service_records(log, project, files, audits)
    # Forward exactly the actual checked messages. Do not construct audit successes.
    for message in messages:
        print(message, flush=True)
    return {"actual_files": files, "actual_audit_messages": messages,
            "service_log_sha256": sha(log), "requests_sha256": sha(out / "requests.jsonl")}


def reproduce(root, tools, output):
    root, tools, output = root.resolve(), tools.resolve(), output.resolve()
    if os.environ.get("RUSTFLAGS") or os.environ.get("CARGO_ENCODED_RUSTFLAGS"):
        raise ValueError("Formal replay requires clean Rust compiler flags")
    contract = check(root)
    commit = frozen_head(root, contract)
    check_tools(root, tools)
    if subprocess.check_output(["git", "-C", str(root / ".lake/packages/aeneas"),
                                "status", "--porcelain", "--untracked-files=no"], text=True):
        raise ValueError("Lean backend tracked source changed")
    libraries = library_sources(root, tools, contract)
    output.mkdir(parents=True, exist_ok=False)
    started = time.monotonic()
    archive = subprocess.check_output(["git", "archive", "HEAD"], cwd=root)
    (output / "source.tar").write_bytes(archive)
    env = os.environ | {"SANDPROOF_PROOF_TOOL_ROOT": str(tools),
                       "CARGO_TARGET_DIR": str(output / "target-formal"), "CARGO_INCREMENTAL": "0"}
    receipt = {"kind": "actual-Std-BTreeSet-formal-component", "complete": False,
               "commit": commit, "library_sources": libraries,
               "source_archive_sha256": sha(output / "source.tar"), "variants": {}}
    for variant in ["correct", "invert-flag"]:
        begin = time.monotonic()
        out = output / variant
        out.mkdir()
        project = prepare(root, archive, out / "source", tools)
        path = project / SOURCE
        path.write_text(mutate(path.read_text(), variant))
        expected = contract if variant == "correct" else contract["wrong_variant"]
        if sha(path) != expected["correct_source_sha256" if variant == "correct" else "source_sha256"]:
            raise ValueError("Formal actual Rust differs from the registered correct/wrong source")
        one = {"commands": generate(project, tools, out, env)}
        data = json.loads((project / LLBC).read_text())
        if llbc_digest(data) != expected["llbc_digest"] or foreign_surface(data) != expected["opaque_surface"]:
            raise ValueError("Full original LLBC changed, including body/borrow/Drop/signature fields")
        generated = {str(p.relative_to(project)): sha(p) for p in (project / GENERATED).rglob("*") if p.is_file()}
        if generated != expected["generated_sha256"]:
            raise ValueError("Unedited original generated Lean/JSON changed")
        if variant == "correct":
            audit = PROOFS + "/Audit.lean"
            targets = ["SandboxModel.Foundations.BtreeSetBinding.Audit"]
            files = [GENERATED + "/SandproofBtreeSetProbe/" + n + ".lean"
                     for n in ["TypesExternal", "Types", "FunsExternal", "Funs"]]
            files += [PROOFS + "/" + n + ".lean"
                      for n in ["Contracts", "Theory", "Refinement", "Examples", "Audit"]]
            audits = POSITIVE
        else:
            audit = EXPERIMENT + "/InvertFlag.lean"
            targets = ["SandboxModel.Foundations.BtreeSetBinding.Theory", "SandboxModel.ProofAuditCommand"]
            files, audits = [GENERATED + "/SandproofBtreeSetProbe/Funs.lean", audit], NEGATIVE
        one["commands"].append(run(["lake", "build", *targets], project, env, out / "build.log"))
        one["commands"].append(run(["lake", "env", "lean", audit], project, env, out / "kernel-audit.log"))
        one["service"] = service(project, files, audits, out, env)
        one.update(success=True, seconds=round(time.monotonic() - begin, 3),
                   llbc_digest=llbc_digest(data), generated=generated, actual_llbc_sha256=sha(project / LLBC))
        receipt["variants"][variant] = one
        (out / "result.json").write_text(json.dumps(one, indent=2) + "\n")
    if check(root) != contract or frozen_head(root, contract) != commit:
        raise ValueError("BTreeSet frozen input changed during formal replay")
    receipt.update(complete=True, exit_code=0, seconds=round(time.monotonic() - started, 3))
    (output / "result.json").write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps({"complete": True, "correct_audits": len(POSITIVE),
                      "wrong_audits": len(NEGATIVE), "seconds": receipt["seconds"]}))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--tool-root", type=Path,
                        default=Path(os.environ.get("SANDPROOF_PROOF_TOOL_ROOT", str(ROOT / ".tools"))))
    args = parser.parse_args()
    reproduce(ROOT, args.tool_root, args.output)
