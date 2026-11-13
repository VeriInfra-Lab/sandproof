"""Assemble an unload proof package and its frozen Lean imports."""

import argparse
import hashlib
import json
from pathlib import Path

PACKAGE = Path(__file__).resolve().parent


def checked(root, relative, expected):
    source = (root / relative).resolve(strict=True)
    if not source.is_relative_to(root) or not source.is_file():
        raise ValueError(f"proof input is outside its source root: {relative}")
    data = source.read_bytes()
    if hashlib.sha256(data).hexdigest() != expected:
        raise ValueError(f"proof input changed: {relative}")
    return data


def materialize(output, package=PACKAGE):
    repository = package.parents[3]
    inventory = json.loads((package / "sources.json").read_bytes())
    files = {
        name: checked(repository, name, expected)
        for name, expected in inventory["tool_inputs"].items()
    }
    for module, entry in inventory["modules"].items():
        parts = module.split(".")
        if not all(part.isidentifier() for part in parts):
            raise ValueError(f"invalid Lean module name: {module}")
        root = package if "package" in entry else repository
        name = entry["package"] if root == package else entry["repository"]
        files[Path(*parts).with_suffix(".lean")] = checked(root, name, entry["sha256"])
    files["lakefile.toml"] = (package / "lakefile.toml.in").read_bytes()
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
    parser.add_argument("--package", type=Path, default=PACKAGE, help="unload proof package")
    args = parser.parse_args()
    materialize(args.output, args.package.resolve())
