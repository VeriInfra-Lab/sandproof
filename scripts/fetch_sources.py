#!/usr/bin/env python3
"""Download only the public source revisions recorded in the manifest."""
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
manifest = json.loads((ROOT / "references/source-manifest.json").read_text())
for name, repo in manifest["repositories"].items():
    destination = ROOT / repo["local_path"]
    if destination.exists():
        sha = subprocess.check_output(["git", "-C", str(destination), "rev-parse", "HEAD"], text=True).strip()
        if sha != repo["sha"]:
            raise SystemExit(f"{name}: existing checkout differs; preserved it. Expected {repo['sha']}")
        print(f"{name}: pinned checkout already present")
        continue
    destination.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(["git", "clone", "--depth", "1", "--no-checkout", repo["url"], str(destination)], check=True)
    subprocess.run(["git", "-C", str(destination), "fetch", "--depth", "1", "origin", repo["sha"]], check=True)
    subprocess.run(["git", "-C", str(destination), "checkout", "--detach", repo["sha"]], check=True)
    print(f"{name}: checked out {repo['sha']}")
