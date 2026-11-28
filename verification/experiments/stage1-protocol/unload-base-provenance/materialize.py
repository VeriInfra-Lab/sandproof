#!/usr/bin/env python3
"""Materialize the frozen first-unload Lean source closure or an audited object overlay."""

import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import shutil

HERE = Path(__file__).resolve().parent
MODULE = re.compile(r"[A-Za-z_][\w']*(?:\.[A-Za-z_][\w']*)*\Z")
ATTRIBUTE = re.compile(rb"@\[discriminant isize(?:, rust_type[^\]]+)?\]\n\Z")
BASE = {"Aeneas", "Init", "Lean", "Std", "Mathlib", "Batteries"}


def read_json(name):
    return json.loads((HERE / name).read_bytes())


def safe_path(root, relative):
    path = PurePosixPath(relative)
    if path.is_absolute() or not path.parts or ".." in path.parts:
        raise ValueError(f"unsafe source path {relative}")
    resolved = (root / path).resolve(strict=True)
    if not resolved.is_relative_to(root) or not resolved.is_file():
        raise ValueError(f"source outside fixed root: {relative}")
    return resolved


def digest(data):
    return hashlib.sha256(data).hexdigest()


def checked(root, relative, sha256):
    data = safe_path(root, relative).read_bytes()
    if digest(data) != sha256:
        raise ValueError(f"input hash differs: {relative}")
    return data


def module_path(name, extension):
    if not MODULE.fullmatch(name):
        raise ValueError(f"invalid module name: {name}")
    return name.replace(".", "/") + extension


def apply_isolation(data, view):
    edits = []
    previous_end = 0
    for item in view["edits"]:
        offset = item["offset"]
        before, after = item["before"].encode(), item["after"].encode()
        expected = before.replace(b"discriminant isize, ", b"", 1) if b", rust_type" in before else b""
        if (offset < previous_end or not ATTRIBUTE.fullmatch(before) or expected != after
                or data[offset:offset + len(before)] != before
                or not data[offset + len(before):].startswith(b"inductive ")):
            raise ValueError("metadata edit does not match its exact original attribute")
        edits.append((offset, before, after))
        previous_end = offset + len(before)
    output = data
    for offset, before, after in reversed(edits):
        output = output[:offset] + after + output[offset + len(before):]
    restored = output
    for offset, before, after in edits:
        if restored[offset:offset + len(after)] != after:
            raise ValueError("metadata inverse cannot recover its original attribute")
        restored = restored[:offset] + before + restored[offset + len(after):]
    if restored != data or digest(output) != view["isolated_sha256"]:
        raise ValueError("metadata isolation failed source identity or inverse check")
    return output


def imports(data):
    return tuple(
        item.decode()
        for statement in re.findall(rb"(?m)^(?:public |private )?import[ \t]+([^\n]+)", data)
        for item in statement.split(b"--", 1)[0].split()
        if item.decode().split(".", 1)[0] not in BASE
    )


def materialize(repository, output, object_root=None):
    repository = repository.resolve(strict=True)
    sources = read_json("sources.json")
    if sources["schema"] != 1:
        raise ValueError("unsupported first-unload source list")
    isolation = read_json("isolation.json")
    if not isolation.keys() <= sources["modules"].keys():
        raise ValueError("unknown isolation module")
    for path, sha256 in read_json("package-inputs.json").items():
        checked(HERE, path, sha256)
    files = {name: checked(repository, name, sha256)
             for name, sha256 in sources["tool_inputs"].items()}
    modules = {}
    for name, source in sources["modules"].items():
        key, root = ("package", HERE) if "package" in source else ("repository", repository)
        path = module_path(name, ".lean")
        data = checked(root, source[key], source["sha256"])
        modules[name] = apply_isolation(data, isolation[name]) if name in isolation else data
        files[path] = modules[name]

    ordered = []
    seen = set()
    active = set()

    def visit(name):
        if name in seen:
            return
        if name in active or name not in modules:
            raise ValueError(f"invalid frozen import graph at {name}")
        active.add(name)
        for dependency in imports(modules[name]):
            visit(dependency)
        active.remove(name)
        seen.add(name)
        ordered.append(name)

    visit(sources["root"])
    if seen != modules.keys():
        raise ValueError("frozen import graph contains unreachable files")

    reuse = {}
    if object_root is not None:
        # Reuse only object names from exact original receipts. The caller
        # stages the real files under canonical module paths.
        object_root = object_root.resolve(strict=True)
        for name, record in read_json("objects.json").items():
            if name == sources["root"] or name not in modules:
                continue
            path = module_path(name, ".olean")
            if not (object_root / path).exists():
                continue
            if digest(modules[name]) != record["materialized_sha256"]:
                raise ValueError(f"object source does not match current materialized view: {name}")
            checked(object_root, path, record["sha256"])
            reuse[name] = (safe_path(object_root, path), record["sha256"])

    # Use the original #771 Lake package resolution and ordinary Lean
    # kernel commands. Plan order is generated from the true import graph.
    files["lakefile.toml"] = checked(repository, sources["lakefile"]["path"], sources["lakefile"]["sha256"])
    output.mkdir(mode=0o700)
    for relative, data in files.items():
        path = output / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
    for name, (source, sha256) in reuse.items():
        path = output / "objects" / module_path(name, ".olean")
        path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, path)
        if digest(path.read_bytes()) != sha256:
            raise ValueError(f"retained object changed during copy: {name}")
    plan = {"root": sources["root"], "build_order": ordered, "retained": {name: row[1] for name, row in reuse.items()}}
    (output / "plan.json").write_text(json.dumps(plan, indent=2) + "\n")
    return plan


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", type=Path, required=True, help="fixed Sandproof source checkout")
    parser.add_argument("--output", type=Path, required=True, help="new private materialized project")
    parser.add_argument("--object-root", type=Path, help="canonical directory containing audited original objects")
    args = parser.parse_args()
    plan = materialize(args.repository, args.output, args.object_root)
    print(f"{len(plan['build_order'])} modules, {len(plan['retained'])} retained original objects")
