from pathlib import Path
import unittest


class FunctionEntryTests(unittest.TestCase):
    def test_readiness_does_not_restart_unchanged_checks_but_code_and_metadata_still_do(self):
        root = Path(__file__).resolve().parents[2]
        workflow = (root / '.github/workflows/function-ci.yml').read_text()
        triggers = workflow.split('  pull_request:', 1)[1].split('  push:', 1)[0]
        self.assertIn('types: [opened, synchronize, reopened, edited]', triggers)
        self.assertNotIn('ready_for_review', triggers)
        for role in ('rust', 'sdk', 'harbor', 'title', 'policy'):
            self.assertIn('runs-on: [self-hosted, Linux, X64, sandproof-ci-' + role + ']', workflow)
        self.assertIn('cancel-in-progress: true', workflow)
        self.assertNotIn('pull_request_target:', workflow)

    def test_cold_reducer_build_precedes_component_tests_and_keeps_checks(self):
        root = Path(__file__).resolve().parents[2]
        commands = [line.strip() for line in (root / 'scripts/check-function.sh').read_text().splitlines()
                    if line.strip() and not line.lstrip().startswith('#')]
        build = 'cargo build --locked -p sandproof-controller-service --bin sandproof-window-reducer'
        tests = 'python3 -m unittest discover -s tools/tests -v'
        self.assertIn('set -euo pipefail', commands)
        self.assertLess(commands.index(build), commands.index(tests))
        for check in ('python3 tools/check_docs.py', 'cargo fmt --all -- --check',
                      'cargo clippy --workspace --all-targets --locked -- -D warnings',
                      'cargo test --workspace --all-targets --locked',
                      'cargo test --workspace --doc --locked'):
            self.assertIn(check, commands)


if __name__ == '__main__':
    unittest.main()
