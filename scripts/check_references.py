#!/usr/bin/env python3
"""Verify pinned local evidence, source hashes, line ranges and file links."""
import hashlib
import json
from pathlib import Path
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]
manifest = json.loads((ROOT / "references/source-manifest.json").read_text())
for name, repo in manifest["repositories"].items():
    directory = ROOT / repo["local_path"]
    sha = subprocess.check_output(["git", "-C", str(directory), "rev-parse", "HEAD"], text=True).strip()
    assert sha == repo["sha"], (name, sha, repo["sha"])
for item in manifest["evidence"]:
    repo = manifest["repositories"][item["repository"]]
    revision = item.get("revision", repo["sha"])
    if revision == repo["sha"]:
        path = ROOT / repo["local_path"] / item["path"]
        data = path.read_bytes()
    else:
        data = subprocess.check_output([
            "git", "-C", str(ROOT / repo["local_path"]),
            "show", f"{revision}:{item['path']}",
        ])
    assert hashlib.sha256(data).hexdigest() == item["sha256"], item["id"]
    assert 1 <= item["start_line"] <= item["end_line"] <= len(data.decode().splitlines()), item["id"]
    assert f"/blob/{revision}/" in item["url"], item["id"]
docs = list(ROOT.glob("*.md"))
for directory in ("docs", "references", "verification"):
    docs.extend((ROOT / directory).glob("*.md"))
for doc in docs:
    for target in re.findall(r"\]\(([^)]+)\)", doc.read_text()):
        if "://" in target or target.startswith("#"):
            continue
        path = target.split("#", 1)[0]
        assert (doc.parent / path).exists(), (doc.name, target)
print(f"Validated {len(manifest['repositories'])} revisions, {len(manifest['evidence'])} source entries and {len(docs)} documents.")
