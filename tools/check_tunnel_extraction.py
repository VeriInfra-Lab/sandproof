"""Admission for a source-bound relay policy and conditional I/O contract."""
import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys

try:
    from .check_foundations_extraction import llbc_digest
    from .check_first_chain_extraction import check_tools
    from .check_task_contract import check as check_b1
except ImportError:
    from check_foundations_extraction import llbc_digest
    from check_first_chain_extraction import check_tools
    from check_task_contract import check as check_b1

ROOT = Path(__file__).resolve().parents[1]
ASSETS = "verification/experiments/tunnel-contract"
LIBRARY = "verification/aeneas/tunnel"
AUDIT = LIBRARY + "/SandproofTunnel/Audit.lean"


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def encoded_sha(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def read_json(path):
    def unique(items):
        result = {}
        for key, value in items:
            if key in result:
                raise ValueError("duplicate JSON key: " + key)
            result[key] = value
        return result
    return json.loads(path.read_text(), object_pairs_hook=unique)


def nodes(value, path=()):
    if isinstance(value, dict):
        yield value, path
        for key, child in value.items():
            yield from nodes(child, path + (key,))
    elif isinstance(value, list):
        for index, child in enumerate(value):
            yield from nodes(child, path + (index,))


def surface(data):
    functions = [f for f in data["translated"]["fun_decls"] if f]
    if len({f["def_id"] for f in functions}) != len(functions):
        raise ValueError("duplicate declaration id")
    declarations = [
        dict(def_id=f["def_id"], name=f["item_meta"]["name"], opacity=f["item_meta"]["opacity"],
             body="Structured" if isinstance(f["body"], dict) else f["body"],
             source=f["src"], sha256=encoded_sha(f)) for f in functions
    ]
    drops = [
        dict(function=f["def_id"], path=list(path), unwind="on_unwind" in path, drop=node["Drop"])
        for f in functions for node, path in nodes(f["body"]) if "Drop" in node
    ]
    return declarations, drops


def audit_names(root):
    env = read_json(root / ASSETS / "environment.json")
    expected = set(env["theorems"])
    names = []
    for file in ("Contract.lean", "Results.lean"):
        text = (root / LIBRARY / "SandproofTunnel" / file).read_text()
        if re.search(r"\b(?:sorry|admit|axiom)\b", text):
            raise ValueError("unapproved proof placeholder or assumption")
        names.extend("SandproofTunnelRust.Contract." + n for n in re.findall(r"^theorem (\w+)", text, re.M))
    actual = re.findall(r"^#audit_controller_proof (\S+)$", (root / AUDIT).read_text(), re.M)
    if not expected or len(names) != len(set(names)) or set(names) != expected:
        raise ValueError("theorem inventory changed")
    if len(actual) != len(set(actual)) or set(actual) != expected:
        raise ValueError("missing, duplicate or extra audit")
    extension = '\n\n[[lean_lib]]\nname = "SandproofTunnel"\nsrcDir = "verification/aeneas/tunnel"\n'
    if (root / "lake-tunnel.toml").read_text() != (root / "lake-first-chain.toml").read_text().rstrip() + extension:
        raise ValueError("relay Lake view must preserve every existing library")
    return expected


def check_pair(root, data):
    env = read_json(root / ASSETS / "environment.json")
    profile = read_json(root / ASSETS / "candidate.json")
    if data.get("has_errors") is not False:
        raise ValueError("incomplete or erroneous frontend output")
    translated = data["translated"]
    if translated["options"] != env["core"]["charon_options"]:
        raise ValueError("unreviewed extraction options")
    if {x["key"] for x in translated["target_information"]} != {profile["target"]}:
        raise ValueError("target changed")
    declarations, drops = surface(data)
    if declarations != env["core"]["declarations"] or drops != env["core"]["drop_sites"]:
        raise ValueError("declaration/normal/unwind Drop surface changed")
    if llbc_digest(data) != env["core"]["llbc_digest"]:
        raise ValueError("complete intermediate representation changed")
    owned = [f for f in translated["fun_decls"] if f and
             f["item_meta"]["name"][0].get("Ident", [None])[0] == "sandproof_data_transport"]
    if not owned or any(not isinstance(f["body"], dict) for f in owned):
        raise ValueError("owned body hidden")
    inventory = read_json(root / LIBRARY / "translation.json")
    if inventory != env["translation"] or any(f["is_opaque"] for f in inventory["functions"]):
        raise ValueError("generated inventory or owned opacity changed")
    for name, expected in profile["source_files"].items():
        if sha(root / name) != expected:
            raise ValueError("production source changed: " + name)
    if sha(root / ASSETS / "candidate.patch") != profile["patch_sha256"]:
        raise ValueError("candidate patch changed")
    return audit_names(root)


def check_inputs(root, rows, expected_names):
    names = [name for _, name in rows]
    if len(names) != len(set(names)) or sorted(names) != sorted(expected_names):
        raise ValueError("missing/duplicate/extra frozen relay input")
    for expected, name in rows:
        path = (root / name).resolve()
        if not path.is_relative_to(root.resolve()) or sha(path) != expected:
            raise ValueError("frozen relay input changed: " + name)


def check_libraries(root, tool_root):
    env = read_json(root / ASSETS / "environment.json")
    if env["proof_kind"] != "extracted-sync-policy-plus-conditional-io-contract":
        raise ValueError("do not relabel the async adapter as machine-refined Rust")
    check_inputs(root, [line.split("  ", 1) for line in
                        (root / "verification/tunnel-inputs.sha256").read_text().splitlines()], env["inputs"])
    check_tools(root, tool_root)
    if subprocess.check_output(["git", "-C", str(root / ".lake/packages/aeneas"),
                                "status", "--porcelain", "--untracked-files=no"], text=True):
        raise ValueError("Lean backend tracked sources changed")
    frozen = read_json(root / env["tool_profile"])
    if env["tools"] != {key: frozen[key] for key in env["tools"]}:
        raise ValueError("tool profile changed")
    # Includes the full locked .crate and every extracted Tokio file.
    check_b1(root)
    package = env["tokio"]
    matches = list((Path(os.environ["CARGO_HOME"]) / "registry/src").glob("*/tokio-" + package["version"]))
    if len(matches) != 1:
        raise ValueError("need the exact locked Tokio source tree")
    for name, expected in package["reviewed_sources"].items():
        if sha(matches[0] / name) != expected:
            raise ValueError("reviewed I/O source changed: " + name)
    frontend = subprocess.check_output([str(tool_root / "aeneas/charon/bin/charon"), "toolchain-path"], text=True).strip()
    production = subprocess.check_output(["rustc", "--print", "sysroot"], text=True).strip()
    for role, sysroot in (("production", production), ("extraction", frontend)):
        for name, expected in env[role]["sources"].items():
            if sha(Path(sysroot) / "lib/rustlib/src/rust/library" / name) != expected:
                raise ValueError("standard-library source changed: " + role + "/" + name)


def sources_digest(root):
    result = hashlib.sha256()
    paths = sorted((root / "SandboxModel").rglob("*.lean"))
    paths += sorted((root / "verification/aeneas").rglob("*.lean"))
    for path in paths:
        name = str(path.relative_to(root)).encode()
        result.update(len(name).to_bytes(8, "big"))
        result.update(name)
        result.update(hashlib.sha256(path.read_bytes()).digest())
    return result.hexdigest()


def check_record(root, row):
    request = row["request"]
    if request != dict(tool="lean_diagnostic_messages", arguments={"file_path": AUDIT},
                       expected_sha256=sha(root / AUDIT)):
        raise ValueError("unfiltered exact physical Audit request required")
    env = read_json(root / ASSETS / "environment.json")
    revision = row["revision"]
    if any(revision.get(k) != v for k, v in env["service_versions"].items()):
        raise ValueError("service/client/toolchain changed")
    if (revision.get("file_sha256") != sha(root / AUDIT)
            or revision.get("sources_sha256") != sources_digest(root)
            or revision.get("lakefile_sha256") != sha(root / "lakefile.toml")
            or revision.get("manifest_sha256") != sha(root / "lake-manifest.json")):
        raise ValueError("file/project/dependency fingerprint changed")
    envelope = row["result"]
    if set(envelope) != {"result"}:
        raise ValueError("unexpected service envelope")
    result = envelope["result"]
    if (row.get("stale") is not False or result.get("success") is not True
            or result.get("partial") is not False or result.get("timed_out") is not False
            or result.get("failed_dependencies") not in (None, [])
            or result.get("still_elaborating_lines") not in (None, [])):
        raise ValueError("incomplete/stale/failed service result")
    names = []
    for item in result["items"]:
        message = str(item.get("message", item.get("data", "")))
        match = re.fullmatch(r"Checked ([\w.]+): \d+ standard logical dependencies", message)
        if item["severity"] in ("error", "warning") or not match:
            raise ValueError("unapproved diagnostic")
        names.append(match.group(1))
    if len(names) != len(set(names)) or set(names) != audit_names(root):
        raise ValueError("missing/duplicate/extra physical theorem audit")


def check_service(root, log):
    records = [json.loads(line) for line in log.read_text().splitlines() if line.startswith("{")]
    records = [row for row in records if row.get("request", {}).get("tool") == "lean_diagnostic_messages"
               and row["request"].get("arguments", {}).get("file_path") == AUDIT]
    if len(records) != 1:
        raise ValueError("one actual physical relay audit required")
    check_record(root, records[0])
    return records[0]


def rejects(action, name):
    try:
        action()
    except (ValueError, KeyError, TypeError, IndexError):
        return
    raise ValueError("admission accepted bad variant: " + name)


def mutations(root, data, row=None):
    rows = [line.split("  ", 1) for line in
            (root / "verification/tunnel-inputs.sha256").read_text().splitlines()]
    expected = read_json(root / ASSETS / "environment.json")["inputs"]
    rejects(lambda: check_inputs(root, rows[1:], expected), "missing frozen input")
    rejects(lambda: check_inputs(root, rows + [rows[0]], expected), "duplicate frozen input")
    bad_hash = copy.deepcopy(rows)
    bad_hash[0][0] = "0" * 64
    rejects(lambda: check_inputs(root, bad_hash, expected), "changed frozen bytes")
    broken = copy.deepcopy(data)
    broken["has_errors"] = True
    rejects(lambda: check_pair(root, broken), "partial frontend")
    for name in ("missing root", "hidden body", "changed Drop"):
        broken = copy.deepcopy(data)
        owned = next(f for f in broken["translated"]["fun_decls"] if f and
                     f["item_meta"]["name"][0].get("Ident", [None])[0] == "sandproof_data_transport")
        if name == "missing root":
            broken["translated"]["fun_decls"][owned["def_id"]] = None
        elif name == "hidden body":
            owned["body"] = "Opaque"
        else:
            drop = next(node for f in broken["translated"]["fun_decls"] if f
                        for node, _ in nodes(f["body"]) if "Drop" in node)
            drop["Drop"] = {"unreviewed": True}
        rejects(lambda: check_pair(root, broken), name)
    if row is not None:
        for key, value in (("stale", True), ("partial", True), ("success", False), ("timed_out", True)):
            broken = copy.deepcopy(row)
            if key == "stale":
                broken[key] = value
            else:
                broken["result"]["result"][key] = value
            rejects(lambda: check_record(root, broken), key)
        broken = copy.deepcopy(row)
        broken["result"]["result"]["items"].pop()
        rejects(lambda: check_record(root, broken), "missing theorem")
        broken = copy.deepcopy(row)
        broken["revision"]["sources_sha256"] = "0" * 64
        rejects(lambda: check_record(root, broken), "wrong source fingerprint")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--llbc", type=Path, required=True)
    parser.add_argument("--tool-root", type=Path, required=True)
    parser.add_argument("--service-log", type=Path)
    parser.add_argument("--check-mutations", action="store_true")
    args = parser.parse_args()
    check_libraries(ROOT, args.tool_root)
    data = read_json(args.llbc)
    check_pair(ROOT, data)
    row = check_service(ROOT, args.service_log) if args.service_log else None
    if args.check_mutations:
        mutations(ROOT, data, row)
    print("Relay: complete source/AST/Drop and actual Audit admission passed.")
    print("A6/B3/B4 and normal plain-value conditions remain explicit.")


if __name__ == "__main__":
    main()
