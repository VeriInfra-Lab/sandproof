from copy import deepcopy
import unittest

from benchmarks.check_harbor import RUNTIMES, check_provenance
from tools.benchmark_tasks import InvalidInput


class ProvenanceTests(unittest.TestCase):
    def test_pypi_version_match_cannot_replace_pinned_git_source(self):
        for name, (version, url, commit, subdirectory) in RUNTIMES.items():
            valid = {"url": url, "vcs_info": {"vcs": "git", "commit_id": commit}}
            if subdirectory:
                valid["subdirectory"] = subdirectory
            self.assertEqual(check_provenance(name, version, valid)["commit"], commit)
            changes = [None, [], {}, {"vcs_info": []},
                       {"url": "https://pypi.org/simple", "archive_info": {}}]
            for key, replacement in (("url", url + "/other"), ("subdirectory", "other")):
                invalid = deepcopy(valid)
                invalid[key] = replacement
                changes.append(invalid)
            wrong_commit = deepcopy(valid)
            wrong_commit["vcs_info"]["commit_id"] = "0" * 40
            changes.append(wrong_commit)
            for invalid in changes:
                with self.subTest(name=name, metadata=invalid), self.assertRaises(InvalidInput):
                    check_provenance(name, version, invalid)
            with self.assertRaises(InvalidInput):
                check_provenance(name, "0.0.0", valid)


if __name__ == "__main__":
    unittest.main()
