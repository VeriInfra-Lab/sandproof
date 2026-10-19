"""Prepare the frozen isolated Rust candidate; never patch the caller checkout.

Run on a6000/Linux with --source-base pointing at this repository and a new
--destination. The roots-template is installed only in the private checkout.
This is source preparation, not a translation, test or proof success check.
"""
from pathlib import Path
import argparse
import hashlib
import json
import platform
import shutil
import subprocess


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def relative(name):
    path = Path(name)
    if path.is_absolute() or ".." in path.parts:
        raise ValueError("candidate paths must stay within the private checkout")
    return path


def prepare(source, destination):
    if platform.system() != "Linux" or platform.machine() != "x86_64":
        raise RuntimeError("this frozen experiment targets Linux x86_64 on a6000")
    assets = Path(__file__).resolve().parent
    profile = json.loads((assets / "candidate.json").read_text())
    anchor = profile["base_anchor"]
    if len(anchor) != 40 or any(char not in "0123456789abcdef" for char in anchor):
        raise ValueError("invalid source anchor")
    patch = assets / "candidate.patch"
    if sha256(patch) != profile["candidate_patch_sha256"]:
        raise ValueError("candidate patch changed")
    destination.mkdir(mode=0o700, parents=False, exist_ok=False)
    checkout = destination / "source"
    subprocess.run(["git", "clone", "--shared", "--no-checkout", str(source), str(checkout)], check=True)
    subprocess.run(["git", "-C", str(checkout), "checkout", "--detach", anchor], check=True)
    for name, digest in profile["base_files"].items():
        if sha256(checkout / relative(name)) != digest:
            raise ValueError("base input changed: " + name)
    store = checkout / "sandbox-center-store/src/store.rs"
    if sha256(store) != profile["original_store_sha256"]:
        raise ValueError("the original store does not match the candidate")
    changes = subprocess.check_output(["git", "apply", "--numstat", str(patch)], cwd=checkout, text=True)
    paths = {line.split("\t", 2)[-1] for line in changes.splitlines()}
    if paths != {"sandbox-center-store/src/store.rs", "sandbox-center-store/src/store/storage_io.rs"}:
        raise ValueError("candidate changes more than the declared store/adapter")
    subprocess.run(["git", "apply", "--check", str(patch)], cwd=checkout, check=True)
    subprocess.run(["git", "apply", str(patch)], cwd=checkout, check=True)
    template = assets / "roots-template"
    files = {str(path.relative_to(template)) for path in template.rglob("*") if path.is_file()}
    if files != {"Cargo.toml", "Cargo.lock", "src/lib.rs", "src/bindings.rs"} or any(path.is_symlink() for path in template.rglob("*")):
        raise ValueError("unexpected root template inputs")
    install = relative(profile["roots_template_destination"])
    if install != Path("verification/extract-storage"):
        raise ValueError("unexpected root destination")
    shutil.copytree(template, checkout / install)
    for name, digest in profile["candidate_files"].items():
        if sha256(checkout / relative(name)) != digest:
            raise ValueError("candidate input changed: " + name)
    result = {"base_anchor": anchor, "checkout": str(checkout),
              "patch_sha256": profile["candidate_patch_sha256"],
              "scope": "isolated Rust candidate; no production change or proof claim"}
    (destination / "prepared.json").write_text(json.dumps(result, indent=2) + "\n")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-base", type=Path, required=True)
    parser.add_argument("--destination", type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(prepare(args.source_base.resolve(), args.destination.resolve()), indent=2))


if __name__ == "__main__":
    main()
