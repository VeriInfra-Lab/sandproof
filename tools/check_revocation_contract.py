"""Admission of the conditional B3 contract; not an async Rust equivalence proof."""

import json
import os
from pathlib import Path
import re
import sys

try:
    from .check_task_contract import ROOT, check as check_b1, digest
except ImportError:
    from check_task_contract import ROOT, check as check_b1, digest


def call_inventory(root):
    # Include tests, rather than guessing whether a nested Rust cfg is active.
    # The environment separately identifies the four production calls.
    result = {}
    for path in sorted(root.glob("*/src/**/*.rs")):
        count = len(re.findall(r"\buntil_revoked\b", path.read_text()))
        if count:
            result[str(path.relative_to(root))] = count
    return result


def check(root=ROOT, cargo_home=None):
    env = json.loads((root / "verification/revocation-contract-environment.json").read_text())
    if env["proof_kind"] != "conditional-interface-model-not-extracted-async-rust":
        raise ValueError("B3 is not an extracted async Rust implementation")
    lines = (root / "verification/revocation-contract-inputs.sha256").read_text().splitlines()
    names = [line.split("  ", 1)[1] for line in lines]
    if len(names) != len(set(names)) or sorted(names) != sorted(env["inputs"]):
        raise ValueError("Missing or duplicate B3 frozen input")
    for line in lines:
        sha, name = line.split("  ", 1)
        path = (root / name).resolve()
        if not path.is_relative_to(root.resolve()) or digest(path) != sha:
            raise ValueError(f"Frozen B3 input changed: {name}")
    # Reuse the complete locked archive/cache and B1 composition checks.
    b1 = check_b1(root, cargo_home)
    if env["tokio"]["version"] != b1["tokio"]["version"]:
        raise ValueError("B1/B3 Tokio versions differ")
    cargo = Path(cargo_home or os.environ.get("CARGO_HOME", Path.home() / ".cargo"))
    registry = next((cargo / "registry/src").glob(f"*/tokio-{env['tokio']['version']}"))
    for name, sha in env["tokio"]["reviewed_sources"].items():
        if digest(registry / name) != sha:
            raise ValueError(f"B3 Tokio review anchor changed: {name}")
    if call_inventory(root) != env["call_inventory"]:
        raise ValueError("B3 call inventory changed; review actual signal/work binding")
    text = (root / "SandboxModel/Boundaries/Revocation/Model.lean").read_text()
    if re.search(r"\b(?:sorry|admit|axiom)\b", text):
        raise ValueError("Unapproved proof placeholder or assumption")
    theorems = ["SandboxModel.Boundaries.Revocation." + name
                for name in re.findall(r"^theorem (\w+)", text, re.M)]
    if sorted(theorems) != sorted(env["theorems"]):
        raise ValueError("B3 theorem inventory changed")
    audit = (root / "SandboxModel/Boundaries/Revocation/Audit.lean").read_text()
    actual = re.findall(r"^#audit_controller_proof (\S+)$", audit, re.M)
    if len(actual) != len(theorems) or sorted(actual) != sorted(theorems):
        raise ValueError("Missing, duplicate or extra B3 theorem audit")
    return env


if __name__ == "__main__":
    try:
        environment = check()
    except (OSError, ValueError, KeyError, StopIteration) as error:
        sys.exit(str(error))
    print(f"B3: {len(environment['theorems'])} conditional model theorems; source/call admission passed.")
    print("Actual async Rust is reviewed/tested, not Aeneas-translated or machine-refined.")
