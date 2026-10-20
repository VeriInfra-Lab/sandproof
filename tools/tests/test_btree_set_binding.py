"""Admission regression tests; synthetic receipts here are not Lean proof evidence."""
import copy
import importlib.util
import io
import tarfile
import tomllib
import json
from pathlib import Path
import tempfile
import unittest

from tools import check_btree_set_binding as binding
from tools import check_btree_set_binding_components as native


class BtreeSetAdmissionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.name = "SandboxModel/Fixture.lean"
        for name, text in [(self.name, "-- synthetic admission fixture\n"),
                           ("lean-toolchain", "leanprover/lean4:v4.31.0\n"),
                           ("lakefile.toml", 'name = "fixture"\n'),
                           ("lake-manifest.json", "{}\n")]:
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
        self.audit = "Fixture.checked"
        self.row = {
            "request": {"tool": "lean_diagnostic_messages", "arguments": {"file_path": self.name}},
            "stale": False,
            "revision": {"file_sha256": binding.sha(self.root / self.name),
                         "sources_sha256": binding.source_fingerprint(self.root),
                         "toolchain": "leanprover/lean4:v4.31.0",
                         "lakefile_sha256": binding.sha(self.root / "lakefile.toml"),
                         "manifest_sha256": binding.sha(self.root / "lake-manifest.json")},
            "result": {"result": {"partial": False, "success": True, "timed_out": False,
                                  "failed_dependencies": [], "still_elaborating_lines": None,
                                  "items": [{"severity": "info", "message":
                                             "Checked Fixture.checked: 2 standard logical dependencies"}]}}}

    def check_record(self, rows):
        path = self.root / "fixture.jsonl"
        path.write_text("".join(json.dumps(row) + "\n" for row in rows))
        return binding.service_records(path, self.root, [self.name], [self.audit])

    def test_valid_record_forwards_original_message(self):
        self.assertEqual(self.check_record([self.row]),
                         ["Checked Fixture.checked: 2 standard logical dependencies"])

    def test_actual_dependency_or_configuration_change_invalidates_receipt(self):
        for name in [self.name, "lakefile.toml", "lake-manifest.json", "lean-toolchain"]:
            with self.subTest(name=name):
                path = self.root / name
                before = path.read_bytes()
                path.write_bytes(before + b" changed")
                with self.assertRaises(ValueError):
                    self.check_record([self.row])
                path.write_bytes(before)
        path = self.root / "verification/aeneas/Changed.lean"
        path.parent.mkdir(parents=True)
        path.write_text("-- changed imported context\n")
        with self.assertRaises(ValueError):
            self.check_record([self.row])

    def test_incomplete_warning_and_duplicate_records_are_rejected(self):
        changes = [{"partial": True}, {"success": False}, {"timed_out": True},
                   {"failed_dependencies": ["bad"]}, {"still_elaborating_lines": [1]},
                   {"items": [{"severity": "warning", "message": "unfinished"}]},
                   {"items": []},
                   {"items": [{"severity": "info", "message":
                                "Checked Fixture.other: 2 standard logical dependencies"}]}]
        for fields in changes:
            with self.subTest(fields=fields):
                row = copy.deepcopy(self.row)
                row["result"]["result"].update(fields)
                with self.assertRaises(ValueError):
                    self.check_record([row])
        row = copy.deepcopy(self.row)
        row["stale"] = True
        with self.assertRaises(ValueError):
            self.check_record([row])
        with self.assertRaises(ValueError):
            self.check_record([self.row, self.row])
        with self.assertRaises(ValueError):
            self.check_record([])

    def test_mutation_changes_only_original_str_return_and_rejects_ambiguity(self):
        source = (binding.ROOT / binding.SOURCE).read_text()
        changed = binding.mutate(source, "invert-flag")
        self.assertEqual(changed.replace("    !seen.insert(value)", "    seen.insert(value)"), source)
        self.assertEqual(changed.count("    !seen.insert(value)"), 1)
        self.assertIn("pub fn insert_u64", changed)
        with self.assertRaises(ValueError):
            binding.mutate(source + source, "invert-flag")
        with self.assertRaises(ValueError):
            binding.mutate(source, "other")

    def artifacts(self):
        source = self.root / "source"
        target = self.root / "target"
        rows = []
        for name, kind, leaf in [("sandproof_btree_set_binding", "lib", "src/lib.rs"),
                                 ("insert_contract", "test", "tests/insert_contract.rs")]:
            rows.append({"reason": "compiler-artifact", "fresh": False,
                         "manifest_path": str(source / native.MANIFEST),
                         "target": {"name": name, "kind": [kind],
                                    "src_path": str(source / native.CRATE / leaf)},
                         "filenames": [str(target / name)],
                         "executable": str(target / name) if kind == "test" else None})
        return source, target, rows

    def check_artifacts(self, rows, source, target):
        log = self.root / "cargo.jsonl"
        log.write_text("".join(json.dumps(row) + "\n" for row in rows))
        return native.fresh_artifacts(log, target, source)

    def test_both_real_library_and_test_identity_required(self):
        source, target, rows = self.artifacts()
        self.assertEqual(set(self.check_artifacts(rows, source, target)),
                         {"sandproof_btree_set_binding", "insert_contract"})
        for index in range(2):
            for field, value in [("fresh", True), ("manifest_path", str(self.root / "other.toml")),
                                 ("filenames", [str(self.root / "elsewhere")])]:
                with self.subTest(index=index, field=field):
                    changed = copy.deepcopy(rows)
                    changed[index][field] = value
                    with self.assertRaises(ValueError):
                        self.check_artifacts(changed, source, target)
            changed = copy.deepcopy(rows)
            changed[index]["target"]["src_path"] = str(self.root / "other.rs")
            with self.assertRaises(ValueError):
                self.check_artifacts(changed, source, target)
        for changed in [rows[:1], rows[1:], rows + [rows[0]]]:
            with self.assertRaises(ValueError):
                self.check_artifacts(changed, source, target)


class BtreeSetReplayViewTests(unittest.TestCase):
    def test_private_views_accept_exact_registered_library_and_reject_duplicates(self):
        file = binding.ROOT / binding.EXPERIMENT / "replay.py"
        spec = importlib.util.spec_from_file_location("btree_set_component_replay", file)
        replay = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(replay)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / ".lake/packages").mkdir(parents=True)
            (root / "fixed-tools").mkdir()
            (root / "lakefile.toml").write_text('name = "fixture"\n')
            stream = io.BytesIO()
            with tarfile.open(fileobj=stream, mode="w"):
                pass
            archive = stream.getvalue()
            first = replay.prepare(root, archive, root / "first", root / "fixed-tools")
            config = tomllib.loads((first / "lakefile.toml").read_text())
            self.assertEqual(config["lean_lib"],
                             [{"name": "SandproofBtreeSetProbe", "srcDir": binding.GENERATED}])
            (root / "lakefile.toml").write_bytes((first / "lakefile.toml").read_bytes())
            second = replay.prepare(root, archive, root / "second", root / "fixed-tools")
            self.assertEqual((first / "lakefile.toml").read_bytes(),
                             (second / "lakefile.toml").read_bytes())
            self.assertEqual((second / ".tools").resolve(), (root / "fixed-tools").resolve())
            (root / "lakefile.toml").write_text(
                (first / "lakefile.toml").read_text() + replay.EXTENSION)
            with self.assertRaises(ValueError):
                replay.prepare(root, archive, root / "duplicate", root / "fixed-tools")
            (root / "lakefile.toml").write_text(
                (first / "lakefile.toml").read_text().replace(binding.GENERATED, "wrong/source"))
            with self.assertRaises(ValueError):
                replay.prepare(root, archive, root / "wrong", root / "fixed-tools")


if __name__ == "__main__":
    unittest.main()
