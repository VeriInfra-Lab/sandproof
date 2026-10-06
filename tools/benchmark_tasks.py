#!/usr/bin/env python3
"""Lock and audit the complete pinned benchmark without executing task code."""

import argparse
from collections import Counter
from dataclasses import dataclass
import hashlib
import json
import math
from pathlib import Path, PurePosixPath
import re
import subprocess
import sys
import tomllib


class InvalidInput(ValueError):
    pass


def digest(data):
    return hashlib.sha256(data).hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


@dataclass(frozen=True)
class Pin:
    harbor_commit: str
    registry_sha256: str
    repository: str
    commit: str
    count: int
    names_sha256: str
    version: str = '2.0'
    prefix: str = ''


PIN = Pin(
    "31668af15560fb50a9d0584816d12726036656cf",
    "da1446bce05eabbd72a25eb9eef5a2f5db94645ce88c28e2497581433b3d2e60",
    "https://github.com/laude-institute/terminal-bench-2.git",
    "69671fbaac6d67a7ef0dfec016cc38a64ef7a77c",
    89,
    "733562cbea1dd6f0af4b13e6c1c54702ec3f822037f05b978e56f63a1c6a1a36",
)
PIN21 = Pin(PIN.harbor_commit,
    '7d4d122312b5323907afb42ca92e912a3771ca1e98d042a386069231530add50',
    'https://github.com/harbor-framework/terminal-bench-2-1.git',
    'c5ee500c185224c97cd6caff7866a990a0057f41', 89, PIN.names_sha256, '2.1', 'tasks/')


def pin_for(lock):
    pins = {'terminal-bench@2.0': PIN, 'terminal-bench@2.1': PIN21}
    try:
        return pins[lock['dataset']]
    except (KeyError, TypeError):
        raise InvalidInput('unreviewed dataset version') from None


def source_manifest(source, pin=PIN21):
    """Local Git inventory, NOT a claim to have verified a Harbor Hub archive."""
    names = git(source, 'ls-tree', '--name-only', pin.commit + ':tasks').decode().splitlines()
    names = sorted(name for name in names if name not in ('README.md', 'dataset.toml'))
    tasks = [dict(name=name, path=pin.prefix + name, git_url=pin.repository,
                  git_commit_id=pin.commit) for name in names]
    return (json.dumps([dict(name='terminal-bench', version=pin.version, tasks=tasks)], indent=2) + '\n').encode()
REQUIRED = (
    "instruction.md", "task.toml", "environment/Dockerfile",
    "solution/solve.sh", "tests/test.sh",
)


def git(source, *args):
    return subprocess.check_output(
        ["git", "-c", "core.fsmonitor=false", "-c", "core.hooksPath=/dev/null",
         "-C", str(source), *args], stderr=subprocess.PIPE,
    )


def positive(value, label, integer=False):
    if (isinstance(value, bool) or not isinstance(value, (int, float))
            or not math.isfinite(value) or value <= 0
            or (integer and not isinstance(value, int))):
        raise InvalidInput(f"{label}: expected a positive {'integer' if integer else 'number'}")
    return value


def size_mib(value, label):
    # Pinned task files use Harbor's legacy binary K/M/G units.
    if not isinstance(value, str) or not re.fullmatch(r"[0-9]+[KMG]", value):
        raise InvalidInput(f"{label}: unsupported legacy size {value!r}")
    result = int(value[:-1]) * {"K": 1 / 1024, "M": 1, "G": 1024}[value[-1]]
    if result != int(result):
        raise InvalidInput(f"{label}: size must be an integral MiB")
    return positive(int(result), label, integer=True)


def registry_tasks(raw, pin):
    if digest(raw) != pin.registry_sha256:
        raise InvalidInput("registry bytes differ from the pinned Harbor registry")
    datasets = [d for d in json.loads(raw)
                if d.get("name") == "terminal-bench" and d.get("version") == pin.version]
    if len(datasets) != 1:
        raise InvalidInput('expected exactly one terminal-bench@' + pin.version + ' dataset')
    tasks = datasets[0]["tasks"]
    names = [t["name"] for t in tasks]
    if (len(names) != pin.count or len(set(names)) != pin.count
            or digest(canonical(sorted(names))) != pin.names_sha256):
        raise InvalidInput("task set differs from the complete pinned task set")
    for task in tasks:
        if (not re.fullmatch(r"[a-z0-9][a-z0-9.-]*", task["name"])
                or task["path"] != pin.prefix + task["name"]
                or task["git_url"] != pin.repository
                or task["git_commit_id"] != pin.commit):
            raise InvalidInput(f"unexpected source identity: {task['name']}")
    return sorted(tasks, key=lambda task: task["name"])


def role(path):
    if path.startswith("environment/"):
        return "environment"
    if path.startswith("solution/"):
        return "oracle"
    if path.startswith("tests/"):
        return "verifier"
    return {"instruction.md": "instruction", "task.toml": "metadata"}.get(path, "other")


def clean_source(source, pin):
    if git(source, "rev-parse", "HEAD").decode().strip() != pin.commit:
        raise InvalidInput("source HEAD differs from the pinned task commit")
    if git(source, "status", "--porcelain=v1", "-z", "--untracked-files=all", "--ignored"):
        raise InvalidInput("source checkout must be clean, including untracked/ignored files")


def freeze(registry, source, pin=PIN):
    tasks = registry_tasks(Path(registry).read_bytes(), pin)
    source = Path(source).resolve(strict=True)
    clean_source(source, pin)
    tracked = {}
    for record in git(source, "ls-tree", "-rz", "--full-tree", "HEAD").split(b"\0"):
        if not record:
            continue
        metadata, path = record.split(b"\t", 1)
        mode, kind, oid = metadata.decode().split()
        tracked[path.decode()] = (mode, kind, oid)
    rows = []
    for task in tasks:
        name = task["name"]
        files = []
        contents = {}
        for path, (mode, kind, oid) in sorted(tracked.items()):
            if not path.startswith(task['path'] + "/"):
                continue
            if mode not in ("100644", "100755") or kind != "blob":
                raise InvalidInput(f"unsupported source file type: {path}")
            relative = path[len(task['path']) + 1:]
            parts = PurePosixPath(path).parts
            if any(part in (".", "..") for part in parts):
                raise InvalidInput(f"unsafe source path: {path}")
            location = source
            for part in parts:
                location = location / part
                if location.is_symlink():
                    raise InvalidInput(f"source symlink is not allowed: {path}")
            data = location.read_bytes()
            # Check bytes against Git as well as status: assume-unchanged and
            # skip-worktree must not hide local substitutions or LFS pointers.
            blob = b"blob " + str(len(data)).encode() + b"\0" + data
            if hashlib.sha1(blob).hexdigest() != oid:
                raise InvalidInput(f"working bytes differ from pinned Git blob: {path}")
            executable = bool(location.stat().st_mode & 0o111)
            if executable != (mode == "100755"):
                raise InvalidInput(f"working executable bit differs from Git: {path}")
            files.append({"path": relative, "role": role(relative), "mode": mode,
                          "bytes": len(data), "sha256": digest(data)})
            if relative in REQUIRED:
                contents[relative] = data
        if not set(REQUIRED).issubset(contents):
            raise InvalidInput(f"{name}: missing task/initialization/oracle/verifier input")
        config = tomllib.loads(contents["task.toml"].decode())
        environment = config["environment"]
        modern = pin.version == '2.1'
        if modern:
            if (config.get('schema_version') != '1.1' or config.get('artifacts') != []
                    or config.get('task', {}).get('name') != 'terminal-bench/' + name
                    or set(environment) != {'build_timeout_sec', 'docker_image', 'cpus',
                                           'memory_mb', 'storage_mb', 'gpus', 'allow_internet', 'mcp_servers', 'env'}
                    or environment['gpus'] != 0 or environment['allow_internet'] is not True
                    or environment['mcp_servers'] != [] or environment['env'] != {}
                    or config['agent'].keys() != {'timeout_sec'}
                    or config['verifier'].keys() != {'timeout_sec', 'env'}
                    or config['verifier']['env'] != {} or config['solution'] != {'env': {}}):
                raise InvalidInput(f'{name}: task environment schema requires explicit review')
        elif config["version"] != "1.0" or set(environment) != {
            "build_timeout_sec", "docker_image", "cpus", "memory", "storage",
        }:
            raise InvalidInput(f"{name}: task environment schema requires explicit review")
        requirements = {
            "cpus": positive(environment["cpus"], name + ".cpus", integer=True),
            "memory_mib": (positive(environment['memory_mb'], name + '.memory', integer=True) if modern
                           else size_mib(environment["memory"], name + ".memory")),
            "disk_mib": (positive(environment['storage_mb'], name + '.storage', integer=True) if modern
                         else size_mib(environment["storage"], name + ".storage")),
            "build_timeout_sec": positive(environment["build_timeout_sec"], name + ".build"),
            "agent_timeout_sec": positive(config["agent"]["timeout_sec"], name + ".agent"),
            "verifier_timeout_sec": positive(config["verifier"]["timeout_sec"], name + ".verifier"),
        }
        dockerfile = contents["environment/Dockerfile"].decode()
        row = {
            "name": name, "source_path": task["path"], "requirements": requirements,
            "upstream_image_reference": environment["docker_image"],
            "dockerfile_from_lines": re.findall(r"(?im)^FROM\s+([^\r\n]+)", dockerfile),
            "files_sha256": digest(canonical(files)), "files": files,
        }
        if modern:
            row.update(package_name=config['task']['name'], dataset='terminal-bench@2.1')
        rows.append(row)
    clean_source(source, pin)
    return {
        "schema_version": 1, "dataset": 'terminal-bench@' + pin.version,
        "harbor_commit": pin.harbor_commit, "e2b_sdk_version": "2.51.0",
        "registry_sha256": pin.registry_sha256,
        "source": {"repository": pin.repository, "commit": pin.commit},
        "task_count": pin.count, "task_names_sha256": pin.names_sha256, "tasks": rows,
    }


def verify(lock, registry, source, pin=None):
    pin = pin_for(lock) if pin is None else pin
    actual = freeze(registry, source, pin)
    if lock != actual:
        raise InvalidInput("lock differs from the complete pinned source; regenerate and review")
    return actual


CAPACITY_FIELDS = {"cpus", "memory_mib", "disk_mib", "max_timeout_sec", "max_lifetime_sec"}


def audit(verified_lock, capacity):
    if not isinstance(capacity, dict) or set(capacity) != CAPACITY_FIELDS:
        raise InvalidInput("capacity must contain exactly: " + ", ".join(sorted(CAPACITY_FIELDS)))
    for field, value in capacity.items():
        positive(value, field, integer=True)
    rows = []
    groups = Counter()
    for task in verified_lock["tasks"]:
        req = task["requirements"]
        gaps = [{"resource": field, "required": req[field], "configured": capacity[field]}
                for field in ("cpus", "memory_mib", "disk_mib") if req[field] > capacity[field]]
        # The pinned unmodified Harbor E2B adapter requests 24h. Do not silently
        # shorten this or substitute agent_timeout for the API lifetime request.
        for field in ("max_timeout_sec", "max_lifetime_sec"):
            if capacity[field] < 86400:
                gaps.append({"resource": field, "required": 86400, "configured": capacity[field]})
        groups[(req["cpus"], req["memory_mib"], req["disk_mib"])] += 1
        rows.append({
            "name": task["name"], "files_sha256": task["files_sha256"],
            "requirements": req, "resource_gaps": gaps,
            "configured_resources_fit": not gaps,
            "initialization": "requires_reviewed_fixed_base_recipe",
            "network": "requires_task_review", "oracle": "not_run",
            "verifier": "not_run", "agent_comparison": "not_run",
            "ready_to_run": False,
        })
    return {
        "schema_version": 1, "dataset": verified_lock["dataset"],
        "source": verified_lock["source"], "lock_sha256": digest(canonical(verified_lock)),
        "task_count": len(rows), "source_verified": True,
        "capacity": capacity, "capacity_evidence": "operator_supplied_not_host_observed",
        "harbor_create_timeout_sec": 86400,
        "configured_resources_fit_count": sum(row["configured_resources_fit"] for row in rows),
        "ready_to_run_count": 0, "resource_groups": [
            {"cpus": key[0], "memory_mib": key[1], "disk_mib": key[2], "tasks": count}
            for key, count in sorted(groups.items())
        ], "tasks": rows,
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("freeze", "verify", "audit", "manifest"))
    parser.add_argument('--dataset', choices=('2.0', '2.1'), default='2.0')
    parser.add_argument("--registry", type=Path, required=True)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--lock", type=Path, required=True)
    parser.add_argument("--capacity", type=Path)
    parser.add_argument("--report", type=Path)
    args = parser.parse_args(argv)
    if args.command == "audit" and (args.capacity is None or args.report is None):
        parser.error("audit requires --capacity and --report")
    try:
        if args.command == 'manifest':
            if args.dataset != '2.1':
                raise InvalidInput('legacy registry must be obtained from the pinned Harbor commit')
            raw = source_manifest(args.source)
            registry_tasks(raw, PIN21)
            args.registry.write_bytes(raw)
            return 0
        if args.command == "freeze":
            lock = freeze(args.registry, args.source, PIN21 if args.dataset == '2.1' else PIN)
            args.lock.write_text(json.dumps(lock, indent=2) + "\n")
            print(f"Locked all {lock['task_count']} tasks; no task code was executed.")
            return 0
        lock = verify(json.loads(args.lock.read_text()), args.registry, args.source)
        if args.command == "verify":
            print(f"Verified all {lock['task_count']} tasks and their source files; no task code was executed.")
            return 0
        report = audit(lock, json.loads(args.capacity.read_text()))
        args.report.write_text(json.dumps(report, indent=2) + "\n")
        count = report["task_count"]
        fit = report["configured_resources_fit_count"]
        print(f"Audited all {count} tasks; {fit} fit configured resources; 0 ready to run.")
        return 0 if fit == count else 1
    except (InvalidInput, OSError, ValueError, KeyError, TypeError, subprocess.CalledProcessError) as error:
        print(f"benchmark input rejected: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
