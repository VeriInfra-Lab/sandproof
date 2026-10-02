"""Unprivileged interpreter probes only; never import an SDK or start a VM."""

import ast
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

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


def complete_report():
    value = reports.new_report("sp12-abcdefgh")
    value.update(status="pass", kvm="executed", commit="a" * 40,
                 cleanup="confirmed", known_sandboxes=4, known_claims=4, staged_copies="removed")
    value["staged_files"] = {name: "removed" for name in reports.STAGED_FILES}
    value["hashes"] = {name: "b" * 64 for name in reports.HASH_NAMES}
    value["preflight"] = {name: "pass" for name in reports.PREFLIGHT_NAMES}
    value["phases"] = {name: "pass" for name in reports.PHASES}
    value["probes"] = {name: {"status": "pass"} for name in reports.PROBES}
    return value


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
        with tempfile.TemporaryDirectory() as temporary:
            for mode in MODES:
                for path in scripts:
                    with self.subTest(mode=mode, script=path.name):
                        result = invoke(mode, [str(path), "--help"], cwd=temporary)
                        self.assertNotEqual(result.returncode, 0, result.stdout)
                        self.assertIn("optimized_python", result.stderr)
                        self.assertEqual(list(Path(temporary).iterdir()), [])

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
            for name in ("server", "jailer", "firecracker", "kernel", "rootfs", "host-config"):
                arguments.extend(["--" + name, str(root / name)])
            arguments.extend(["--scratch-parent", str(root), "--uid-base", "1000",
                              "--expected-commit", "a" * 40, "--server-sha256", "b" * 64])
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
