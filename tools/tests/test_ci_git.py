import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from tools import ci_git


class CiGitTests(unittest.TestCase):
    def test_existing_locks_are_the_only_source_of_revisions(self):
        root = Path(__file__).resolve().parents[2]
        sdk = ci_git.locked_refs(root / 'tests/compat/uv.lock')
        bench = ci_git.locked_refs(root / 'benchmarks/uv.lock')
        self.assertEqual(len(sdk), 1)
        self.assertEqual(len(bench), 2)
        self.assertTrue(sdk <= bench)

    def test_only_exact_uv_fetch_gains_depth(self):
        ref = ('https://github.com/e2b-dev/E2B.git', '+' + 'a' * 40 + ':refs/commit/' + 'a' * 40)
        args = ['-c', 'gc.auto=0', 'fetch', '--force', '--update-head-ok', *ref]
        expected = args[:3] + ['--depth=1'] + args[3:] + ['+' + 'a' * 40 + ':refs/heads/sandproof-ci-' + 'a' * 40]
        self.assertEqual(ci_git.git_arguments(args, {ref}), expected)
        for other in ([*args, '--tags'], ['fetch', *ref], ['clone', *ref],
                      args[:-1] + ['main'], args[:-2] + ['https://example.com/repo.git', ref[1]],
                      ['-C', 'fetch', 'status'], args[:3] + ['--depth=2'] + args[3:]):
            self.assertEqual(ci_git.git_arguments(other, {ref}), other)

    def test_non_git_floating_wrong_repo_or_mismatched_lock_refuses(self):
        with tempfile.TemporaryDirectory() as temp:
            lock = Path(temp) / 'uv.lock'
            for source in ('https://example.com/E2B.git?rev=' + 'a' * 40 + '#' + 'a' * 40,
                           ci_git.SOURCES['e2b'] + '?rev=main#' + 'a' * 40,
                           ci_git.SOURCES['e2b'] + '?rev=' + 'a' * 40 + '#' + 'b' * 40,
                           ci_git.SOURCES['e2b'] + '?rev=main#main'):
                lock.write_text('[[package]]\nname="e2b"\nsource={git=' + json.dumps(source) + '}\n')
                with self.assertRaises(ValueError):
                    ci_git.locked_refs(lock)
            lock.write_text('[[package]]\nname="e2b"\nsource={registry="pypi"}\n')
            with self.assertRaises(ValueError):
                ci_git.locked_refs(lock)

    def test_actual_shallow_fetch_keeps_exact_commit_tree_without_parent_history(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            origin, target = root / 'origin', root / 'target'
            subprocess.run(['git', 'init', '-q', str(origin)], check=True)
            for contents in ('first', 'locked content'):
                (origin / 'file').write_text(contents)
                subprocess.run(['git', '-C', str(origin), 'add', 'file'], check=True)
                subprocess.run(['git', '-C', str(origin), '-c', 'user.name=CI Fixture',
                    '-c', 'user.email=fixture@example.invalid', 'commit', '-qm', contents], check=True)
            commit = subprocess.check_output(['git', '-C', str(origin), 'rev-parse', 'HEAD'], text=True).strip()
            subprocess.run(['git', 'init', '--bare', '-q', str(target)], check=True)
            ref = (origin.as_uri(), '+' + commit + ':refs/commit/' + commit)
            args = ci_git.git_arguments(['fetch', '--force', '--update-head-ok', *ref], {ref})
            subprocess.run(['git', '-C', str(target), *args], check=True, capture_output=True)
            self.assertEqual(subprocess.check_output(['git', '-C', str(target),
                'show', commit + ':file'], text=True), 'locked content')
            self.assertEqual(subprocess.check_output(['git', '-C', str(target),
                'rev-list', '--count', commit], text=True).strip(), '1')
            self.assertTrue((target / 'shallow').exists())
            # uv's real local checkout mechanism also accepts this database.
            checkout = root / 'checkout'
            subprocess.run(['git', 'clone', '--local', str(target), str(checkout)],
                           check=True, capture_output=True)
            self.assertEqual(subprocess.check_output(['git', '-C', str(checkout),
                'show', commit + ':file'], text=True), 'locked content')


class CiProxyTests(unittest.TestCase):
    def test_direct_route_is_subset_of_existing_allowlist_and_not_github(self):
        root = Path(__file__).resolve().parents[2]
        text = (root / 'tools/ci_proxy.conf').read_text()
        directives = [line.split() for line in text.splitlines() if line and not line.startswith('#')]
        acls = {row[1]: set(row[3:]) for row in directives if row[:1] == ['acl'] and row[2] == 'dstdomain'}
        self.assertEqual(acls['direct_dependencies'], {'.pypi.org', '.pythonhosted.org', '.crates.io', '.rust-lang.org'})
        self.assertTrue(acls['direct_dependencies'] <= acls['ci_domains'])
        self.assertNotIn('.github.com', acls['direct_dependencies'])
        self.assertIn(['always_direct', 'allow', 'direct_dependencies'], directives)
        self.assertIn(['never_direct', 'deny', 'direct_dependencies'], directives)
        self.assertIn(['never_direct', 'allow', 'all'], directives)
        self.assertLess(directives.index(['http_access', 'deny', 'private_dst']),
                        directives.index(['http_access', 'allow', 'ci_domains']))
        self.assertIn(['http_access', 'deny', 'all'], directives)


if __name__ == '__main__':
    unittest.main()
