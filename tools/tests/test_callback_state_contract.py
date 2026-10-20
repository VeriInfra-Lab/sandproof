"""Checks for stale business-library admission and exact callback-body mutations."""
import copy
import json
from pathlib import Path
import tempfile
import unittest

from tools.check_callback_state_components import fresh_artifacts, mutate
from tools.check_callback_state_contract import ROOT, SOURCE


class CallbackArtifactAdmission(unittest.TestCase):
    def setUp(self):
        self.work = tempfile.TemporaryDirectory(prefix="callback-state-admission-")
        self.addCleanup(self.work.cleanup)
        self.root = Path(self.work.name)
        self.target = self.root / "target"
        self.target.mkdir()
        self.log = self.root / "cargo.log"
        self.rows = []
        for name, kind in (("sandbox_controller_core", "lib"),
                           ("sandproof_callback_state_extract", "lib"),
                           ("contract", "test")):
            package = (self.root / "controller-core" if name == "sandbox_controller_core"
                       else self.root / "verification/extract-callback-state")
            source = package / ("tests/contract.rs" if kind == "test" else "src/lib.rs")
            self.rows.append({"reason": "compiler-artifact",
                              "manifest_path": str(package / "Cargo.toml"),
                              "target": {"name": name, "kind": [kind], "src_path": str(source)},
                              "filenames": [str(self.target / name)], "fresh": False,
                              "executable": str(self.target / name) if kind == "test" else None})

    def admit(self, rows=None):
        self.log.write_text("\n".join(json.dumps(r) for r in (self.rows if rows is None else rows)))
        return fresh_artifacts(self.log, self.target, self.root)

    def test_requires_all_three_fresh_artifacts(self):
        self.assertEqual(set(self.admit()),
                         {"sandbox_controller_core", "sandproof_callback_state_extract", "contract"})

    def test_rejects_stale_original_business_library(self):
        self.rows[0]["fresh"] = True
        with self.assertRaisesRegex(ValueError, "fresh"):
            self.admit()

    def test_rejects_stale_callback_library(self):
        self.rows[1]["fresh"] = True
        with self.assertRaisesRegex(ValueError, "fresh"):
            self.admit()

    def test_rejects_stale_test_binary(self):
        self.rows[2]["fresh"] = True
        with self.assertRaisesRegex(ValueError, "fresh"):
            self.admit()

    def test_rejects_missing_business_library(self):
        with self.assertRaisesRegex(ValueError, "unambiguous"):
            self.admit(self.rows[1:])

    def test_rejects_duplicate_business_library_identity(self):
        with self.assertRaisesRegex(ValueError, "unambiguous"):
            self.admit(self.rows + [copy.deepcopy(self.rows[0])])

    def test_rejects_artifact_outside_private_target(self):
        self.rows[0]["filenames"] = [str(self.root / "other-library")]
        with self.assertRaisesRegex(ValueError, "escaped"):
            self.admit()

    def test_rejects_missing_test_executable(self):
        self.rows[2]["executable"] = None
        with self.assertRaisesRegex(ValueError, "executable"):
            self.admit()

    def test_rejects_empty_artifact_filenames(self):
        self.rows[0]["filenames"] = []
        with self.assertRaisesRegex(ValueError, "escaped"):
            self.admit()

    def test_rejects_fresh_library_from_another_manifest(self):
        self.rows[0]["manifest_path"] = str(self.root / "other/Cargo.toml")
        with self.assertRaisesRegex(ValueError, "identity"):
            self.admit()

    def test_rejects_fresh_library_with_another_source(self):
        self.rows[0]["target"]["src_path"] = str(self.root / "other/lib.rs")
        with self.assertRaisesRegex(ValueError, "identity"):
            self.admit()

    def test_no_write_mutation_only_removes_real_write(self):
        source = (ROOT / SOURCE).read_text()
        wrong = mutate(source, "no-write-toggle")
        self.assertEqual(wrong, source.replace("    *state = !*state;\n    *state", "    !*state"))
        self.assertIn("toggle_step(state, key, value)", wrong)
        self.assertEqual(mutate(source, "correct"), source)

    def test_double_record_mutation_keeps_full_original_arguments(self):
        source = (ROOT / SOURCE).read_text()
        wrong = mutate(source, "double-record")
        self.assertEqual(wrong.count("visited.push((*key, *value));"), 2)
        self.assertIn("*value > floor", wrong)
        self.assertIn("trace_step(visited, key, value, floor)", wrong)

    def test_rejects_ambiguous_mutation(self):
        source = (ROOT / SOURCE).read_text()
        for variant in ("no-write-toggle", "double-record"):
            with self.subTest(variant=variant), self.assertRaisesRegex(ValueError, "Ambiguous"):
                mutate(source + source, variant)


if __name__ == "__main__":
    unittest.main()
