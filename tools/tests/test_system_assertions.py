"""Unprivileged interpreter probes only; never import an SDK or start a VM."""

import ast
from concurrent.futures import ThreadPoolExecutor, wait
from itertools import islice
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import unittest
from unittest.mock import patch

from tools import system_acceptance as reports


ROOT = Path(__file__).resolve().parents[2]
SYSTEM = ROOT / "tests/system"
GUARD = SYSTEM / "_assertions_required.py"
GUESTS = {"buildkit_export_guest.py", "ext4_population_guest.py", "umoci_unpack_guest.py"}
C02_ACTIONS = (
    "processes_in_uid_range", "durable_rows", "wait_for_destroyed", "assert_no_exact_resources",
    "run_probe", "preflight", "prepare", "kill_one", "cleanup", "unchanged",
    "remove_staged_copies", "run_profile", "exercise",
)
# Only these existing pure helpers/read-only CLI dispatcher can omit a
# function-level guard. New C02 resource entry points are guarded by default.
C02_SAFE_FUNCTIONS = {"destroyed", "entries", "probe_environment", "main"}
MODES = (("-O",), ("-OO",), ("env", "1"), ("env", "2"))
PROBE_WORKERS = 2
COMMON_PREFIX = '''import runpy as _runpy
from pathlib import Path as _GuardPath
_require_assertions = _runpy.run_path(
    str(_GuardPath(__file__).with_name("_assertions_required.py"))
)["require_assertions"]
_require_assertions()
'''
GUEST_PREFIX = '''import sys as _guard_sys
if _guard_sys.flags.optimize:
    raise RuntimeError("optimized_python")
'''


def body_without_docstring(body):
    if (body and isinstance(body[0], ast.Expr) and isinstance(body[0].value, ast.Constant)
            and isinstance(body[0].value.value, str)):
        return body[1:]
    return body


def tree_dump(nodes):
    return [ast.dump(node, include_attributes=False) for node in nodes]


def environment(mode):
    result = {key: value for key, value in os.environ.items()
              if key not in {"PYTHONOPTIMIZE", "PYTHONPATH"}}
    result["PYTHONDONTWRITEBYTECODE"] = "1"
    if mode[0] == "env":
        result["PYTHONOPTIMIZE"] = mode[1]
    return result


def invoke(mode, arguments, *, cwd=ROOT):
    flags = [] if mode[0] == "env" else list(mode)
    return subprocess.run([sys.executable, *flags, *arguments], cwd=cwd,
                          env=environment(mode), capture_output=True, text=True,
                          timeout=10, check=False)


def bounded_results(function, items, workers=PROBE_WORKERS):
    """Keep both running probes and submitted futures bounded, not just threads.

    Yield futures to the test thread so subTest records each error/timeout and
    continues the entire matrix. Joining the executor also drains owned work.
    """
    if type(workers) is not int or workers < 1:
        raise ValueError("positive probe worker count required")
    items = iter(items)
    with ThreadPoolExecutor(max_workers=workers) as executor:
        while batch := tuple(islice(items, workers)):
            pending = [(item, executor.submit(function, item)) for item in batch]
            yield from pending
            # Even a consumer collecting futures without result() cannot grow
            # the queue while the previous bounded batch is still running.
            wait([future for _, future in pending])


def isolated_cli_probe(case):
    mode, path = case
    with tempfile.TemporaryDirectory() as temporary:
        result = invoke(mode, [str(path), "--help"], cwd=temporary)
        return result, [path.name for path in Path(temporary).iterdir()]


def complete_report():
    from tools.tests.test_system_acceptance import complete_report as fixture
    return fixture()


class BoundedProbeTests(unittest.TestCase):
    def test_two_running_and_two_submitted_with_complete_matrix(self):
        barrier = threading.Barrier(PROBE_WORKERS)
        lock = threading.Lock()
        active, high_water = 0, 0
        consumed, finished = [], []

        def items():
            for item in range(8):
                consumed.append(item)
                yield item

        def work(item):
            nonlocal active, high_water
            with lock:
                active += 1
                high_water = max(high_water, active)
            barrier.wait(timeout=5)
            with lock:
                active -= 1
                finished.append(item)
            return item

        results = bounded_results(work, items())
        for expected in range(8):
            item, future = next(results)
            self.assertEqual(item, expected)
            self.assertEqual(consumed, list(range((expected // 2 + 1) * 2)))
            self.assertEqual(future.result(timeout=5), item)
        with self.assertRaises(StopIteration):
            next(results)
        self.assertEqual(sorted(finished), list(range(8)))
        self.assertEqual(high_water, PROBE_WORKERS)
        self.assertEqual(active, 0)

    def test_errors_and_timeouts_are_not_lost_and_remaining_cases_complete(self):
        def work(item):
            if item == 0:
                raise subprocess.TimeoutExpired('probe', 10)
            if item == 1:
                raise RuntimeError('probe failed')
            return item

        results = list(bounded_results(work, range(4)))
        with self.assertRaises(subprocess.TimeoutExpired):
            results[0][1].result()
        with self.assertRaisesRegex(RuntimeError, 'probe failed'):
            results[1][1].result()
        self.assertEqual([future.result() for _, future in results[2:]], [2, 3])

    def test_parallel_probes_use_distinct_workdirs_and_report_residue_before_cleanup(self):
        barrier = threading.Barrier(PROBE_WORKERS)
        directories = []
        lock = threading.Lock()

        def fake_invoke(mode, arguments, *, cwd):
            root = Path(cwd)
            with lock:
                directories.append(root)
            if arguments[0] == 'dirty.py':
                (root / 'unexpected').touch()
            barrier.wait(timeout=5)
            return subprocess.CompletedProcess(arguments, 1, '', 'optimized_python')

        cases = [(MODES[0], Path('dirty.py')), (MODES[1], Path('clean.py'))]
        with patch(__name__ + '.invoke', side_effect=fake_invoke):
            results = [(case, future.result()) for case, future in bounded_results(isolated_cli_probe, cases)]
        self.assertEqual(results[0][1][1], ['unexpected'])
        self.assertEqual(results[1][1][1], [])
        self.assertEqual(len(set(directories)), 2)
        self.assertTrue(all(not root.exists() for root in directories))

    def test_nonpositive_or_noninteger_limits_are_rejected(self):
        for workers in (0, -1, True, 1.5):
            with self.subTest(workers=workers), self.assertRaises(ValueError):
                list(bounded_results(lambda item: item, (), workers=workers))


class SystemAssertionsTests(unittest.TestCase):
    def test_every_system_entry_guards_before_imports_or_resource_actions(self):
        # New scripts cannot silently inherit the optimized-assertion hazard.
        for path in sorted(SYSTEM.glob("*.py")):
            with self.subTest(script=path.name):
                tree = ast.parse(path.read_text())
                body = body_without_docstring(tree.body)
                if path == GUARD:
                    self.assertFalse(any(isinstance(node, ast.Assert) for node in ast.walk(tree)))
                    continue
                if path.name == "sdk_c02_acceptance.py":
                    expected = ast.parse(COMMON_PREFIX).body[:-1]
                    self.assertEqual(tree_dump(body[:len(expected)]), tree_dump(expected))
                    functions = {node.name: node for node in body if isinstance(node, ast.FunctionDef)}
                    expected_call = ast.parse("_require_assertions(Failure)").body[0]
                    self.assertTrue(set(C02_ACTIONS) <= functions.keys())
                    for name, function in functions.items():
                        if name in C02_SAFE_FUNCTIONS:
                            continue
                        first = body_without_docstring(function.body)[0]
                        self.assertEqual(ast.dump(first), ast.dump(expected_call), name)
                else:
                    expected = ast.parse(GUEST_PREFIX if path.name in GUESTS else COMMON_PREFIX).body
                    self.assertEqual(tree_dump(body[:len(expected)]), tree_dump(expected))
                # Cleanup and command termination must remain explicit actions,
                # even if a future edit accidentally weakens an entry guard.
                for node in ast.walk(tree):
                    if isinstance(node, ast.Assert):
                        self.assertFalse(any(isinstance(value, ast.Attribute) and value.attr == "kill"
                                             for value in ast.walk(node.test)),
                                         f"kill inside assert: {path.name}:{node.lineno}")

    def test_optimized_cli_refuses_every_script_before_sdk_or_platform_tools(self):
        scripts = [path for path in sorted(SYSTEM.glob("*.py"))
                   if path.name != "sdk_c02_acceptance.py"]
        cases = [(mode, path) for mode in MODES for path in scripts]
        completed = 0
        for (mode, path), future in bounded_results(isolated_cli_probe, cases):
            completed += 1
            with self.subTest(mode=mode, script=path.name):
                result, entries = future.result()
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertIn("optimized_python", result.stderr)
                self.assertEqual(entries, [])
        self.assertEqual(completed, len(MODES) * len(scripts))
        print(f"assertion CLI probes: {completed}/{len(cases)}, workers={PROBE_WORKERS}", flush=True)

    def test_optimized_importlib_refuses_including_reusable_resource_audits(self):
        scripts = [str(path) for path in sorted(SYSTEM.glob("*.py"))
                   if path.name not in {GUARD.name, "sdk_c02_acceptance.py"}]
        code = f'''
import importlib.util
for path in {scripts!r}:
    spec = importlib.util.spec_from_file_location("isolated_system_probe", path)
    module = importlib.util.module_from_spec(spec)
    try:
        spec.loader.exec_module(module)
    except RuntimeError as error:
        if str(error) != "optimized_python":
            raise
    else:
        raise RuntimeError("resource entry imported under optimization: " + path)
print("all_resource_imports_rejected")
'''
        with tempfile.TemporaryDirectory() as temporary:
            for mode in MODES:
                with self.subTest(mode=mode):
                    result = invoke(mode, ["-c", code], cwd=temporary)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertEqual(result.stdout.strip(), "all_resource_imports_rejected")

    def test_standalone_guest_guard_does_not_require_an_uploaded_sibling(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "renamed-guest.py"
            for name in sorted(GUESTS):
                path.write_bytes((SYSTEM / name).read_bytes())
                for mode in MODES:
                    with self.subTest(mode=mode, guest=name):
                        result = invoke(mode, [str(path)], cwd=temporary)
                        self.assertNotEqual(result.returncode, 0)
                        self.assertIn("optimized_python", result.stderr)
                        self.assertEqual(list(Path(temporary).iterdir()), [path])

    def test_normal_importlib_guard_and_stdlib_resource_helper_are_unchanged(self):
        code = f'''
import importlib.util
for path in {str(GUARD)!r}, {str(SYSTEM / "server_process_crash.py")!r}:
    spec = importlib.util.spec_from_file_location("isolated_normal_probe", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    if hasattr(module, "require_assertions"):
        module.require_assertions()
    else:
        if not callable(module.process_start_ticks):
            raise RuntimeError("normal helper unavailable")
print("normal_imports_preserved")
'''
        with tempfile.TemporaryDirectory() as temporary:
            result = invoke(("env", "0"), ["-c", code], cwd=temporary)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout.strip(), "normal_imports_preserved")

    def test_c02_rejects_direct_execute_and_audit_before_any_action(self):
        code = f'''
import inspect, sys
sys.path.insert(0, {str(SYSTEM)!r})
import sdk_c02_acceptance as runner
for name in {C02_ACTIONS!r}:
    function = getattr(runner, name)
    arguments = {{p.name: None for p in inspect.signature(function).parameters.values()
                 if p.default is inspect.Parameter.empty}}
    try:
        function(**arguments)
    except runner.Failure as error:
        if error.code != "optimized_python":
            raise
    else:
        raise RuntimeError("C02 action entered: " + name)
print("all_c02_actions_rejected")
'''
        for mode in MODES:
            with self.subTest(mode=mode):
                result = invoke(mode, ["-c", code])
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout.strip(), "all_c02_actions_rejected")

    def test_c02_execution_cli_rejects_before_creating_private_evidence_or_resources(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            arguments = [str(SYSTEM / "sdk_c02_acceptance.py")]
            for name in ('center','host','candidate-manifest','redis-material','tls-material',
                         'service-staging-parent','service-cgroup-record','cgroup-parent',
                         "jailer", "firecracker", "kernel", "rootfs", "host-config"):
                arguments.extend(["--" + name, str(root / name)])
            arguments.extend(["--scratch-parent", str(root), "--uid-base", "1000",
                              "--expected-commit", "a" * 40])
            for name in ('center-uid','center-gid','redis-uid','redis-gid'):
                arguments.extend(['--'+name,'1001'])
            for mode in MODES:
                with self.subTest(mode=mode):
                    result = invoke(mode, arguments)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn("optimized_python", result.stderr)
                    self.assertEqual(list(root.iterdir()), [])

    def test_c02_read_only_export_remains_available_with_guard_digest(self):
        source = ast.parse((SYSTEM / "sdk_c02_acceptance.py").read_text())
        files = next(node.value for node in source.body if isinstance(node, ast.Assign)
                     and any(isinstance(target, ast.Name) and target.id == "SOURCE_FILES"
                             for target in node.targets))
        self.assertEqual(ast.literal_eval(files)["assertions_guard"], "tests/system/_assertions_required.py")
        self.assertIn("assertions_guard", reports.HASH_NAMES)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            value = complete_report()
            (root / "run.lock").touch(mode=0o600)
            reports.write_once(root / "intent.json", value)
            reports.write_once(root / "terminal.json", value)
            before = {path.name: path.read_bytes() for path in root.iterdir()}
            for mode in MODES:
                with self.subTest(mode=mode):
                    result = invoke(mode, [str(SYSTEM / "sdk_c02_acceptance.py"), "--export-run", str(root)])
                    self.assertEqual(result.returncode, 0, result.stderr)
                    exported = json.loads(result.stdout)
                    self.assertEqual(exported["status"], "pass")
                    self.assertEqual(exported["hashes"]["assertions_guard"], "b" * 64)
                    self.assertEqual({path.name: path.read_bytes() for path in root.iterdir()}, before)
        legacy = complete_report()
        del legacy["hashes"]["assertions_guard"]
        self.assertEqual(reports.safe_export(legacy)["status"], "fail")


if __name__ == "__main__":
    unittest.main()
