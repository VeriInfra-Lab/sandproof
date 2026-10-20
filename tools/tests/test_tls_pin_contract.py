"""Admission regression checks; these are not the Rust negative program proof."""
import copy
import json
from pathlib import Path
import shutil
import tempfile
import unittest

from tools import check_tls_pin_contract as tls

ROOT = Path(__file__).resolve().parents[2]


class TlsPinAdmission(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        files = [tls.ENVIRONMENT, tls.LLBC,
                 tls.DEST + "/translation.json", tls.DEST + "/SandproofTls/Types.lean",
                 tls.DEST + "/SandproofTls/Funs.lean", tls.DEST + "/SandproofTls/FunsExternal.lean"]
        for name in files:
            target = self.root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, target)
        self.llbc = json.loads((self.root / tls.LLBC).read_text())
        self.inventory = json.loads((self.root / tls.DEST / "translation.json").read_text())

    def admit(self):
        return tls.check_artifacts(self.root, self.llbc, self.inventory, frozen=False)

    def owned(self, name):
        identifier = next(item["def_id"] for item in self.inventory["functions"]
                          if item["rust_name"] == name)
        return next(item for item in self.llbc["translated"]["fun_decls"]
                    if item and item["def_id"] == identifier)

    def test_actual_frozen_candidate_surface_is_admitted(self):
        tls.check_artifacts(self.root)
        self.admit()

    def test_missing_authorization_root_is_rejected(self):
        self.inventory["functions"] = [item for item in self.inventory["functions"]
                                       if item["rust_name"] != tls.VERIFY]
        with self.assertRaisesRegex(ValueError, "missing or additional"):
            self.admit()

    def test_missing_leaf_return_body_is_rejected(self):
        self.owned(tls.LEAF)["body"] = "Opaque"
        with self.assertRaisesRegex(ValueError, "body missing"):
            self.admit()

    def test_owned_guard_cannot_be_an_external_assumption(self):
        next(item for item in self.inventory["functions"]
             if item["rust_name"] == tls.VERIFY)["is_opaque"] = True
        with self.assertRaisesRegex(ValueError, "opaque"):
            self.admit()

    def test_full_pin_width_is_independent_of_the_model_digest(self):
        path = self.root / tls.DEST / "SandproofTls/Types.lean"
        text = path.read_text()
        self.assertIn("Array Std.U8 32#usize", text)
        path.write_text(text.replace("Array Std.U8 32#usize", "Array Std.U8 16#usize"))
        with self.assertRaisesRegex(ValueError, "all 32 bytes"):
            self.admit()

    def test_missing_declaration_order_is_rejected(self):
        identifier = self.owned(tls.VERIFY)["def_id"]
        groups = self.llbc["translated"]["ordered_decls"]
        self.llbc["translated"]["ordered_decls"] = [
            group for group in groups if group.get("Fun") != {"NonRec": identifier}]
        with self.assertRaisesRegex(ValueError, "declaration order"):
            self.admit()

    def test_new_observable_drop_is_not_silently_ignored(self):
        self.owned(tls.VERIFY)["body"]["Structured"]["admission_test"] = {"Drop": {}}
        with self.assertRaisesRegex(ValueError, "Drop"):
            self.admit()

    def test_digest_cannot_acquire_a_default_implementation(self):
        trait = next(item for item in self.llbc["translated"]["trait_decls"]
                     if item and tls.identifiers(item["item_meta"]["name"]) ==
                     ["sandproof_internal", "tls", "CertificateDigest"])
        trait["methods"][0]["skip_binder"]["default"] = {}
        with self.assertRaisesRegex(ValueError, "explicit safe input"):
            self.admit()

    def test_wrong_digest_method_is_not_the_registered_interface(self):
        body = self.owned(tls.VERIFY)["body"]
        call = next(node["Call"] for node in tls.nodes(body) if "Call" in node
                    and "Trait" in node["Call"]["call"]["func"]["Regular"]["kind"])
        call["call"]["func"]["Regular"]["kind"]["Trait"][1] = 1
        with self.assertRaisesRegex(ValueError, "original dictionary"):
            self.admit()

    def test_target_or_disabled_checks_cannot_drift(self):
        original = copy.deepcopy(self.llbc)
        self.llbc["translated"]["target_information"][0]["key"] = "aarch64-unknown-linux-gnu"
        with self.assertRaisesRegex(ValueError, "target"):
            self.admit()
        self.llbc = original
        self.llbc["translated"]["options"]["no_typecheck"] = True
        with self.assertRaisesRegex(ValueError, "option|required compiler"):
            self.admit()

    def test_registered_model_or_generated_body_change_is_rejected(self):
        path = self.root / tls.DEST / "SandproofTls/FunsExternal.lean"
        path.write_text(path.read_text() + "\n-- changed input\n")
        with self.assertRaisesRegex(ValueError, "changed"):
            tls.check_artifacts(self.root)

    def test_theorem_audit_cannot_omit_the_input_binding(self):
        source = ROOT / "SandboxModel/Boundaries/TlsPin"
        target = self.root / "SandboxModel/Boundaries/TlsPin"
        shutil.copytree(source, target)
        tls.check_laws(self.root)
        path = target / "Audit.lean"
        original = path.read_text()
        line = "#audit_controller_proof " + tls.NAMESPACE + "linked_matches_original\n"
        self.assertEqual(original.count(line), 1)
        path.write_text(original.replace(line, ""))
        with self.assertRaisesRegex(ValueError, "audit omitted"):
            tls.check_laws(self.root)


if __name__ == "__main__":
    unittest.main()

