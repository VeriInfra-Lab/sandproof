"""Integrity/admission checks for the conditional B1 contract, not a Rust proof."""

import hashlib
import json
import os
from pathlib import Path
import re
import sys
import tarfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def check(root=ROOT, cargo_home=None):
    env = json.loads((root / "verification/task-contract-environment.json").read_text())
    if env["proof_kind"] != "conditional-interface-model-not-extracted-async-rust":
        raise ValueError("Do not relabel the contract as an extracted async implementation")
    entries = (root / "verification/task-contract-inputs.sha256").read_text().splitlines()
    names = [line.split("  ", 1)[1] for line in entries]
    if len(names) != len(set(names)) or sorted(names) != sorted(env["inputs"]):
        raise ValueError("Missing or duplicate frozen input")
    for line in entries:
        sha, name = line.split("  ", 1)
        path = (root / name).resolve()
        if not path.is_relative_to(root.resolve()) or digest(path) != sha:
            raise ValueError(f"Frozen contract input changed: {name}")
    package = next(
        p for p in tomllib.loads((root / "Cargo.lock").read_text())["package"]
        if p["name"] == "tokio"
    )
    if (package["version"], package["checksum"]) != (
        env["tokio"]["version"], env["tokio"]["checksum"]
    ):
        raise ValueError("Tokio version/checksum differs from the reviewed contract")
    cargo = Path(cargo_home or os.environ.get("CARGO_HOME", Path.home() / ".cargo"))
    matches = list((cargo / "registry/src").glob(f"*/tokio-{package['version']}"))
    if len(matches) != 1:
        raise ValueError("Need exactly one source tree for the frozen Tokio version")
    registry = matches[0]
    archive = cargo / "registry/cache" / registry.parent.name / f"tokio-{package['version']}.crate"
    if digest(archive) != package["checksum"]:
        raise ValueError("Cached Tokio archive differs from Cargo.lock")
    expected_files = set()
    # Registry caches contain a .crate archive, unlike `cargo vendor` output.
    with tarfile.open(archive) as crate:
        for member in crate.getmembers():
            if not member.isfile():
                if not member.isdir():
                    raise ValueError("Unexpected link or device in Tokio archive")
                continue
            relative = Path(member.name).relative_to(f"tokio-{package['version']}")
            path = (registry / relative).resolve()
            expected_files.add(str(relative))
            if not path.is_relative_to(registry.resolve()):
                raise ValueError("Tokio archive path escaped the registry")
            if digest(path) != hashlib.sha256(crate.extractfile(member).read()).hexdigest():
                raise ValueError(f"Cached Tokio source changed: {relative}")
    actual_files = {str(p.relative_to(registry)) for p in registry.rglob("*") if p.is_file()}
    if actual_files - {".cargo-ok"} != expected_files:
        raise ValueError("Cached Tokio source inventory changed")
    for name, sha in env["tokio"]["reviewed_sources"].items():
        if digest(registry / name) != sha:
            raise ValueError(f"Tokio review anchor changed: {name}")
    found = []
    for name in ["Model", "Capacity", "Examples"]:
        text = (root / f"SandboxModel/Boundaries/Task/{name}.lean").read_text()
        if re.search(r"\b(?:sorry|admit|axiom)\b", text):
            raise ValueError("Unapproved proof placeholder or assumption")
        prefix = "SandboxModel.Boundaries.Task." + ("Examples." if name == "Examples" else "")
        found.extend(prefix + n for n in re.findall(r"^theorem (\w+)", text, re.M))
    if sorted(found) != sorted(env["theorems"]):
        raise ValueError("The theorem inventory changed; review its coverage")
    audit = (root / "SandboxModel/Boundaries/Task/Audit.lean").read_text()
    actual = re.findall(r"^#audit_controller_proof (\S+)$", audit, re.M)
    if len(actual) != len(found) or sorted(actual) != sorted(found):
        raise ValueError("Missing, duplicate or extra contract theorem audit")
    return env


if __name__ == "__main__":
    try:
        environment = check()
    except (OSError, ValueError, KeyError, StopIteration) as error:
        sys.exit(str(error))
    print(f"B1 conditional contract: {len(environment['theorems'])} theorem roots; source integrity checked.")
    print("This checks frozen inputs and review records, not async Rust translation correctness.")
