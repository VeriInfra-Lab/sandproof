"""Evidence-bookkeeping regressions; synthetic MIR is never a program model."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from tools import v0_use_inventory as inventory


class MirIndexTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name) / "complete.mir"
        self.text = """// WARNING: This output format is intended for human consumers only
fn policy(_1: &mut OwnResource) -> () {
    let mut _0: ();
    let _2: Box<dyn Business>;
    bb0: {
        _0 = <dyn Business as Business>::run(move _2) -> [return: bb1, unwind: bb2];
    }
    bb1: {
        drop(_1) -> [return: bb3, unwind continue];
    }
    bb2 (cleanup): {
        drop(_1) -> [return: bb4, unwind terminate(cleanup)];
    }
    bb3: {
        return;
    }
    bb4 (cleanup): {
        resume;
    }
}
const policy::promoted[0]: &[u8; 1] = {
    let mut _0: &[u8; 1];
    bb0: {
        return;
    }
}
alloc1 (size: 1, align: 1) {
    78                                              | x
}
"""
        self.path.write_text(self.text)

    def test_complete_fn_constant_and_allocation_are_retained(self):
        indexed = inventory.index_mir(self.path)
        self.assertEqual(indexed["sha256"], inventory.sha(self.path))
        self.assertEqual([b["kind"] for b in indexed["bodies"]], ["fn", "const", "alloc"])
        self.assertEqual(len(indexed["bodies"][0]["locals"]), 2)
        terminators = indexed["bodies"][0]["terminators"]
        self.assertEqual(len(terminators), 3)
        self.assertIn("dyn Business", terminators[0]["text"])
        self.assertIn("unwind terminate(cleanup)", terminators[2]["text"])

    def test_unknown_terminator_is_retained_without_interpretation(self):
        self.path.write_text(self.text.replace("drop(_1)", "new_terminator(_1)"))
        indexed = inventory.index_mir(self.path)
        self.assertIn("new_terminator", indexed["bodies"][0]["terminators"][1]["text"])

    def test_literal_constant_and_empty_allocation_are_retained(self):
        self.path.write_text(self.text + 'const LIMIT: usize = const 32_usize;\n'
                             + 'alloc9 (size: 0, align: 1) {}\n')
        indexed = inventory.index_mir(self.path)
        self.assertEqual([b["kind"] for b in indexed["bodies"][-2:]], ["const", "alloc"])
        self.assertEqual(indexed["bodies"][-1]["line"], indexed["bodies"][-1]["end"])

    def test_function_address_allocation_is_retained_as_raw_data(self):
        self.path.write_text(self.text + 'alloc2183 (fn: digest::dynstate::sha256_block_data_order)\n')
        indexed = inventory.index_mir(self.path)
        self.assertEqual(indexed["bodies"][-1]["kind"], "alloc")
        self.assertIn("sha256_block_data_order", indexed["bodies"][-1]["header"])

    def test_unknown_top_level_format_is_rejected(self):
        self.path.write_text(self.text + "new_kind future_body {\n}\n")
        with self.assertRaisesRegex(ValueError, "unreviewed top-level"):
            inventory.index_mir(self.path)

    def test_truncated_body_is_rejected(self):
        self.path.write_text(self.text.split("const policy")[0].rstrip()[:-1])
        with self.assertRaisesRegex(ValueError, "missing complete"):
            inventory.index_mir(self.path)

    def test_nested_top_level_declaration_is_rejected(self):
        self.path.write_text(self.text.replace("    bb0: {", "fn hidden() -> () {", 1))
        with self.assertRaisesRegex(ValueError, "unterminated"):
            inventory.index_mir(self.path)

    def test_unknown_compiler_format_is_rejected(self):
        self.path.write_text(self.text.replace("// WARNING:", "// unknown:"))
        with self.assertRaisesRegex(ValueError, "unreviewed MIR format"):
            inventory.index_mir(self.path)

    def test_duplicate_json_keys_are_rejected(self):
        self.path.write_text('{"exit":0,"exit":1}')
        with self.assertRaisesRegex(ValueError, "duplicate key"):
            inventory.read_json(self.path)


class BuildCoverageTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.addCleanup(patch.stopall)
        patch.object(inventory, "ROOT", self.root).start()
        self.metadata = {"workspace_members": ["own"], "packages": [{"id": "own"}]}
        inventory.write_json(self.root / "metadata.json", self.metadata)
        self.artifact = {"reason": "compiler-artifact", "package_id": "own", "features": [],
                         "target": {"src_path": str(self.root / "own/src/lib.rs"), "name": "own"},
                         "profile": {"test": False, "opt_level": "0"}}
        self.log = self.root / "build.jsonl"
        self.log.write_text(json.dumps(self.artifact) + "\n")
        self.receipt = {"metadata_sha256": inventory.sha(self.root / "metadata.json"),
                        "stages": [{"log": self.log.name}],
                        "units": [{"source": "own/src/lib.rs", "cfg": [], "crate": "own", "codegen": []}]}

    def check(self):
        inventory.check_build_coverage(self.root, self.receipt)

    def test_actual_artifact_must_have_a_compiler_unit(self):
        self.check()
        self.receipt["units"] = []
        with self.assertRaisesRegex(ValueError, "no complete MIR"):
            self.check()

    def test_unused_unit_is_rejected(self):
        extra = copy.deepcopy(self.receipt["units"][0])
        extra["source"] = "other/src/lib.rs"
        self.receipt["units"].append(extra)
        with self.assertRaisesRegex(ValueError, "unexplained"):
            self.check()

    def test_missing_workspace_package_is_rejected(self):
        self.metadata["packages"].append({"id": "forgotten"})
        self.metadata["workspace_members"].append("forgotten")
        inventory.write_json(self.root / "metadata.json", self.metadata)
        self.receipt["metadata_sha256"] = inventory.sha(self.root / "metadata.json")
        with self.assertRaisesRegex(ValueError, "unexplained"):
            self.check()

    def test_wrong_feature_is_rejected(self):
        self.receipt["units"][0]["cfg"] = ['feature="durable"']
        with self.assertRaisesRegex(ValueError, "no complete MIR"):
            self.check()

    def test_wrong_optimization_profile_is_rejected(self):
        self.receipt["units"][0]["codegen"] = ["opt-level=3"]
        with self.assertRaisesRegex(ValueError, "no complete MIR"):
            self.check()

    def test_test_artifact_cannot_replace_production_artifact(self):
        self.artifact["profile"]["test"] = True
        self.log.write_text(json.dumps(self.artifact) + "\n")
        with self.assertRaisesRegex(ValueError, "test artifact"):
            self.check()

    def test_empty_build_output_is_not_success_evidence(self):
        self.log.write_text("Finished without a compiler receipt\n")
        with self.assertRaisesRegex(ValueError, "no own compiler artifacts"):
            self.check()

    def test_changed_metadata_is_rejected(self):
        self.metadata["workspace_members"] = []
        inventory.write_json(self.root / "metadata.json", self.metadata)
        with self.assertRaisesRegex(ValueError, "changed Cargo metadata"):
            self.check()


if __name__ == "__main__":
    unittest.main()
