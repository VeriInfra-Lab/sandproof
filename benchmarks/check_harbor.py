"""Check the joint Harbor/SDK installation and optionally all pinned task inputs."""

import argparse
from importlib.metadata import distribution
import json
from pathlib import Path
import platform
import subprocess
import sys

from tools.benchmark_tasks import InvalidInput, PIN, verify


RUNTIMES = {
    "harbor": ("0.23.0", "https://github.com/laude-institute/harbor.git", PIN.harbor_commit, None),
    "e2b": ("2.51.0", "https://github.com/e2b-dev/E2B.git",
            "ccaf9fc0ffe6ac39c7ec786af7608ab1de19467b", "packages/python-sdk"),
}


def check_provenance(name, version, direct_url):
    expected_version, url, commit, subdirectory = RUNTIMES[name]
    if not isinstance(direct_url, dict) or not isinstance(direct_url.get("vcs_info", {}), dict):
        raise InvalidInput(f"{name}: malformed Git installation provenance")
    vcs = direct_url.get("vcs_info", {})
    if (version != expected_version or direct_url.get("url") != url
            or vcs.get("vcs") != "git" or vcs.get("commit_id") != commit
            or direct_url.get("subdirectory") != subdirectory):
        raise InvalidInput(f"{name}: installation differs from the pinned Git source")
    return {"version": version, "repository": url, "commit": commit}


def check_runtime():
    runtimes = {}
    for name in RUNTIMES:
        installed = distribution(name)
        provenance = installed.read_text("direct_url.json")
        if provenance is None:
            raise InvalidInput(f"{name}: missing Git installation provenance")
        runtimes[name] = check_provenance(name, installed.version, json.loads(provenance))
    # Import the actual pinned adapter, including its optional SDK imports.
    # No environment instance is created and no API key is needed.
    from harbor.environments import e2b
    if not e2b._HAS_E2B:
        raise InvalidInput("pinned Harbor cannot import its E2B SDK dependencies")
    return runtimes


def check_tasks(lock, source):
    from harbor.models.task.task import Task

    rows = []
    for entry in lock["tasks"]:
        task = Task(Path(source) / entry["source_path"])
        config = task.config
        actual = {
            "cpus": config.environment.cpus,
            "memory_mib": config.environment.memory_mb,
            "disk_mib": config.environment.storage_mb,
            "build_timeout_sec": config.environment.build_timeout_sec,
            "agent_timeout_sec": config.agent.timeout_sec,
            "verifier_timeout_sec": config.verifier.timeout_sec,
        }
        if task.name != entry["name"] or actual != entry["requirements"]:
            raise InvalidInput(f"{entry['name']}: original Harbor parser differs from the task lock")
        rows.append({"name": task.name, "harbor_inputs_valid": True,
                     "requirements": actual, "execution": "not_run"})
    return rows


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--registry", type=Path)
    parser.add_argument("--source", type=Path)
    parser.add_argument("--lock", type=Path)
    parser.add_argument("--report", type=Path)
    args = parser.parse_args(argv)
    supplied = (args.registry, args.source, args.lock, args.report)
    if any(value is not None for value in supplied) and not all(value is not None for value in supplied):
        parser.error("complete task validation requires --registry, --source, --lock and --report together")
    try:
        runtimes = check_runtime()
        if args.source is None:
            print("Pinned Harbor/E2B Git installations and adapter imports passed; no tasks executed.")
            return 0
        lock = verify(json.loads(args.lock.read_text()), args.registry, args.source)
        rows = check_tasks(lock, args.source)
        args.report.write_text(json.dumps({
            "schema_version": 1, "python": platform.python_version(), "runtimes": runtimes,
            "source": lock["source"], "task_count": len(rows), "tasks": rows,
            "executed_task_count": 0, "scope": "original Harbor input parsing, not environment acceptance",
        }, indent=2) + "\n")
        print(f"Original pinned Harbor loaded all {len(rows)} tasks; 0 tasks executed.")
        return 0
    except (InvalidInput, OSError, ValueError, KeyError, TypeError, ImportError,
            subprocess.CalledProcessError) as error:
        print(f"Harbor input check rejected: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
