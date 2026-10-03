import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from tools.check_pr_policy import validate


def event(head="feat/4-transaction-store", base="test", head_repo=1, base_repo=1):
    return {"pull_request": {
        "head": {"ref": head, "repo": {"id": head_repo}},
        "base": {"ref": base, "repo": {"id": base_repo}},
    }}


class PullRequestPolicyTests(unittest.TestCase):
    def test_task_types_and_codex_prefix(self):
        for kind in ("feat", "fix", "refactor", "test", "docs", "ci", "chore", "proof"):
            for prefix in ("", "codex/"):
                with self.subTest(kind=kind, prefix=prefix):
                    self.assertIsNone(validate(event(f"{prefix}{kind}/2-example-task")))

    def test_fork_task_may_target_test(self):
        self.assertIsNone(validate(event(head_repo=2)))

    def test_task_stacks_accept_all_types_and_codex_prefixes(self):
        branches = [
            f"{prefix}{kind}/2-example"
            for kind in ("feat", "fix", "refactor", "test", "docs", "ci", "chore", "proof")
            for prefix in ("", "codex/")
        ]
        for head in branches:
            for base in branches:
                with self.subTest(head=head, base=base):
                    self.assertIsNone(validate(event(head + "-child", base)))

    def test_task_stack_length_boundaries_apply_to_both_branches(self):
        for prefix in ("feat/2-", "codex/proof/14-"):
            for length in (99, 100, 101):
                branch = prefix + "a" * (length - len(prefix))
                for head, base in ((branch, "ci/2-workflow"), ("ci/2-workflow", branch)):
                    with self.subTest(head=head, base=base, length=length):
                        self.assertEqual(validate(event(head, base)) is None, length <= 100)

    def test_task_stacks_reject_forks_and_invalid_repository_identity(self):
        base = "codex/ci/2-workflow"
        self.assertIsNotNone(validate(event(base=base, head_repo=2)))
        for side in ("head", "base"):
            for identity in (None, True, False, "1", 1.0, 0, -1):
                with self.subTest(side=side, identity=identity):
                    value = event(base=base)
                    value["pull_request"][side]["repo"]["id"] = identity
                    self.assertIsNotNone(validate(value))
            for repo in (None, {}, []):
                with self.subTest(side=side, repo=repo):
                    value = event(base=base)
                    value["pull_request"][side]["repo"] = repo
                    self.assertIsNotNone(validate(value))
            value = event(base=base)
            del value["pull_request"][side]["repo"]
            self.assertIsNotNone(validate(value))
        # Matching, non-integer or non-positive IDs do not establish identity.
        for identity in (None, True, False, "1", 1.0, 0, -1):
            with self.subTest(identity=identity):
                self.assertIsNotNone(validate(event(base=base, head_repo=identity, base_repo=identity)))

    def test_stacks_preserve_all_existing_branch_directions(self):
        branches = ("main", "test", "dot-ning", "feat/4-transaction-store", "ci/2-workflow", "develop")
        same_repo_allowed = {
            ("test", "main"), ("main", "test"),
            ("feat/4-transaction-store", "test"), ("ci/2-workflow", "test"),
            ("feat/4-transaction-store", "dot-ning"), ("ci/2-workflow", "dot-ning"),
            ("feat/4-transaction-store", "ci/2-workflow"),
            ("ci/2-workflow", "feat/4-transaction-store"),
        }
        fork_allowed = {("feat/4-transaction-store", "test"), ("ci/2-workflow", "test")}
        for head_repo, allowed in ((1, same_repo_allowed), (2, fork_allowed)):
            for head in branches:
                for base in branches:
                    with self.subTest(head=head, base=base, head_repo=head_repo):
                        self.assertEqual(
                            validate(event(head, base, head_repo=head_repo)) is None,
                            (head, base) in allowed,
                        )

    def test_temporary_batch_accepts_same_repository_tasks(self):
        for kind in ("feat", "fix", "refactor", "test", "docs", "ci", "chore", "proof"):
            for prefix in ("", "codex/"):
                with self.subTest(kind=kind, prefix=prefix):
                    self.assertIsNone(validate(event(f"{prefix}{kind}/14-example-task", "dot-ning")))
        self.assertIsNone(validate(event("feat/14-" + "a" * 92, "dot-ning")))

    def test_temporary_batch_rejects_other_branch_directions(self):
        for head, base in (
            ("main", "dot-ning"), ("test", "dot-ning"), ("dot-ning", "dot-ning"),
            ("dot-ning", "main"), ("dot-ning", "test"), ("dot-ning", "develop"),
            ("feat/14-task", "dot-other"), ("feat/14-task", "dot-ning-next"),
        ):
            with self.subTest(head=head, base=base):
                self.assertIsNotNone(validate(event(head, base)))

    def test_temporary_batch_rejects_forks_and_invalid_repository_identity(self):
        self.assertIsNotNone(validate(event(base="dot-ning", head_repo=2)))
        for side in ("head", "base"):
            for identity in (None, True, "1", 0, -1):
                with self.subTest(side=side, identity=identity):
                    value = event(base="dot-ning")
                    value["pull_request"][side]["repo"]["id"] = identity
                    self.assertIsNotNone(validate(value))
            for repo in (None, {}, []):
                with self.subTest(side=side, repo=repo):
                    value = event(base="dot-ning")
                    value["pull_request"][side]["repo"] = repo
                    self.assertIsNotNone(validate(value))
            value = event(base="dot-ning")
            del value["pull_request"][side]["repo"]
            self.assertIsNotNone(validate(value))

    def test_promotion_and_sync(self):
        self.assertIsNone(validate(event("test", "main")))
        self.assertIsNone(validate(event("main", "test")))

    def test_task_cannot_skip_integration(self):
        self.assertIsNotNone(validate(event(base="main")))

    def test_fork_cannot_impersonate_long_lived_branch(self):
        self.assertIsNotNone(validate(event("test", "main", head_repo=2)))
        self.assertIsNotNone(validate(event("main", "test", head_repo=2)))

    def test_same_source_and_target(self):
        for branch in ("main", "test", "dot-ning", "feat/4-transaction-store", "codex/ci/2-workflow"):
            self.assertIsNotNone(validate(event(branch, branch)))

    def test_no_custom_target(self):
        for base in ("develop", "release", "formal", "alice", "dot-other", "dot-ning-next"):
            with self.subTest(base=base):
                self.assertIsNotNone(validate(event(base=base)))

    def test_rejects_bad_names(self):
        for name in ("feature/2-x", "feat/0-x", "feat/02-x", "feat/2-", "feat/2-X",
                     "feat/2-x_y", "feat/2-x--y", "feat/2-x/other", "feat/2-x\n",
                     "alice/feat/2-x", "codex/2-x", "feat/2-" + "a" * 94):
            for base in ("test", "dot-ning", "ci/2-workflow"):
                with self.subTest(name=name, base=base):
                    self.assertIsNotNone(validate(event(name, base)))
            with self.subTest(base=name):
                self.assertIsNotNone(validate(event(base=name)))

    def test_length_limit_is_inclusive(self):
        self.assertIsNone(validate(event("feat/2-" + "a" * 93)))

    def test_missing_or_invalid_event(self):
        for value in (None, [], {}, {"pull_request": None}, {"pull_request": {}},
                      event(head=None), event(base=None)):
            with self.subTest(value=value):
                self.assertIsNotNone(validate(value))

    def test_missing_or_invalid_repository_identity(self):
        for identity in (None, True, "1", 0):
            self.assertIsNotNone(validate(event("test", "main", head_repo=identity)))
        value = event("test", "main")
        value["pull_request"]["head"]["repo"] = None
        self.assertIsNotNone(validate(value))

    def test_cli_checks_actual_event_json_and_exit_status(self):
        script = Path(__file__).resolve().parents[1] / "check_pr_policy.py"
        for value, expected in (
            (event(base="codex/ci/2-workflow"), 0),
            (event(base="codex/ci/2-workflow", head_repo=2), 1),
            (event(base="develop"), 1),
            (event("main", "codex/ci/2-workflow"), 1),
        ):
            with self.subTest(value=value), tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / "event.json"
                path.write_text(json.dumps(value), encoding="utf-8")
                result = subprocess.run(
                    [sys.executable, str(script), "--event", str(path)],
                    check=False, capture_output=True, text=True,
                )
                self.assertEqual(result.returncode, expected, result.stderr)
                if expected == 0:
                    self.assertIn("policy passed", result.stdout)
                else:
                    self.assertIn("See CONTRIBUTING.md.", result.stderr)
                    self.assertNotIn("policy passed", result.stdout)


if __name__ == "__main__":
    unittest.main()
