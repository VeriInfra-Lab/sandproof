"""Materialize the frozen SQLite candidate without editing the caller's source."""
from pathlib import Path
import argparse
import hashlib
import json
import subprocess

HERE = Path(__file__).resolve().parent


def digest(data):
    return hashlib.sha256(data).hexdigest()


def prepare(source_base, destination):
    source_base = source_base.resolve(strict=True)
    destination = destination.absolute()
    if destination.exists() or destination.is_symlink():
        raise ValueError(f"destination already exists: {destination}")
    profile = json.loads((HERE / "candidate.json").read_text())
    patch = HERE / "candidate.patch"
    if digest(patch.read_bytes()) != profile["patch_sha256"]:
        raise ValueError("candidate patch does not match the frozen profile")
    for name, expected in profile["source_files"].items():
        data = subprocess.check_output(
            ["git", "show", f"{profile['base']}:{name}"], cwd=source_base
        )
        if digest(data) != expected:
            raise ValueError(f"frozen production source mismatch: {name}")
    # Keep a self-contained object store. Repeated shared clones can exceed
    # Git's alternates nesting limit before compilation has even started.
    subprocess.run(
        ["git", "clone", "--quiet", "--no-local", "--no-checkout",
         str(source_base), str(destination)], check=True
    )
    subprocess.run(
        ["git", "-c", "advice.detachedHead=false", "checkout", "--detach", profile["base"]],
        cwd=destination, check=True
    )
    for name, expected in profile["source_files"].items():
        if digest((destination / name).read_bytes()) != expected:
            raise ValueError(f"checkout source mismatch: {name}")
    subprocess.run(["git", "apply", "--check", str(patch)], cwd=destination, check=True)
    subprocess.run(["git", "apply", str(patch)], cwd=destination, check=True)
    tracked = subprocess.check_output(
        ["git", "diff", "--name-only"], cwd=destination, text=True
    ).splitlines()
    untracked = subprocess.check_output(
        ["git", "ls-files", "--others", "--exclude-standard"],
        cwd=destination, text=True
    ).splitlines()
    if sorted(tracked + untracked) != sorted(profile["candidate_files"]):
        raise ValueError("candidate changed files beyond the registered scope")
    for name, expected in profile["candidate_files"].items():
        if digest((destination / name).read_bytes()) != expected:
            raise ValueError(f"candidate source mismatch: {name}")
    evidence = destination / "artifacts" / "sqlite-candidate.json"
    evidence.parent.mkdir(parents=True, exist_ok=True)
    record = {
        "base": profile["base"],
        "patch_sha256": profile["patch_sha256"],
        "candidate_files": profile["candidate_files"],
        "scope": "source materialization only; not a build, test or proof result",
    }
    evidence.write_text(json.dumps(record, indent=2) + "\n")
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-base", type=Path, required=True)
    parser.add_argument("--destination", type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(prepare(args.source_base, args.destination), indent=2))


if __name__ == "__main__":
    main()
