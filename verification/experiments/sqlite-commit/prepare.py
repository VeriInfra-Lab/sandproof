"""Materialize the frozen SQLite candidate without editing the caller's source."""
from pathlib import Path
import argparse
import hashlib
import json
import subprocess

HERE = Path(__file__).resolve().parent


def digest(data):
    return hashlib.sha256(data).hexdigest()


def prepare_adopted(source_base, destination, profile):
    """Copy the actual adopted commit, never replay an old private algorithm."""
    head = subprocess.check_output(
        ["git", "rev-parse", "HEAD"], cwd=source_base, text=True
    ).strip()
    subprocess.run(["git", "merge-base", "--is-ancestor", profile["base"], head],
                   cwd=source_base, check=True)
    for name, expected in profile["source_files"].items():
        source = source_base/name
        if source.is_symlink() or digest(source.read_bytes()) != expected:
            raise ValueError("actual adopted source differs: " + name)
        committed = subprocess.check_output(["git", "show", head+":"+name], cwd=source_base)
        if committed != source.read_bytes():
            raise ValueError("adopted source must be committed before acceptance: " + name)
    subprocess.run(["git", "clone", "--quiet", "--no-local", "--no-checkout",
                    str(source_base), str(destination)], check=True)
    subprocess.run(["git", "-c", "advice.detachedHead=false", "checkout", "--detach", head],
                   cwd=destination, check=True)
    for name, expected in profile["source_files"].items():
        if digest((destination/name).read_bytes()) != expected:
            raise ValueError("adopted checkout source differs: " + name)
    for name, expected in profile["candidate_files"].items():
        if digest((destination/name).read_bytes()) != expected:
            raise ValueError("actual adopted core differs: " + name)
    record = dict(base=head, reviewed_base=profile["base"], production_adopted=True,
        private_business_replacement=False, patch_sha256=profile["patch_sha256"],
        candidate_files=profile["candidate_files"],
        scope="actual committed source materialization only; not a build, test or proof result")
    evidence=destination/"artifacts/sqlite-candidate.json"
    evidence.parent.mkdir(parents=True, exist_ok=True)
    evidence.write_text(json.dumps(record, indent=2)+"\n")
    return record


def prepare(source_base, destination):
    source_base = source_base.resolve(strict=True)
    destination = destination.absolute()
    if destination.exists() or destination.is_symlink():
        raise ValueError(f"destination already exists: {destination}")
    profile = json.loads((HERE / "candidate.json").read_text())
    patch = HERE / "candidate.patch"
    if digest(patch.read_bytes()) != profile["patch_sha256"]:
        raise ValueError("candidate patch does not match the frozen profile")
    if profile.get("production_adopted") is not True:
        raise ValueError("the historical private candidate path is retired")
    return prepare_adopted(source_base, destination, profile)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-base", type=Path, required=True)
    parser.add_argument("--destination", type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(prepare(args.source_base, args.destination), indent=2))


if __name__ == "__main__":
    main()
