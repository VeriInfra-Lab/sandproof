"""Assemble the frozen continued-unload proof and its original #770 imports."""

import argparse
import hashlib
import json
from pathlib import Path

PACKAGE = Path(__file__).resolve().parent
REPOSITORY = PACKAGE.parents[3]


def checked(root, relative, expected):
    source = (root / relative).resolve(strict=True)
    if not source.is_relative_to(root) or not source.is_file():
        raise ValueError(f"proof input is outside its source root: {relative}")
    data = source.read_bytes()
    if hashlib.sha256(data).hexdigest() != expected:
        raise ValueError(f"proof input changed: {relative}")
    return data


def materialize(output):
    inventory = json.loads((PACKAGE / "sources.json").read_bytes())
    files = {
        name: checked(REPOSITORY, name, expected)
        for name, expected in inventory["tool_inputs"].items()
    }
    for module, entry in inventory["modules"].items():
        parts = module.split(".")
        if not all(part.isidentifier() for part in parts):
            raise ValueError(f"invalid Lean module name: {module}")
        root = PACKAGE if "package" in entry else REPOSITORY
        name = entry["package"] if root == PACKAGE else entry["repository"]
        files[Path(*parts).with_suffix(".lean")] = checked(root, name, entry["sha256"])
    files["lakefile.toml"] = (PACKAGE / "lakefile.toml.in").read_bytes()
    files["build-order.txt"] = "".join(
        module.replace(".", "/") + "\n" for module in inventory["modules"]
    ).encode()

    output.mkdir(mode=0o700)
    for relative, data in files.items():
        target = output / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
    print(f"Materialized {len(inventory['modules'])} modules for {inventory['root']}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, help="new directory for the isolated Lean project")
    materialize(parser.parse_args().output)
