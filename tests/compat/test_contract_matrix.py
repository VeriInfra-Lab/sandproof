"""Validate the frozen capability inventory, not feature implementation."""

import json
import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
CONTRACT = ROOT / "tests/compat/contract.json"
SCENARIOS = (ROOT / "docs/testing-plan.md").read_text()


class CapabilityMatrix(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.manifest = json.loads(CONTRACT.read_text())

    def test_exact_owner_confirmed_inventory(self):
        capabilities = self.manifest["capabilities"]
        self.assertEqual([item["id"] for item in capabilities], list(range(1, 25)))
        self.assertEqual(len({item["surface"] for item in capabilities}), 24)

    def test_each_entry_has_separate_target_current_and_evidence(self):
        sources = self.manifest["upstream_sources"]
        levels = self.manifest["evidence_levels"]
        for item in self.manifest["capabilities"]:
            with self.subTest(id=item["id"]):
                for field in ("surface", "request", "response", "error", "difference"):
                    self.assertTrue(item[field].strip(), field)
                self.assertIn(item["source"], sources)
                self.assertIn(item["target"], {"support", "intentional_difference", "limited_support"})
                self.assertIn(item["current"], {"selected_tested", "partial", "rejected", "untested"})
                self.assertIn(item["evidence_level"], levels)
                self.assertTrue(item["issues"])
                self.assertTrue(all(number > 0 for number in item["issues"]))
                if item["current"] == "selected_tested":
                    self.assertNotEqual(item["evidence_level"], "contract_only")
                    self.assertTrue(item["tests"])

    def test_test_references_exist_without_treating_plans_as_passes(self):
        for item in self.manifest["capabilities"]:
            with self.subTest(id=item["id"]):
                for reference in item["tests"] + item.get("planned_tests", []):
                    path, separator, method = reference.partition("::")
                    source = ROOT / path
                    self.assertTrue(source.is_file(), reference)
                    if separator:
                        if source.suffix == ".rs":
                            self.assertRegex(method, r"^[a-z][a-z0-9_]+$")
                            self.assertIn(f"fn {method}(", source.read_text())
                        else:
                            self.assertRegex(method, r"^test_[a-z0-9_]+$")
                            self.assertIn(f"def {method}(", source.read_text())
                for scenario in item["planned_scenarios"]:
                    self.assertRegex(scenario, r"^S\d{2}$")
                    self.assertIn(f"| {scenario} |", SCENARIOS)

    def test_new_template_target_is_not_misreported_as_already_tested(self):
        self.assertTrue(
            any("template build/import" in gap for gap in self.manifest["not_yet_tested"])
        )
        template = self.manifest["capabilities"][16]
        self.assertEqual(template["id"], 17)
        self.assertEqual(template["target"], "limited_support")
        self.assertEqual(template["current"], "partial")
        self.assertIn(123, template["issues"])
        self.assertEqual(template["source"], "owner_templates")
        self.assertIn("未交付", template["difference"])


if __name__ == "__main__":
    unittest.main()
