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

    def test_promotion_and_sync(self):
        self.assertIsNone(validate(event("test", "main")))
        self.assertIsNone(validate(event("main", "test")))

    def test_task_cannot_skip_integration(self):
        self.assertIsNotNone(validate(event(base="main")))

    def test_fork_cannot_impersonate_long_lived_branch(self):
        self.assertIsNotNone(validate(event("test", "main", head_repo=2)))
        self.assertIsNotNone(validate(event("main", "test", head_repo=2)))

    def test_same_source_and_target(self):
        for branch in ("main", "test"):
            self.assertIsNotNone(validate(event(branch, branch)))

    def test_no_other_target(self):
        self.assertIsNotNone(validate(event(base="develop")))

    def test_rejects_bad_names(self):
        for name in ("feature/2-x", "feat/0-x", "feat/02-x", "feat/2-", "feat/2-X",
                     "feat/2-x_y", "feat/2-x--y", "feat/2-x/other", "feat/2-x\n",
                     "alice/feat/2-x", "codex/2-x", "feat/2-" + "a" * 94):
            with self.subTest(name=name):
                self.assertIsNotNone(validate(event(name)))

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


if __name__ == "__main__":
    unittest.main()
