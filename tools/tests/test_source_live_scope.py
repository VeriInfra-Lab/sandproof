"""Fixed source-live receipt integrity; synthetic packets are not execution evidence."""
import ast
from pathlib import Path
import unittest
from tools import acceptance_receipts as receipts
from tools import proof_batch
from tools.tests import test_pause_receipt_scope as fixture
import subprocess
import sys


class SourceLiveScopeTests(unittest.TestCase):
    setUp = fixture.PauseReceiptScopeTests.setUp
    key = fixture.PauseReceiptScopeTests.key
    packet = fixture.PauseReceiptScopeTests.packet
    # Retain only this range's invariants; the parent fixture constructs exact
    # original receipt digests and never executes Rust or Lean.
    def test_exact_two_cases(self):
        keys = [self.key(label, None, None) for label in ('functional', 'source-live')]
        receipts.check_native_components(keys, scope='stage1-source-live')
        for wrong in (keys[:1], keys[1:], keys+keys[:1], keys+[self.key('stage1-protocol',None,None)]):
            with self.subTest(wrong=wrong), self.assertRaises(ValueError):
                receipts.check_native_components(wrong, scope='stage1-source-live')

    def test_complete_scope_and_archive(self):
        self.packet(labels=('functional','source-live'), scope='stage1-source-live')
        self.assertEqual(receipts.validate_native(self.cache, scope='stage1-source-live')['scope'], 'stage1-source-live')
        receipts.native_case('source-live', receipts.ROOT, None, scope='stage1-source-live')
        for scope in ('all','stage1-protocol','stage1-create-delivery','stage1-protocol-convergence','pause-receipt'):
            with self.subTest(scope=scope), self.assertRaises(ValueError):
                receipts.validate_native(self.cache, scope=scope)
        entry = self.cache/'commands'/receipts.digest(self.key('source-live',None,None))
        (entry/'evidence/source.tar').unlink()
        with self.assertRaises(ValueError):
            receipts.validate_native(self.cache,scope='stage1-source-live')

    def test_real_component_and_original_full_registration(self):
        root=Path(__file__).resolve().parents[2]
        self.assertIn('source-live', proof_batch.MODULES)
        tree=ast.parse((root/'verification/experiments/stage1-protocol/registered.py').read_text())
        text=ast.unparse(tree)
        self.assertIn("'source-live', 'tools/check_source_live_components.py', False",text)
        shell=(root/'scripts/verify-source-live.sh').read_text()
        self.assertIn('require source-live --scope stage1-source-live',shell)
        self.assertIn('tools/check_source_live_components.py formal',shell)

    def test_original_native_verifier_regressions(self):
        root=Path(__file__).resolve().parents[2]
        subprocess.run([sys.executable, str(root/'verification/experiments/stage1-protocol/source-live/test_native.py'),
                        '--repo', str(root)], cwd=root, check=True)
