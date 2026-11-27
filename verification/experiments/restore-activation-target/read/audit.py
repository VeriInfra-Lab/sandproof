"""Check fresh file diagnostics from the standard Lean service."""
import hashlib
import json
from pathlib import Path
import re
import sys


def file_hash(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def requests(project, paths):
    for path in paths:
        print(json.dumps({
            "tool": "lean_diagnostic_messages",
            "arguments": {"file_path": path},
            "expected_sha256": file_hash(project / path),
        }))
    print(json.dumps({"tool": "exit"}))


def results(project, log, paths):
    records = [json.loads(line) for line in log.read_text().splitlines()]
    observed = [entry for entry in records if "request" in entry]
    if [entry["request"]["arguments"]["file_path"] for entry in observed] != paths:
        raise SystemExit("Lean did not diagnose the exact requested files")
    for path, entry in zip(paths, observed, strict=True):
        result = entry["result"].get("result", {})
        items = result.get("items", [])
        if (entry["stale"] or entry["revision"]["file_sha256"] != file_hash(project / path)
                or not result.get("success") or result.get("partial") or result.get("timed_out")
                or any(item["severity"] == "error" for item in items)):
            raise SystemExit(f"Lean diagnostics failed: {path}")
        if Path(path).name.startswith("PermitRead"):
            expected = re.findall(r"^#audit_controller_proof (\S+)", (project / path).read_text(), re.M)
            for name in expected:
                prefix = f"Checked {name}:"
                if not any(item["severity"] == "info" and item["message"].startswith(prefix) for item in items):
                    raise SystemExit(f"missing Lean audit: {name}")
                print(prefix.removesuffix(":"))


if __name__ == "__main__":
    action, folder, *arguments = sys.argv[1:]
    if action == "requests":
        requests(Path(folder), arguments)
    elif action == "results":
        results(Path(folder), Path(arguments[0]), arguments[1:])
    else:
        raise SystemExit(f"unknown diagnostics action: {action}")
