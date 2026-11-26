"""Check the Stage1 host's input admission without running a compiler."""
from contextlib import chdir
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from tools import stage1_host


class IndependentReferenceTests(unittest.TestCase):
    def test_resolves_sibling_helper_and_checks_original_inputs(self):
        with tempfile.TemporaryDirectory() as temporary, chdir(temporary):
            package = Path('verification/experiments/source-save-order/unload-parent-link')
            package.mkdir(parents=True)
            inputs = dict(source_commit='fixed-commit', source_tree='fixed-tree',
                          source_archive_sha256='archive', fixture_sha256='fixture',
                          entry_sha256='entry')
            (package / 'prepare_source_save_order.py').write_text(f'INPUTS = {inputs!r}\n')
            (package / 'verify.py').write_text(
                'from prepare_source_save_order import INPUTS\n\ndef candidate():\n    return INPUTS\n')
            subprocess.run(['git', 'init', '-q'], check=True)
            subprocess.run(['git', 'add', 'verification'], check=True)
            subprocess.run(['git', '-c', 'user.name=Test', '-c', 'user.email=test@example.test',
                            '-c', 'commit.gpgsign=false', 'commit', '-qm', 'proof fixture'], check=True)
            head, tree = subprocess.check_output(['git', 'rev-parse', 'HEAD', 'HEAD^{tree}'],
                                                 text=True).splitlines()
            result = Path('result.json').resolve()
            record = dict(inputs, kind='source-save-order-formal', complete=True,
                          views=['factored', 'reordered'], review_commit=head, review_tree=tree)
            with patch.dict(os.environ, {'SANDPROOF_STAGE1_SOURCE_SAVE_RESULT': str(result)}):
                result.write_text(json.dumps(record))
                self.assertEqual(stage1_host.independent_reference()['review_commit'], head)
                record['source_tree'] = 'unrelated-tree'
                result.write_text(json.dumps(record))
                with self.assertRaisesRegex(ValueError, 'uses different inputs'):
                    stage1_host.independent_reference()


class RustSourceAdmissionTests(unittest.TestCase):
    def test_two_pinned_toolchains_require_every_original_source_file(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            rustup_home = root / 'rustup'
            target = 'x86_64-unknown-linux-gnu'
            pin = root / stage1_host.SERDE_RUST_SOURCE_PIN
            pin.parent.mkdir(parents=True)
            original = Path(stage1_host.__file__).resolve().parents[1] / stage1_host.SERDE_RUST_SOURCE_PIN
            inputs = json.loads(original.read_text())['library_inputs']['standard_library']
            sources = {}
            for channel, files in inputs.items():
                sysroot = rustup_home / 'toolchains' / f'{channel}-{target}'
                compiler = sysroot / 'bin/rustc'
                compiler.parent.mkdir(parents=True)
                compiler.touch(mode=0o700)
                for relative in files:
                    path = sysroot / 'lib/rustlib/src/rust/library' / relative
                    path.parent.mkdir(parents=True, exist_ok=True)
                    material = f'fixture bytes for {channel}/{relative}'.encode()
                    path.write_bytes(material)
                    files[relative] = hashlib.sha256(material).hexdigest()
                    sources[(channel, relative)] = path
            pin.write_text(json.dumps({'library_inputs': {'standard_library': inputs}}))
            complete = stage1_host.pinned_rust_sources(root, rustup_home, target)
            self.assertEqual({channel: row['sources'] for channel, row in complete.items()}, inputs)
            needed = sources[('nightly-2026-02-07', 'std/src/io/mod.rs')]
            needed.unlink()
            with self.assertRaisesRegex(ValueError, 'missing locally: nightly-2026-02-07/std/src/io/mod.rs'):
                stage1_host.pinned_rust_sources(root, rustup_home, target)
            needed.write_text('replacement Rust source')
            with self.assertRaisesRegex(ValueError, 'differs from the serde binding: nightly-2026-02-07/std/src/io/mod.rs'):
                stage1_host.pinned_rust_sources(root, rustup_home, target)
