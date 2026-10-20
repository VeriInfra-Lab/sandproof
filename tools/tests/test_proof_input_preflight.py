"""A stale parent checksum must stop the full entry before Cargo starts.

A sentinel Cargo detects accidental native work: these isolated fixtures are
not Rust or proof evidence. Their hashes are consistent by construction, while
actual repository admission retains its registered frozen inputs unchanged.
"""
import hashlib
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
        # Ordinary product changes intentionally leave prior proof inputs stale.
        # Construct only this TEMPORARY fixture's consistent checksum closure;
        # never refresh the repository's published proof/census evidence. The
        # actual shell admission still runs, including the parent-hash check.
        fixture_manifests={str(p.relative_to(ROOT)) for p in manifests}
        refreshed=set()
        def refresh(name, active):
            if name in refreshed:
                return
            self.assertNotIn(name,active,'cyclic fixture manifests')
            path=self.root/name
            records=[]
            for line in path.read_text().splitlines():
                _, child=line.split('  ',1)
                if child in fixture_manifests:
                    refresh(child,active|{name})
                checksum=hashlib.sha256((self.root/child).read_bytes()).hexdigest()
                records.append(checksum+'  '+child+'\n')
            path.write_text(''.join(records))
            refreshed.add(name)
        for name in sorted(fixture_manifests):
            refresh(name,set())
        # Admission compares tracked native inputs and rejects untracked Rust.
        # Give this fixture a real Git index; do not mock/skip source admission.
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

    def test_changed_native_source_stops_before_any_cargo_invocation(self):
        path=self.root/'sandbox-center-store/src/store.rs'
        path.write_text(path.read_text()+'\n// changed after frozen fixture input\n')
        result=self.run_entry()
        self.assertEqual(result.returncode,1,result.stdout)
        self.assertIn('sandbox-center-store/src/store.rs: FAILED',result.stdout)
        self.assertFalse(self.marker.exists())

    def test_untracked_native_source_stops_before_any_cargo_invocation(self):
        path = self.root / 'sandbox-unreviewed/src/lib.rs'
        path.parent.mkdir(parents=True)
        path.write_text('pub fn unreviewed() {}\n')
        result = self.run_entry()
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn('untracked native source', result.stdout)
        self.assertFalse(self.marker.exists())
