from pathlib import Path
import os
import json
import re
import subprocess
import sys
import tempfile
import tomllib
import unittest


class FunctionEntryTests(unittest.TestCase):
    def test_real_optimized_test_profile_retains_debug_assertions_and_overflow_panics(self):
        root = Path(__file__).resolve().parents[2]
        script = (root / 'scripts/check-function.sh').read_text()
        settings = dict(re.findall(r'^export (CARGO_PROFILE_(?:DEV|TEST)_[A-Z_]+)=([^\n]+)$',
                                   script, re.MULTILINE))
        self.assertEqual(len(settings), 8)
        configuration = root / 'scripts/check-profiles.toml'
        profiles = tomllib.loads(configuration.read_text())['profile']
        selected = {'sandbox-controller-core', 'sandproof-controller-service', 'libsqlite3-sys'}
        for profile in ('dev', 'test'):
            self.assertEqual(profiles[profile], {'package': {name: {'opt-level': 1} for name in selected}})
        self.assertIn('export CARGO_INCREMENTAL=0', script)
        for name in sorted(selected) + ['sandproof-unselected-profile-probe']:
            with self.subTest(package=name), tempfile.TemporaryDirectory() as temporary:
                stage = Path(temporary)
                (stage / 'src').mkdir()
                (stage / 'Cargo.toml').write_text(
                    '[package]\nname="' + name + '"\nversion="0.0.0"\n'
                    'edition="2024"\n[workspace]\n')
                (stage / 'Cargo.lock').write_text(
                    'version = 4\n[[package]]\nname = "' + name + '"\nversion = "0.0.0"\n')
                (stage / 'src/lib.rs').write_text('''
#[test]
fn debug_configuration_is_enabled() { assert!(cfg!(debug_assertions)); }
#[test]
#[should_panic]
fn debug_assertion_executes() { debug_assert!(std::hint::black_box(false)); }
#[test]
#[should_panic]
fn integer_overflow_panics() {
    let mut value = std::hint::black_box(u8::MAX);
    value += 1;
    std::hint::black_box(value);
}
''')
                env = os.environ | settings | {'CARGO_TARGET_DIR': str(stage / 'target'),
                                               'CARGO_NET_OFFLINE': 'true', 'CARGO_INCREMENTAL': '0'}
                result = subprocess.run(['cargo', '--config', str(configuration), 'test', '--locked',
                                          '--offline', '--lib', '--message-format=json'], cwd=stage, env=env,
                                         capture_output=True, text=True, timeout=20)
                self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
                artifacts = [json.loads(line) for line in result.stdout.splitlines() if line.startswith('{')]
                tests = [a['profile'] for a in artifacts if a.get('reason') == 'compiler-artifact'
                         and a.get('executable') and a['profile']['test']]
                self.assertEqual(len(tests), 1)
                self.assertEqual(tests[0]['opt_level'], '1' if name in selected else '0')
                self.assertEqual(tests[0]['debuginfo'], 1)
                self.assertIs(tests[0]['debug_assertions'], True)
                self.assertIs(tests[0]['overflow_checks'], True)
                self.assertIn('3 passed; 0 failed', result.stdout)

    def test_readiness_does_not_restart_unchanged_checks_but_code_and_metadata_still_do(self):
        root = Path(__file__).resolve().parents[2]
        workflow = (root / '.github/workflows/function-ci.yml').read_text()
        triggers = workflow.split('  pull_request:', 1)[1].split('  push:', 1)[0]
        self.assertIn('types: [opened, synchronize, reopened, edited]', triggers)
        self.assertNotIn('ready_for_review', triggers)
        for role in ('sdk', 'harbor', 'title', 'policy'):
            self.assertIn('runs-on: [self-hosted, Linux, X64, sandproof-ci-' + role + ']', workflow)
        self.assertNotIn('  rust:', workflow)
        self.assertIn('cancel-in-progress: true', workflow)
        self.assertNotIn('pull_request_target:', workflow)

    def test_rust_runs_on_integrated_push_or_manual_not_pr(self):
        root = Path(__file__).resolve().parents[2]
        workflow = (root / '.github/workflows/rust-ci.yml').read_text()
        self.assertNotIn('pull_request:', workflow)
        self.assertNotIn('pull_request_target:', workflow)
        self.assertIn('branches: [main, test]', workflow)
        self.assertIn('workflow_dispatch:', workflow)
        self.assertIn('bash scripts/check-function.sh', workflow)

    def test_data_checks_use_only_locked_ci_git_adapter_without_changing_tests(self):
        root = Path(__file__).resolve().parents[2]
        workflow = (root / '.github/workflows/function-ci.yml').read_text()
        self.assertIn('CI_GIT_LOCK="$GITHUB_WORKSPACE/tests/compat/uv.lock"', workflow)
        self.assertIn('CI_GIT_LOCK="$GITHUB_WORKSPACE/benchmarks/uv.lock"', workflow)
        self.assertIn('bash scripts/check-compat.sh', workflow)
        self.assertIn('bash scripts/check-bench.sh', workflow)
        self.assertNotIn('continue-on-error:', workflow)

    def test_cold_reducer_build_precedes_component_tests_and_keeps_checks(self):
        root = Path(__file__).resolve().parents[2]
        commands = [line.strip() for line in (root / 'scripts/check-function.sh').read_text().splitlines()
                    if line.strip() and not line.lstrip().startswith('#')]
        cargo = 'cargo --config scripts/check-profiles.toml '
        build = cargo + 'build --locked -p sandproof-controller-service --bin sandproof-window-reducer'
        tests = 'bash scripts/python-entry.sh tools/check_function_tests.py --mode "$check_mode"'
        self.assertIn('set -euo pipefail', commands)
        self.assertLess(commands.index(build), commands.index(tests))
        for check in ('bash scripts/python-entry.sh tools/check_docs.py', cargo + 'fmt --all -- --check',
                      cargo + 'clippy --workspace --all-targets --locked -- -D warnings',
                      cargo + 'test --workspace --all-targets --locked',
                      cargo + 'test --workspace --doc --locked'):
            self.assertIn(check, commands)
        preflight = 'bash scripts/python-entry.sh tools/check_function_tests.py --mode "$check_mode" --preflight-only'
        self.assertLess(commands.index(preflight), commands.index(build))


    def test_cold_harbor_entry_prepares_reducer_and_build_failure_stops_tests(self):
        root = Path(__file__).resolve().parents[2]
        shim = """#!PYTHON
from pathlib import Path
import os, sys
stage = Path(os.environ['ENTRY_STAGE'])
name = Path(sys.argv[0]).name
with (stage / 'trace').open('a') as output:
    output.write(name + '\\n')
if name == 'cargo':
    if sys.argv[1:] != ['build', '--locked', '-p', 'sandproof-controller-service', '--bin', 'sandproof-window-reducer']:
        raise SystemExit(91)
    if os.environ.get('ENTRY_BUILD_FAILURE'):
        raise SystemExit(17)
    (stage / 'reducer').touch()
elif 'unittest' in sys.argv and not (stage / 'reducer').exists():
    # A cold test environment cannot execute window tests without the helper.
    raise SystemExit(23)
""".replace('PYTHON', sys.executable, 1)
        for fail in (False, True):
            with self.subTest(build_failure=fail), tempfile.TemporaryDirectory() as temporary:
                stage = Path(temporary)
                for name in ('cargo', 'uv'):
                    tool = stage / name
                    tool.write_text(shim)
                    tool.chmod(0o700)
                env = os.environ | {'ENTRY_STAGE': temporary,
                                    'PATH': temporary + os.pathsep + os.environ['PATH']}
                env.pop('ENTRY_BUILD_FAILURE', None)
                if fail:
                    env['ENTRY_BUILD_FAILURE'] = '1'
                result = subprocess.run(['bash', str(root / 'scripts/check-bench.sh')],
                    cwd=temporary, env=env, capture_output=True, text=True, timeout=10)
                self.assertEqual(result.returncode, 17 if fail else 0, result.stderr)
                trace = (stage / 'trace').read_text().splitlines()
                self.assertEqual(trace, ['cargo'] if fail else ['cargo', 'uv', 'uv', 'uv'])


if __name__ == '__main__':
    unittest.main()
