from pathlib import Path
import unittest


class FunctionEntryTests(unittest.TestCase):
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
