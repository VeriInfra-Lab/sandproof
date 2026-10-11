"""Probe/entry guards only; original recovery is tested in its single module.

The old SQLite fake-journal tests were replaced by the checked dual-authority
observer regressions in test_runtime_create_recovery, not another implementation.
Actual guest policy, immutable binding and physical checks remain in the KVM probe.
"""
import ast
import importlib.util
import os
from pathlib import Path
import subprocess
import sys
from types import SimpleNamespace
import unittest

SOURCE = Path(__file__).resolve().parents[2]/'tests/system/benchmark_proxy_policy.py'
SPEC = importlib.util.spec_from_file_location('benchmark_proxy_policy_component', SOURCE)
probe = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(probe)


class ProbeResultTests(unittest.TestCase):
    def test_only_successful_fixed_results_are_policy_observations(self):
        for output, expected in [('allowed\n', True), ('denied\n', False)]:
            with self.subTest(output=output):
                self.assertIs(probe.probe_result(SimpleNamespace(
                    exit_code=0, stdout=output, stderr='')), expected)

    def test_execution_errors_and_malformed_output_are_not_denial_passes(self):
        for code, stdout, stderr in [(127,'','missing executable'),
                (2,'denied\n','probe-error:socket'), (0,'denied\n','probe-error:poll'),
                (0,'',''), (0,'denied\nallowed\n',''), (0,'probe-error:arguments','')]:
            with self.subTest(code=code, stdout=stdout, stderr=stderr):
                with self.assertRaises(RuntimeError):
                    probe.probe_result(SimpleNamespace(exit_code=code,stdout=stdout,stderr=stderr))

    def test_sole_entry_and_observer_without_legacy_public_or_raw_sql_algorithm(self):
        tree = ast.parse(SOURCE.read_text())
        imports = {alias.name for node in ast.walk(tree)
            if isinstance(node,(ast.Import,ast.ImportFrom)) for alias in node.names}
        self.assertTrue({'RuntimeServices','recover_original_create','find_runtime_create'} <= imports)
        self.assertFalse({'sqlite3','start_server','stop_server','provision_runtime'} & imports)
        names = {node.name for node in ast.walk(tree) if isinstance(node,ast.FunctionDef)}
        self.assertNotIn('journal',names)
        self.assertIn('capture',names)
        self.assertIn('settle',names)

    def test_all_original_network_cases_and_physical_proof_inputs_remain(self):
        source = SOURCE.read_text()
        for case in ("create('public'", "create('closed'", "create('static'", "create('proxy_down'"):
            self.assertIn(case,source)
        for value in ('network_inode','kernel','rootfs','resources','start_time_ticks',
                      'audit_gone','processes_in_uid_range','host_default_unchanged'):
            # PID/start-time rejection itself belongs to the original audit_gone.
            if value == 'start_time_ticks':
                audit = SOURCE.with_name('server_template_isolation.py').read_text()
                self.assertIn(value,audit)
            else:
                self.assertIn(value,source)


class OptimizedInterpreterTests(unittest.TestCase):
    def test_optimized_entry_refuses_before_sdk_or_platform_effects(self):
        for mode in ('-O','-OO','1','2'):
            with self.subTest(mode=mode):
                env={key:value for key,value in os.environ.items()
                     if key not in ('PYTHONOPTIMIZE','PYTHONPATH')}
                env['PYTHONDONTWRITEBYTECODE']='1'
                flags=[mode] if mode.startswith('-') else []
                if not flags:
                    env['PYTHONOPTIMIZE']=mode
                result=subprocess.run([sys.executable,*flags,str(SOURCE),'--help'],
                    env=env,capture_output=True,text=True,timeout=10)
                self.assertNotEqual(result.returncode,0)
                self.assertIn('optimized_python',result.stderr)
                self.assertNotIn('ModuleNotFoundError',result.stderr)


if __name__=='__main__':
    unittest.main()
