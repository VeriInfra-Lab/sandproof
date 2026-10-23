"""Admit the actual callback helper bodies and the narrow conditional boundary.

This checks frozen source/artifacts, not compiler correctness or whole FnMut
refinement. Actual contracts and real wrong-body refutations are checked by Lean.
"""
import hashlib
import json
from pathlib import Path
import re

try:
    from .check_foundations_extraction import llbc_digest
    from .check_loop_callbacks_contract import canonical, foreign_surface
except ImportError:
    from check_foundations_extraction import llbc_digest
    from check_loop_callbacks_contract import canonical, foreign_surface

ROOT = Path(__file__).resolve().parents[1]
DEST = "verification/aeneas/callback-state"
LLBC = "verification/aeneas/llbc/sandproof_callback_state.llbc"
SOURCE = "verification/extract-callback-state/src/lib.rs"
ENV = "verification/callback-state-environment.json"
FREEZE = "verification/callback-state-inputs.sha256"
CRATE = "sandproof_callback_state_extract"
ROOTS = ("toggle_step", "trace_step")
NAMESPACE = "SandboxModel.Foundations.CallbackState."
LAKE_EXTENSION = '\n[[lean_lib]]\nname = "SandproofCallbackState"\nsrcDir = "verification/aeneas/callback-state"\n'


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def check_extraction(root, llbc, inventory, contract):
    if llbc.get("has_errors") is not False or inventory.get("crate") != CRATE:
        raise ValueError("Unsuccessful original-tool extraction")
    translated = llbc["translated"]
    options = translated["options"]
    if (translated["crate_name"] != CRATE
            or options["start_from"] != ["crate::" + name for name in ROOTS]
            or options["include"] != ["sandbox_controller_core"]
            or options["dest_file"] != contract["charon_dest_file"]
            or options["preset"] != "Aeneas"
            or options["error_on_warnings"] is not True
            or any(options[key] is not False for key in
                   ("skip_borrowck", "no_typecheck", "no_normalize", "no_reorder_decls"))):
        raise ValueError("Changed callback roots or compiler checks")
    if translated["target_information"] != contract["target_information"]:
        raise ValueError("Changed target representation")
    if foreign_surface(llbc) != contract["reviewed_opaque_signatures"]:
        raise ValueError("Unreviewed external signature")
    declarations = {row["def_id"]: row for row in translated["fun_decls"] if row is not None}
    functions = inventory["functions"]
    if (len(functions) != 2
            or {row["rust_name"] for row in functions} != {CRATE + "::" + name for name in ROOTS}):
        raise ValueError("Missing/extra real callback helper")
    for row in functions:
        declaration = declarations.get(row["def_id"], {})
        body = declaration.get("body")
        if row["is_opaque"] or not isinstance(body, dict) or "Structured" not in body:
            raise ValueError("Actual callback business body became opaque")
        if canonical(body) != contract["root_body_digests"][row["rust_name"]]:
            raise ValueError("Changed complete actual callback body")
    if llbc_digest(llbc) != contract["llbc_digest"]:
        raise ValueError("Changed complete LLBC beyond existing short-name normalization")
    generated = {str(p.relative_to(root)): sha(p) for p in (root / DEST).rglob("*") if p.is_file()}
    if generated != contract["generated"]:
        raise ValueError("Changed, missing or extra generated file")


def check(root=ROOT):
    root = root.resolve()
    contract = json.loads((root / ENV).read_text())
    if contract["kind"] != "actual-helper-proof-and-conditional-external-state-binding":
        raise ValueError("Changed scope")
    expected_scope = {
        "helper_bodies": "actual-original-tool-generated",
        "traversal": "A6-normal-Vec-retain-order-once-and-survivors",
        "forwarding": "B4-same-external-state-original-arguments-and-return",
        "whole_fnmut_machine_refinement": False,
        "panic_heap_rollback": False,
        "new_trusted_guarantees": [],
    }
    if contract["scope"] != expected_scope:
        raise ValueError("Changed trusted boundary")
    frozen = {}
    for line in (root / FREEZE).read_text().splitlines():
        if not line or line.startswith("#"):
            continue
        digest, name = line.split(None, 1)
        name = name.strip()
        if (name in frozen or Path(name).is_absolute() or ".." in Path(name).parts
                or not re.fullmatch("[0-9a-f]{64}", digest)):
            raise ValueError("Invalid frozen input identity")
        frozen[name] = digest
    if set(frozen) != set(contract["inputs"]):
        raise ValueError("Missing/extra frozen input")
    for name, expected in frozen.items():
        if sha(root / name) != expected:
            raise ValueError("Changed frozen input: " + name)
    sources = json.loads((root / "verification/experiments/callback-state/source-map.json").read_text())
    for name, expected in sources["production_sources"].items():
        if sha(root / name) != expected:
            raise ValueError("Changed actual source: " + name)
    if sha(root / SOURCE) != contract["helper_source_sha256"]:
        raise ValueError("Changed complete helper/forwarding source")
    expected_files = {root / "SandboxModel/Foundations/CallbackState" / (name + ".lean")
                      for name in contract["model_modules"]}
    actual_files = set((root / "SandboxModel/Foundations/CallbackState").glob("*.lean"))
    if expected_files != actual_files:
        raise ValueError("Missing/extra proof module")
    audit = root / "SandboxModel/Foundations/CallbackState/Audit.lean"
    roots = re.findall(r"^#audit_controller_proof (\S+)$", audit.read_text(), re.M)
    if roots != contract["theorems"] or len(roots) != len(set(roots)):
        raise ValueError("Missing/duplicate/extra audit root")
    for path in expected_files:
        if re.search(r"^\s*(?:axiom|sorry|admit)\b", path.read_text(), re.M):
            raise ValueError("Unchecked proof placeholder")
    check_extraction(root, json.loads((root / LLBC).read_text()),
                     json.loads((root / DEST / "translation.json").read_text()), contract)
    return contract


if __name__ == "__main__":
    contract = check()
    print("Callback source/artifact admission passed; conditional traversal/forwarding only.")
