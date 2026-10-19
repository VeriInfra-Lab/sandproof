"""A stale parent checksum must stop the full entry before Cargo starts.

A sentinel Cargo detects accidental native work: these tests are not Rust or proof evidence. Actual
source admission uses shasum and the registered frozen inputs, without mocks.
"""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class ProofInputPreflightTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        manifests = sorted((ROOT / 'verification').glob('*inputs.sha256'))
        self.assertTrue(manifests)
        names = {str(p.relative_to(ROOT)) for p in manifests}
        for manifest in manifests:
            names.update(line.split('  ', 1)[1] for line in manifest.read_text().splitlines())
        for name in names:
            source = (ROOT / name).resolve()
            self.assertTrue(source.is_relative_to(ROOT.resolve()))
            destination = self.root / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, destination)
        # Source admission compares Git's tracked native inputs and rejects
        # untracked Rust. Give this isolated copy the same repository context;
        # never replace or skip the real admission check in the test.
        subprocess.run(['git', 'init', '--quiet', str(self.root)], check=True)
        subprocess.run(['git', '-C', str(self.root), 'add', '--', *sorted(names)],
                       check=True)
        binary = self.root / 'sentinel-bin'
        binary.mkdir()
        cargo = binary / 'cargo'
        cargo.write_text('#!/bin/sh\nprintf started > "$SANDPROOF_TEST_CARGO_MARKER"\nexit 125\n')
        cargo.chmod(0o700)
        self.marker = self.root / 'cargo-started'
        self.env = os.environ | {'PATH': str(binary) + os.pathsep + os.environ['PATH'],
                                 'SANDPROOF_TEST_CARGO_MARKER': str(self.marker)}

    def run_entry(self):
        self.env.pop('SANDPROOF_NATIVE_EVIDENCE', None)
        return subprocess.run(['bash', 'scripts/verify-controller.sh'], cwd=self.root,
                              env=self.env, text=True, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, check=False, timeout=30)

    def test_consistent_inputs_require_ning_evidence_without_starting_cargo(self):
        result = self.run_entry()
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('ning native acceptance evidence is required', result.stdout)
        self.assertFalse(self.marker.exists())

    def test_stale_parent_stops_before_any_cargo_invocation(self):
        path = self.root / 'verification/first-chain-inputs.sha256'
        rows = path.read_text().splitlines()
        hooks = [i for i, line in enumerate(rows)
                 if line.endswith('  verification/foundations-inputs.sha256')]
        self.assertEqual(len(hooks), 1)
        rows[hooks[0]] = '0' * 64 + '  verification/foundations-inputs.sha256'
        path.write_text('\n'.join(rows) + '\n')
        result = self.run_entry()
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn('verification/foundations-inputs.sha256: FAILED', result.stdout)
        self.assertFalse(self.marker.exists())

    def test_untracked_native_source_stops_before_any_cargo_invocation(self):
        path = self.root / 'sandbox-unreviewed/src/lib.rs'
        path.parent.mkdir(parents=True)
        path.write_text('pub fn unreviewed() {}\n')
        result = self.run_entry()
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn('untracked native source', result.stdout)
        self.assertFalse(self.marker.exists())
