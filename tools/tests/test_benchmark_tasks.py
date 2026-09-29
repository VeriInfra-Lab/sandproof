"""Real Git fixtures test complete-set integrity without executing benchmarks."""

from copy import deepcopy
from dataclasses import replace
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

from tools import benchmark_tasks as benchmark


class BenchmarkTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.source = self.root / "source"
        self.source.mkdir()
        self.git("init", "-q")
        self.git("config", "user.email", "fixture@example.invalid")
        self.git("config", "user.name", "Benchmark fixture")
        for name, cpus, memory in (("first-task", 1, "2G"), ("second-task", 4, "8G")):
            task = self.source / name
            for path in benchmark.REQUIRED:
                target = task / path
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text("# fixture; never execute\n")
            (task / "environment/Dockerfile").write_text("FROM ubuntu:24.04\nRUN false\n")
            # Executing any task initializer would create this external marker.
            (task / "environment/setup.sh").write_text(f"touch {self.root / 'executed'}\n")
            (task / "task.toml").write_text(
                'version = "1.0"\n[environment]\nbuild_timeout_sec = 600.0\n'
                f'cpus = {cpus}\nmemory = "{memory}"\nstorage = "10G"\n'
                'docker_image = "fixture:mutable-tag"\n'
                '[agent]\ntimeout_sec = 900.0\n[verifier]\ntimeout_sec = 900.0\n'
            )
        self.git("add", ".")
        self.git("commit", "-qm", "fixture")
        self.repin()

    def git(self, *args):
        return subprocess.check_output(
            ["git", "-c", "core.hooksPath=/dev/null", "-C", str(self.source), *args],
            stderr=subprocess.PIPE,
        ).decode().strip()

    def repin(self):
        commit = self.git("rev-parse", "HEAD")
        names = ["first-task", "second-task"]
        entries = [{"name": name, "path": name, "git_url": "https://example.invalid/tasks.git",
                    "git_commit_id": commit} for name in names]
        raw = json.dumps([{"name": "terminal-bench", "version": "2.0", "tasks": entries}]).encode()
        self.registry = self.root / "registry.json"
        self.registry.write_bytes(raw)
        self.pin = benchmark.Pin("a" * 40, benchmark.digest(raw), entries[0]["git_url"],
                                 commit, 2, benchmark.digest(benchmark.canonical(names)))

    def freeze(self):
        return benchmark.freeze(self.registry, self.source, self.pin)

    def test_reproducible_complete_lock_and_phase_separation(self):
        first = self.freeze()
        self.assertEqual(first, self.freeze())
        self.assertEqual(first["task_count"], 2)
        self.assertEqual(first["tasks"][1]["requirements"]["memory_mib"], 8192)
        files = {f["path"]: f["role"] for f in first["tasks"][0]["files"]}
        self.assertEqual(files["environment/setup.sh"], "environment")
        self.assertEqual(files["solution/solve.sh"], "oracle")
        self.assertEqual(files["tests/test.sh"], "verifier")
        self.assertFalse((self.root / "executed").exists())

    def test_lock_omission_duplicate_reorder_and_forged_hash_rejected(self):
        lock = self.freeze()
        variants = []
        missing = deepcopy(lock)
        missing["tasks"].pop()
        variants.append(missing)
        duplicate = deepcopy(lock)
        duplicate["tasks"][1] = duplicate["tasks"][0]
        variants.append(duplicate)
        reordered = deepcopy(lock)
        reordered["tasks"].reverse()
        variants.append(reordered)
        forged = deepcopy(lock)
        forged["tasks"][0]["files"][0]["sha256"] = "0" * 64
        variants.append(forged)
        for invalid in variants:
            with self.subTest(invalid=invalid["tasks"]), self.assertRaises(benchmark.InvalidInput):
                benchmark.verify(invalid, self.registry, self.source, self.pin)

    def test_registry_change_and_wrong_revision_rejected(self):
        raw = self.registry.read_bytes()
        self.registry.write_bytes(raw + b"\n")
        with self.assertRaisesRegex(benchmark.InvalidInput, "registry bytes"):
            self.freeze()
        self.registry.write_bytes(raw)
        self.git("commit", "--allow-empty", "-qm", "another revision")
        with self.assertRaisesRegex(benchmark.InvalidInput, "HEAD"):
            self.freeze()

    def test_pinned_registry_still_requires_complete_safe_task_set(self):
        original = json.loads(self.registry.read_bytes())
        for change in ("omitted", "duplicate", "path", "repository"):
            data = deepcopy(original)
            entries = data[0]["tasks"]
            if change == "omitted":
                entries.pop()
            elif change == "duplicate":
                entries[1] = entries[0]
            elif change == "path":
                entries[0]["path"] = "../escape"
            else:
                entries[0]["git_url"] = "https://example.invalid/other.git"
            raw = json.dumps(data).encode()
            pin = replace(self.pin, registry_sha256=benchmark.digest(raw))
            with self.subTest(change=change), self.assertRaises(benchmark.InvalidInput):
                benchmark.registry_tasks(raw, pin)

    def test_safe_dotted_task_name_is_preserved(self):
        data = json.loads(self.registry.read_bytes())
        data[0]["tasks"][1]["name"] = "install-windows-3.11"
        data[0]["tasks"][1]["path"] = "install-windows-3.11"
        raw = json.dumps(data).encode()
        names = sorted(t["name"] for t in data[0]["tasks"])
        pin = replace(self.pin, registry_sha256=benchmark.digest(raw),
                      names_sha256=benchmark.digest(benchmark.canonical(names)))
        self.assertEqual([t["name"] for t in benchmark.registry_tasks(raw, pin)], names)

    def test_untracked_and_ignored_inputs_rejected(self):
        added = self.source / "first-task/environment/extra"
        added.write_text("not part of benchmark")
        with self.assertRaisesRegex(benchmark.InvalidInput, "clean"):
            self.freeze()
        added.unlink()
        (self.source / ".git/info/exclude").write_text("ignored-input\n")
        (self.source / "ignored-input").write_text("hidden")
        with self.assertRaisesRegex(benchmark.InvalidInput, "clean"):
            self.freeze()

    def test_assume_unchanged_cannot_hide_modified_bytes(self):
        path = "first-task/environment/setup.sh"
        self.git("update-index", "--assume-unchanged", path)
        (self.source / path).write_text("modified\n")
        with self.assertRaisesRegex(benchmark.InvalidInput, "Git blob"):
            self.freeze()

    def test_tracked_symlink_rejected(self):
        (self.source / "first-task/environment/escape").symlink_to(self.root)
        self.git("add", ".")
        self.git("commit", "-qm", "symlink")
        self.repin()
        with self.assertRaisesRegex(benchmark.InvalidInput, "source file type"):
            self.freeze()

    def test_all_resource_gaps_reported_without_dropping_any_task(self):
        capacity = {"cpus": 1, "memory_mib": 512, "disk_mib": 1024,
                    "max_timeout_sec": 600, "max_lifetime_sec": 600}
        report = benchmark.audit(self.freeze(), capacity)
        self.assertEqual(report["task_count"], 2)
        self.assertEqual(report["configured_resources_fit_count"], 0)
        self.assertEqual(len(report["tasks"][0]["resource_gaps"]), 4)
        self.assertEqual(len(report["tasks"][1]["resource_gaps"]), 5)
        self.assertTrue(report["source_verified"])

    def test_fitting_resources_are_not_execution_acceptance(self):
        capacity = {"cpus": 4, "memory_mib": 8192, "disk_mib": 10240,
                    "max_timeout_sec": 86400, "max_lifetime_sec": 86400}
        report = benchmark.audit(self.freeze(), capacity)
        self.assertEqual(report["configured_resources_fit_count"], 2)
        self.assertEqual(report["ready_to_run_count"], 0)
        for task in report["tasks"]:
            self.assertEqual(task["oracle"], "not_run")
            self.assertEqual(task["verifier"], "not_run")
            self.assertFalse(task["ready_to_run"])
        capacity["memory_mib"] -= 1
        self.assertEqual(benchmark.audit(self.freeze(), capacity)["configured_resources_fit_count"], 1)
        capacity["max_lifetime_sec"] -= 1
        self.assertEqual(benchmark.audit(self.freeze(), capacity)["configured_resources_fit_count"], 0)

    def test_malformed_capacity_and_size_are_rejected(self):
        valid = {"cpus": 4, "memory_mib": 8192, "disk_mib": 10240,
                 "max_timeout_sec": 86400, "max_lifetime_sec": 86400}
        lock = self.freeze()
        for value in (True, 0, -1, 1.5, float("inf"), "4"):
            with self.subTest(value=value), self.assertRaises(benchmark.InvalidInput):
                benchmark.audit(lock, dict(valid, cpus=value))
        with self.assertRaises(benchmark.InvalidInput):
            benchmark.audit(lock, dict(valid, guess_network=True))
        for size in ("2GB", "-2G", "0M", "1K", 2048):
            with self.subTest(size=size), self.assertRaises(benchmark.InvalidInput):
                benchmark.size_mib(size, "test")


class CommittedLockTests(unittest.TestCase):
    def test_complete_lock_and_per_task_file_fingerprints(self):
        root = Path(__file__).resolve().parents[2]
        lock = json.loads((root / "benchmarks/terminal-bench-2.0.lock.json").read_text())
        names = [task["name"] for task in lock["tasks"]]
        self.assertEqual(len(names), benchmark.PIN.count)
        self.assertEqual(len(set(names)), benchmark.PIN.count)
        self.assertEqual(benchmark.digest(benchmark.canonical(names)), benchmark.PIN.names_sha256)
        self.assertEqual(lock["source"]["commit"], benchmark.PIN.commit)
        self.assertEqual(lock["registry_sha256"], benchmark.PIN.registry_sha256)
        for task in lock["tasks"]:
            with self.subTest(task=task["name"]):
                self.assertEqual(task["files_sha256"], benchmark.digest(benchmark.canonical(task["files"])))
                self.assertTrue(set(benchmark.REQUIRED).issubset(f["path"] for f in task["files"]))


if __name__ == "__main__":
    unittest.main()
